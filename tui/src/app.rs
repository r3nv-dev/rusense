//! TUI state and key handling — no terminal I/O, fully unit-testable.

use ratatui::crossterm::event::KeyCode;
use rusense_core::{Capabilities, FanDuty, FanMode, History, PowerSettings, ProfileSet, SensePort};

/// Telemetry samples kept for the sparkline (one per ~2s tick).
const HISTORY_CAPACITY: usize = 60;
/// Slider step for the custom fan duties, in percent.
const SLIDER_STEP: u8 = 10;

/// Where the error shown in the footer came from.
///
/// Tick errors are transient — the next fully successful refresh clears
/// them. Action errors carry actionable hints (e.g. ReadOnly's "rode o
/// install.sh") and must survive healthy ticks: only the next
/// successful user action clears them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorOrigin {
    /// Set by the periodic background refresh (`on_tick`).
    Tick,
    /// Set by a user-triggered write.
    Action,
}

/// All mutable UI state. Writes go exclusively through `port`.
pub struct App {
    port: Box<dyn SensePort + Send>,
    pub history: History,
    /// `None` until the first successful load.
    pub profiles: Option<ProfileSet>,
    pub fan: FanMode,
    pub power: Option<PowerSettings>,
    pub caps: Capabilities,
    /// Slider position for the custom CPU fan duty (0–100, step 10).
    pub custom_cpu: u8,
    /// Slider position for the custom GPU fan duty (0–100, step 10).
    pub custom_gpu: u8,
    /// Which slider ←/→ adjusts; toggled with Tab.
    pub focus_gpu: bool,
    /// Origin and display text of the last `SenseError`, shown in the
    /// status bar. See [`ErrorOrigin`] for the clearing rules.
    pub last_error: Option<(ErrorOrigin, String)>,
    /// True when running against the mock backend (`--mock`); the
    /// header shows "mock" instead of "driver ok".
    pub mock: bool,
}

impl App {
    /// Build the app and load the initial state from the port. `mock`
    /// is presentation-only (header badge).
    ///
    /// Load errors land in `last_error`; fields stay `None`/default.
    pub fn new(port: Box<dyn SensePort + Send>, mock: bool) -> Self {
        let caps = port.capabilities();
        let mut app = Self {
            port,
            history: History::new(HISTORY_CAPACITY),
            profiles: None,
            fan: FanMode::Auto,
            power: None,
            caps,
            custom_cpu: 50,
            custom_gpu: 50,
            focus_gpu: false,
            last_error: None,
            mock,
        };
        app.refresh_controls();
        app
    }

    /// Periodic refresh: push one telemetry sample and re-read the
    /// controls (the driver is the source of truth — another tool may
    /// have changed them). Errors land in `last_error`, old data stays;
    /// a fully successful refresh clears a stale tick error but never a
    /// standing action error.
    pub fn on_tick(&mut self) {
        let mut ok = true;
        match self.port.telemetry() {
            Ok(t) => self.history.push(t),
            Err(e) => {
                self.set_tick_error(e.to_string());
                ok = false;
            }
        }
        ok &= self.refresh_controls();
        if ok && !self.has_action_error() {
            self.last_error = None;
        }
    }

    fn has_action_error(&self) -> bool {
        matches!(self.last_error, Some((ErrorOrigin::Action, _)))
    }

    /// Record a background-refresh error — without clobbering a
    /// standing action error, whose hint is more actionable than a
    /// read hiccup.
    fn set_tick_error(&mut self, msg: String) {
        if !self.has_action_error() {
            self.last_error = Some((ErrorOrigin::Tick, msg));
        }
    }

    /// Record a user-action error; it persists until the next
    /// successful action.
    fn set_action_error(&mut self, msg: String) {
        self.last_error = Some((ErrorOrigin::Action, msg));
    }

