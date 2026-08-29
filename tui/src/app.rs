//! TUI state and key handling — no terminal I/O, fully unit-testable.

use ratatui::crossterm::event::KeyCode;
use rusense_core::{
    Capabilities, FanDuty, FanMode, History, PowerSettings, ProfileSet, SensePort, UsbChargeLevel,
};

/// Telemetry samples kept for the sparkline (one per ~2s tick).
const HISTORY_CAPACITY: usize = 60;
/// Slider step for the custom fan duties, in percent.
const SLIDER_STEP: u8 = 10;

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
    /// Display text of the last `SenseError`, shown in the status bar.
    pub last_error: Option<String>,
}

impl App {
    /// Build the app and load the initial state from the port.
    ///
    /// Load errors land in `last_error`; fields stay `None`/default.
    pub fn new(port: Box<dyn SensePort + Send>) -> Self {
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
        };
        app.refresh_controls();
        app
    }

    /// Periodic refresh: push one telemetry sample and re-read the
    /// controls (the driver is the source of truth — another tool may
    /// have changed them). Errors land in `last_error`, old data stays.
    pub fn on_tick(&mut self) {
        match self.port.telemetry() {
            Ok(t) => self.history.push(t),
            Err(e) => self.last_error = Some(e.to_string()),
        }
        self.refresh_controls();
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
            KeyCode::Char('c') if self.caps.fan_control => self.apply_custom_fan(),
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

    /// Re-read profiles, fan mode and power from the port. Read errors
    /// land in `last_error`; previously loaded data is kept.
    fn refresh_controls(&mut self) {
        match self.port.profiles() {
            Ok(p) => self.profiles = Some(p),
            Err(e) => self.last_error = Some(e.to_string()),
        }
        match self.port.fan_mode() {
            Ok(m) => {
                self.fan = m;
                // Keep the sliders in sync with an externally set custom mode.
                if let FanMode::Custom { cpu, gpu } = m {
                    self.custom_cpu = cpu.get();
                    self.custom_gpu = gpu.get();
                }
            }
            Err(e) => self.last_error = Some(e.to_string()),
        }
        match self.port.power() {
            Ok(s) => self.power = Some(s),
            Err(e) => self.last_error = Some(e.to_string()),
        }
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
            Err(e) => self.last_error = Some(e.to_string()),
        }
    }

    fn apply_fan(&mut self, mode: FanMode) {
        match self.port.set_fan_mode(mode) {
            Ok(()) => {
                self.fan = mode;
                self.last_error = None;
            }
            Err(e) => self.last_error = Some(e.to_string()),
        }
    }

    /// Apply the slider positions as a custom mode. `FanMode::custom`
    /// normalizes (0,0)→Auto and (100,100)→Max by design.
    fn apply_custom_fan(&mut self) {
        match (FanDuty::new(self.custom_cpu), FanDuty::new(self.custom_gpu)) {
            (Ok(cpu), Ok(gpu)) => self.apply_fan(FanMode::custom(cpu, gpu)),
            (Err(e), _) | (_, Err(e)) => self.last_error = Some(e.to_string()),
        }
    }

    /// Move the focused slider one step; while in custom mode the new
    /// duties are applied immediately.
    fn adjust_slider(&mut self, up: bool) {
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
        if matches!(self.fan, FanMode::Custom { .. }) {
            self.apply_custom_fan();
        }
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
                self.last_error = Some(e.to_string());
                if let Ok(current) = self.port.power() {
                    self.power = Some(current);
                }
            }
        }
    }

    /// Cycle the usb charge level 0 → 10 → 20 → 30 → 0.
    fn cycle_usb(&mut self) {
        let Some(current) = self.power.as_ref().map(|s| s.usb.get()) else {
            return;
        };
        match UsbChargeLevel::new((current + 10) % 40) {
            Ok(next) => self.update_power(|s| s.usb = next),
            Err(e) => self.last_error = Some(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use rusense_core::connect;

    use super::*;

    fn mock_app() -> App {
        App::new(connect(true).expect("mock connect nunca falha"))
    }

    fn custom(cpu: u8, gpu: u8) -> FanMode {
        FanMode::custom(FanDuty::new(cpu).unwrap(), FanDuty::new(gpu).unwrap())
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
    fn arrows_adjust_focused_slider_without_applying_outside_custom() {
        let mut app = mock_app();
        app.on_key(KeyCode::Right);
        assert_eq!(app.custom_cpu, 60);
        app.on_key(KeyCode::Tab);
        app.on_key(KeyCode::Left);
        assert_eq!(app.custom_gpu, 40);
        // Mode stayed Auto: nothing was applied.
        assert_eq!(app.port.fan_mode().unwrap(), FanMode::Auto);
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