    /// Handle one key press. `q`/`Esc` (quit) are the caller's job.
    pub fn on_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char(c @ '1'..='9') => {
                let index = (c as usize) - ('1' as usize);
                self.select_profile(index);
            }
            KeyCode::Char('a') if self.caps.fan_control => self.apply_fan(FanMode::Auto),
            KeyCode::Char('m') if self.caps.fan_control => self.apply_fan(FanMode::Max),
            KeyCode::Char('c') if self.caps.fan_control => self.enter_custom_fan(),
            KeyCode::Tab if self.caps.fan_control => self.focus_gpu = !self.focus_gpu,
            KeyCode::Left if self.caps.fan_control => self.adjust_slider(false),
            KeyCode::Right if self.caps.fan_control => self.adjust_slider(true),
            KeyCode::Char('b') if self.caps.power => {
                self.update_power(|s| s.limiter = !s.limiter);
            }
            KeyCode::Char('u') if self.caps.power => self.cycle_usb(),
            KeyCode::Char('k') if self.caps.power => {
                self.update_power(|s| s.backlight_timeout = !s.backlight_timeout);
            }
            _ => {}
        }
    }

    /// Re-read profiles and the capability-gated controls from the port
    /// (fan mode and power are skipped without the matching capability —
    /// they would fail on every tick forever on limited hardware).
    /// Read errors land in `last_error`; previously loaded data is kept.
    /// Returns whether every attempted read succeeded.
    fn refresh_controls(&mut self) -> bool {
        let mut ok = true;
        match self.port.profiles() {
            Ok(p) => self.profiles = Some(p),
            Err(e) => {
                self.set_tick_error(e.to_string());
                ok = false;
            }
        }
        if self.caps.fan_control {
            match self.port.fan_mode() {
                Ok(m) => {
                    self.fan = m;
                    // Keep the sliders in sync with an externally set custom mode.
                    if let FanMode::Custom { cpu, gpu } = m {
                        self.custom_cpu = cpu.get();
                        self.custom_gpu = gpu.get();
                    }
                }
                Err(e) => {
                    self.set_tick_error(e.to_string());
                    ok = false;
                }
            }
        }
        if self.caps.power {
            match self.port.power() {
                Ok(s) => self.power = Some(s),
                Err(e) => {
                    self.set_tick_error(e.to_string());
                    ok = false;
                }
            }
        }
        ok
    }

    /// Activate the `index`-th available profile (0-based). Out-of-range
    /// indices are a no-op: the hint row only shows existing profiles.
    fn select_profile(&mut self, index: usize) {
        let Some(profile) = self
            .profiles
            .as_ref()
            .and_then(|set| set.available.get(index))
            .cloned()
        else {
            return;
        };
        match self.port.set_profile(&profile) {
            Ok(()) => {
                if let Some(set) = self.profiles.as_mut() {
                    set.active = profile;
                }
                self.last_error = None;
            }
            Err(e) => self.set_action_error(e.to_string()),
        }
    }

    fn apply_fan(&mut self, mode: FanMode) {
        match self.port.set_fan_mode(mode) {
            Ok(()) => {
                self.fan = mode;
                self.last_error = None;
            }
            Err(e) => self.set_action_error(e.to_string()),
        }
    }

    /// Enter custom mode via the `c` key. (0,0) and (100,100) are
    /// aliased pairs — `FanMode::custom` normalizes them to Auto/Max —
    /// so if BOTH sliders sit on one, reset them to 50,50 first so `c`
    /// genuinely engages Custom (mirrors the GUI guard in
    /// `gui/frontend/app.js`, `#modes` click handler).
    fn enter_custom_fan(&mut self) {
        if (self.custom_cpu == 0 && self.custom_gpu == 0)
            || (self.custom_cpu == 100 && self.custom_gpu == 100)
        {
            self.custom_cpu = 50;
            self.custom_gpu = 50;
        }
        self.apply_custom_fan();
    }

    /// Apply the slider positions as a custom mode. `FanMode::custom`
    /// normalizes (0,0)→Auto and (100,100)→Max by design.
    fn apply_custom_fan(&mut self) {
        match (FanDuty::new(self.custom_cpu), FanDuty::new(self.custom_gpu)) {
            (Ok(cpu), Ok(gpu)) => self.apply_fan(FanMode::custom(cpu, gpu)),
            (Err(e), _) | (_, Err(e)) => self.set_action_error(e.to_string()),
        }
    }

    /// Move the focused slider one step and apply. Outside custom mode
    /// the first arrow press engages Custom at the current slider
    /// positions instead of adjusting — a shortcut equivalent to `c`,
    /// so the sliders are always live when the user reaches for them.
    fn adjust_slider(&mut self, up: bool) {
        if !matches!(self.fan, FanMode::Custom { .. }) {
            self.enter_custom_fan();
            return;
        }
        let slider = if self.focus_gpu {
            &mut self.custom_gpu
        } else {
            &mut self.custom_cpu
        };
        *slider = if up {
            slider.saturating_add(SLIDER_STEP).min(100)
        } else {
            slider.saturating_sub(SLIDER_STEP)
        };
        self.apply_custom_fan();
    }

    /// Apply `change` to a copy of the current power settings and write it.
    /// Per the port contract a failed write may have applied earlier
    /// fields, so the settings are re-read on error.
    fn update_power(&mut self, change: impl FnOnce(&mut PowerSettings)) {
        let Some(mut settings) = self.power.clone() else {
            return;
        };
        change(&mut settings);
        match self.port.set_power(settings.clone()) {
            Ok(()) => {
                self.power = Some(settings);
                self.last_error = None;
            }
            Err(e) => {
                self.set_action_error(e.to_string());
                if let Ok(current) = self.port.power() {
                    self.power = Some(current);
                }
            }
        }
    }

    /// Cycle the usb charge level 0 → 10 → 20 → 30 → 0.
    fn cycle_usb(&mut self) {
        self.update_power(|s| s.usb = s.usb.next());
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use rusense_core::{connect, MockSense, Profile, SenseError, Telemetry};

    use super::*;

    fn mock_app() -> App {
        App::new(connect(true).expect("mock connect nunca falha"), true)
    }

    fn custom(cpu: u8, gpu: u8) -> FanMode {
        FanMode::custom(FanDuty::new(cpu).unwrap(), FanDuty::new(gpu).unwrap())
    }

    // --- error-path fixture ---

    /// Per-operation failure switches, shared with the test through an
    /// `Arc<Mutex<_>>` because the [`App`] owns the port.
    #[derive(Default)]
    struct Failures {
        telemetry: bool,
        profiles: bool,
        fan_mode: bool,
        power: bool,
        set_profile: bool,
        set_fan_mode: bool,
        set_power: bool,
    }

    /// [`MockSense`] wrapper with switchable per-operation failures and
    /// overridable capabilities (pattern: `BareCaps` in ui.rs).
    struct FailingPort {
        inner: MockSense,
        caps: Capabilities,
        fail: Arc<Mutex<Failures>>,
    }

    fn sim_error(op: &str) -> SenseError {
        SenseError::Io(format!("falha simulada: {op}"))
    }

    impl FailingPort {
        fn fails(&self, pick: impl Fn(&Failures) -> bool) -> bool {
            pick(&self.fail.lock().unwrap())
        }
    }

    impl SensePort for FailingPort {
        fn capabilities(&self) -> Capabilities {
            self.caps
        }
        fn telemetry(&self) -> Result<Telemetry, SenseError> {
            if self.fails(|f| f.telemetry) {
                return Err(sim_error("telemetry"));
            }
            self.inner.telemetry()
        }
        fn profiles(&self) -> Result<ProfileSet, SenseError> {
            if self.fails(|f| f.profiles) {
                return Err(sim_error("profiles"));
            }
            self.inner.profiles()
        }
        fn set_profile(&mut self, p: &Profile) -> Result<(), SenseError> {
            if self.fails(|f| f.set_profile) {
                return Err(sim_error("set_profile"));
            }
            self.inner.set_profile(p)
        }
        fn fan_mode(&self) -> Result<FanMode, SenseError> {
            if self.fails(|f| f.fan_mode) {
                return Err(sim_error("fan_mode"));
            }
            self.inner.fan_mode()
        }
        fn set_fan_mode(&mut self, m: FanMode) -> Result<(), SenseError> {
            if self.fails(|f| f.set_fan_mode) {
                return Err(sim_error("set_fan_mode"));
            }
            self.inner.set_fan_mode(m)
        }
        fn power(&self) -> Result<PowerSettings, SenseError> {
            if self.fails(|f| f.power) {
                return Err(sim_error("power"));
            }
            self.inner.power()
        }
        fn set_power(&mut self, s: PowerSettings) -> Result<(), SenseError> {
            if self.fails(|f| f.set_power) {
                // Honor the port contract: earlier fields may already be
                // applied when the write fails — apply, then fail.
                self.inner.set_power(s)?;
                return Err(sim_error("set_power"));
            }
            self.inner.set_power(s)
        }
    }

    /// App over a [`FailingPort`] plus the shared failure switches.
    fn failing_app(caps: Capabilities) -> (App, Arc<Mutex<Failures>>) {
        let fail = Arc::new(Mutex::new(Failures::default()));
        let port = FailingPort {
            inner: MockSense::new(),
            caps,
            fail: Arc::clone(&fail),
        };
        (App::new(Box::new(port), false), fail)
    }

    fn full_caps() -> Capabilities {
        Capabilities {
            fan_control: true,
            power: true,
            four_zone_kb: false,
        }
    }

    // --- construction ---

    #[test]
    fn new_loads_initial_state_from_port() {
        let app = mock_app();
        assert!(app.caps.fan_control && app.caps.power);
        assert_eq!(
            app.profiles.as_ref().unwrap().active.as_str(),
            "balanced-performance"
        );
        assert_eq!(app.fan, FanMode::Auto);
        assert!(app.power.as_ref().unwrap().limiter);
        assert_eq!((app.custom_cpu, app.custom_gpu), (50, 50));
        assert!(!app.focus_gpu);
        assert!(app.last_error.is_none());
        assert!(app.history.is_empty());
    }

    // --- tick ---

    #[test]
    fn on_tick_pushes_telemetry_into_history() {
        let mut app = mock_app();
        app.on_tick();
        app.on_tick();
        assert_eq!(app.history.len(), 2);
        assert!(app.last_error.is_none());
    }

    #[test]
    fn on_tick_syncs_sliders_with_externally_set_custom_mode() {
        let mut app = mock_app();
        app.port.set_fan_mode(custom(30, 70)).unwrap();
        app.on_tick();
        assert_eq!(app.fan, custom(30, 70));
        assert_eq!((app.custom_cpu, app.custom_gpu), (30, 70));
    }

    // --- profiles ---

    #[test]
    fn digit_selects_nth_available_profile() {
        let mut app = mock_app();
        app.on_key(KeyCode::Char('2'));
        assert_eq!(app.port.profiles().unwrap().active.as_str(), "quiet");
        assert_eq!(app.profiles.as_ref().unwrap().active.as_str(), "quiet");
        assert!(app.last_error.is_none());
    }

    #[test]
    fn digit_beyond_available_profiles_is_a_noop() {
        let mut app = mock_app();
        app.on_key(KeyCode::Char('9'));
        assert_eq!(
            app.port.profiles().unwrap().active.as_str(),
            "balanced-performance"
        );
        assert!(app.last_error.is_none());
    }

    // --- fan modes ---

    #[test]
    fn a_m_c_apply_fan_modes_through_the_port() {
        let mut app = mock_app();
        app.on_key(KeyCode::Char('m'));
        assert_eq!(app.port.fan_mode().unwrap(), FanMode::Max);

        app.on_key(KeyCode::Char('c'));
        assert_eq!(app.port.fan_mode().unwrap(), custom(50, 50));

        app.on_key(KeyCode::Char('a'));
        assert_eq!(app.port.fan_mode().unwrap(), FanMode::Auto);
        assert!(app.last_error.is_none());
    }

    #[test]
    fn tab_toggles_slider_focus() {
        let mut app = mock_app();
        app.on_key(KeyCode::Tab);
        assert!(app.focus_gpu);
        app.on_key(KeyCode::Tab);
        assert!(!app.focus_gpu);
    }

    #[test]
    fn first_arrow_outside_custom_engages_custom_at_current_sliders() {
        let mut app = mock_app();
        // From Auto the first press engages Custom (like `c`), without
        // moving the slider yet.
        app.on_key(KeyCode::Right);
        assert_eq!(app.port.fan_mode().unwrap(), custom(50, 50));
        assert_eq!((app.custom_cpu, app.custom_gpu), (50, 50));
        // Now in Custom, the next press adjusts and applies.
        app.on_key(KeyCode::Right);
        assert_eq!(app.port.fan_mode().unwrap(), custom(60, 50));
    }

    #[test]
    fn arrows_apply_immediately_while_in_custom_mode() {
        let mut app = mock_app();
        app.on_key(KeyCode::Char('c')); // Custom 50,50.
        app.on_key(KeyCode::Right); // cpu 50 → 60.
        assert_eq!(app.port.fan_mode().unwrap(), custom(60, 50));
        app.on_key(KeyCode::Tab);
        app.on_key(KeyCode::Left); // gpu 50 → 40.
        assert_eq!(app.port.fan_mode().unwrap(), custom(60, 40));
    }

    #[test]
    fn c_from_aliased_slider_extremes_reenters_custom_at_50_50() {
        let mut app = mock_app();
        // Walk both sliders down to 0 inside custom mode; the final
        // apply of (0,0) normalizes to Auto (FanMode::custom contract).
        app.on_key(KeyCode::Char('c'));
        for _ in 0..5 {
            app.on_key(KeyCode::Left);
        }
        app.on_key(KeyCode::Tab);
        for _ in 0..5 {
            app.on_key(KeyCode::Left);
        }
        assert_eq!((app.custom_cpu, app.custom_gpu), (0, 0));
        assert_eq!(app.port.fan_mode().unwrap(), FanMode::Auto);

        // `c` must engage Custom for real, not send the Auto alias.
        app.on_key(KeyCode::Char('c'));
        assert_eq!(app.port.fan_mode().unwrap(), custom(50, 50));
        assert_eq!((app.custom_cpu, app.custom_gpu), (50, 50));
        assert!(app.last_error.is_none());
    }

    #[test]
    fn sliders_clamp_to_0_and_100() {
        let mut app = mock_app();
        for _ in 0..12 {
            app.on_key(KeyCode::Left);
        }
        assert_eq!(app.custom_cpu, 0);
        for _ in 0..12 {
            app.on_key(KeyCode::Right);
        }
        assert_eq!(app.custom_cpu, 100);
    }

    // --- power ---

    #[test]
    fn b_toggles_battery_limiter() {
        let mut app = mock_app();
        app.on_key(KeyCode::Char('b'));
        assert!(!app.port.power().unwrap().limiter);
        assert!(!app.power.as_ref().unwrap().limiter);
        app.on_key(KeyCode::Char('b'));
        assert!(app.port.power().unwrap().limiter);
        assert!(app.last_error.is_none());
    }

    #[test]
    fn u_cycles_usb_charge_level() {
        let mut app = mock_app();
        // The mock starts at 30; the cycle wraps to 0.
        for expected in [0u8, 10, 20, 30] {
            app.on_key(KeyCode::Char('u'));
            assert_eq!(app.port.power().unwrap().usb.get(), expected);
        }
        assert!(app.last_error.is_none());
    }

    #[test]
    fn k_toggles_backlight_timeout() {
        let mut app = mock_app();
        app.on_key(KeyCode::Char('k'));
        assert!(!app.port.power().unwrap().backlight_timeout);
        app.on_key(KeyCode::Char('k'));
        assert!(app.port.power().unwrap().backlight_timeout);
    }

    // --- error paths ---

    #[test]
    fn set_fan_mode_failure_sets_error_and_keeps_state() {
        let (mut app, fail) = failing_app(full_caps());
        fail.lock().unwrap().set_fan_mode = true;
        app.on_key(KeyCode::Char('m'));
        assert!(app.last_error.is_some());
        assert_eq!(app.fan, FanMode::Auto);
        assert_eq!(app.port.fan_mode().unwrap(), FanMode::Auto);
    }

    #[test]
    fn set_profile_failure_sets_error_and_keeps_active() {
        let (mut app, fail) = failing_app(full_caps());
        fail.lock().unwrap().set_profile = true;
        app.on_key(KeyCode::Char('2'));
        assert!(app.last_error.is_some());
        assert_eq!(
            app.profiles.as_ref().unwrap().active.as_str(),
            "balanced-performance"
        );
    }

    #[test]
    fn set_power_failure_rereads_power_from_port() {
        let (mut app, fail) = failing_app(full_caps());
        fail.lock().unwrap().set_power = true;
        app.on_key(KeyCode::Char('b'));
        assert!(app.last_error.is_some());
        // The wrapper applies before failing (partial-write contract):
        // only the re-read branch can observe the toggled limiter.
        assert!(!app.power.as_ref().unwrap().limiter);
    }

    #[test]
    fn telemetry_failure_keeps_history_and_next_success_clears_error() {
        let (mut app, fail) = failing_app(full_caps());
        app.on_tick();
        assert_eq!(app.history.len(), 1);

        fail.lock().unwrap().telemetry = true;
        app.on_tick();
        assert_eq!(app.history.len(), 1); // Old history kept.
        assert!(app.last_error.is_some());

        fail.lock().unwrap().telemetry = false;
        app.on_tick();
        assert_eq!(app.history.len(), 2);
        // A fully successful refresh clears the transient error.
        assert!(app.last_error.is_none());
    }

    #[test]
    fn profiles_read_failure_keeps_loaded_set() {
        let (mut app, fail) = failing_app(full_caps());
        fail.lock().unwrap().profiles = true;
        app.on_tick();
        assert!(app.last_error.is_some());
        assert_eq!(
            app.profiles.as_ref().unwrap().active.as_str(),
            "balanced-performance"
        );
    }

    #[test]
    fn gated_reads_are_skipped_without_capabilities() {
        let (mut app, fail) = failing_app(Capabilities {
            fan_control: false,
            power: false,
            four_zone_kb: false,
        });
        {
            let mut f = fail.lock().unwrap();
            f.fan_mode = true;
            f.power = true;
        }
        app.on_tick();
        app.on_tick();
        // The failing gated reads were never attempted: no error
        // surfaced, while the ungated profile read kept refreshing.
        assert!(app.last_error.is_none());
        assert!(app.profiles.is_some());
    }

    #[test]
    fn action_error_persists_across_successful_ticks() {
        let (mut app, fail) = failing_app(full_caps());
        fail.lock().unwrap().set_fan_mode = true;
        app.on_key(KeyCode::Char('m'));
        assert!(app.last_error.is_some());
        fail.lock().unwrap().set_fan_mode = false;
        app.on_tick();
        app.on_tick();
        // Healthy ticks must not erase an actionable write error
        // (e.g. ReadOnly's install.sh hint).
        assert!(matches!(app.last_error, Some((ErrorOrigin::Action, _))));
    }

    #[test]
    fn action_error_clears_on_next_successful_action() {
        let (mut app, fail) = failing_app(full_caps());
        fail.lock().unwrap().set_fan_mode = true;
        app.on_key(KeyCode::Char('m'));
        assert!(app.last_error.is_some());
        fail.lock().unwrap().set_fan_mode = false;
        app.on_key(KeyCode::Char('a'));
        assert!(app.last_error.is_none());
    }

    #[test]
    fn tick_error_does_not_overwrite_action_error() {
        let (mut app, fail) = failing_app(full_caps());
        {
            let mut f = fail.lock().unwrap();
            f.set_fan_mode = true;
            f.telemetry = true;
        }
        app.on_key(KeyCode::Char('m'));
        app.on_tick(); // Failing tick while an action error stands.
        let (origin, msg) = app.last_error.clone().expect("erro presente");
        assert_eq!(origin, ErrorOrigin::Action);
        assert!(msg.contains("set_fan_mode"), "mensagem errada: {msg}");
    }

    // --- unmapped keys ---

    #[test]
    fn unmapped_key_changes_nothing() {
        let mut app = mock_app();
        app.on_key(KeyCode::Char('x'));
        assert_eq!(app.port.fan_mode().unwrap(), FanMode::Auto);
        assert_eq!(
            app.port.profiles().unwrap().active.as_str(),
            "balanced-performance"
        );
        assert!(app.last_error.is_none());
    }
}
