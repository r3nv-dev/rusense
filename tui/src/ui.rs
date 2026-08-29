//! Pure rendering of [`App`] state — no mutation, no I/O.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Gauge, Paragraph, Sparkline, Wrap};
use ratatui::Frame;
use rusense_core::FanMode;

use crate::app::App;

/// Project accent (#e8362d), same as the GUI mockup.
const ACCENT: Color = Color::Rgb(232, 54, 45);
/// Full-scale RPM for the fan gauges.
const MAX_RPM: f64 = 6000.0;

/// Render the whole UI. Sections without the matching capability are
/// hidden entirely, mirroring the official AcerSense behavior.
pub fn render(frame: &mut Frame, app: &App) {
    // The footer is split off first so it survives small heights: only
    // what remains is disputed by the section constraints.
    let [main_area, footer_area] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(footer_height(app, frame.area().width)),
    ])
    .areas(frame.area());

    let mut constraints = vec![Constraint::Length(7), Constraint::Length(3)];
    if app.caps.fan_control {
        constraints.push(Constraint::Length(4));
    }
    if app.caps.power {
        constraints.push(Constraint::Length(4));
    }
    constraints.push(Constraint::Min(0));

    let areas = Layout::vertical(constraints).split(main_area);
    // One area per constraint pushed above, in the same order; the
    // last one is the flexible spacer.
    let mut areas = areas.iter().copied();
    let mut next = || areas.next().expect("uma área por constraint");

    render_monitor(frame, app, next());
    render_profiles(frame, app, next());
    if app.caps.fan_control {
        render_fans(frame, app, next());
    }
    if app.caps.power {
        render_power(frame, app, next());
    }
    render_footer(frame, app, footer_area);
}

/// Footer height: one hint line, or enough lines (capped at 3) to wrap
/// the current error message at `width` columns. One line of slack on
/// top of the char-count estimate, because `Wrap` breaks at word
/// boundaries and can need more lines than `chars / width` predicts.
fn footer_height(app: &App, width: u16) -> u16 {
    match &app.last_error {
        Some((_, msg)) if width > 0 => {
            let lines = msg.chars().count().div_ceil(usize::from(width));
            u16::try_from((lines + 1).clamp(2, 3)).expect("2..=3 cabe em u16")
        }
        _ => 1,
    }
}

/// Header block with fan gauges, current temps and the CPU sparkline.
fn render_monitor(frame: &mut Frame, app: &App, area: Rect) {
    let badge = if app.mock { " mock " } else { " driver ok " };
    let block = Block::bordered()
        .title(Line::from(" RUSENSE ").fg(ACCENT).bold())
        .title_top(Line::from(badge).right_aligned().dim());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [cpu_row, gpu_row, temps_row, spark_row] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);

    let latest = app.history.latest();
    let (cpu_rpm, gpu_rpm) = latest.map_or((0, 0), |t| (t.fan_cpu_rpm, t.fan_gpu_rpm));
    render_fan_gauge(frame, cpu_row, "FAN CPU", cpu_rpm);
    render_fan_gauge(frame, gpu_row, "FAN GPU", gpu_rpm);

    let temps = latest.map_or_else(
        || "aguardando leitura...".to_string(),
        |t| {
            format!(
                "{:.0}°C cpu · {:.0}°C gpu · {:.0}°C sys",
                t.temps[0], t.temps[1], t.temps[2]
            )
        },
    );
    frame.render_widget(Paragraph::new(temps), temps_row);

    let series = app.history.temps_series(0);
    let spark = Sparkline::default()
        .data(series.iter().map(|&t| t.round() as u64))
        .max(100)
        .style(Style::new().fg(ACCENT));
    frame.render_widget(spark, spark_row);
}

/// One labeled gauge: `ratio = rpm / 6000` clamped, label in RPM.
fn render_fan_gauge(frame: &mut Frame, area: Rect, name: &str, rpm: u32) {
    let [label, gauge] =
        Layout::horizontal([Constraint::Length(9), Constraint::Min(0)]).areas(area);
    frame.render_widget(Paragraph::new(name), label);
    frame.render_widget(
        Gauge::default()
            .ratio((f64::from(rpm) / MAX_RPM).clamp(0.0, 1.0))
            .label(format!("{rpm} rpm"))
            .gauge_style(Style::new().fg(ACCENT)),
        gauge,
    );
}

/// Available profiles inline, active one in accent bold, `[n]` hints.
fn render_profiles(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans: Vec<Span> = Vec::new();
    match &app.profiles {
        Some(set) => {
            for (i, profile) in set.available.iter().enumerate() {
                spans.push(Span::raw(format!("[{}] ", i + 1)).dim());
                let name = Span::raw(profile.as_str());
                spans.push(if *profile == set.active {
                    name.fg(ACCENT).bold()
                } else {
                    name
                });
                spans.push(Span::raw("   "));
            }
        }
        None => spans.push(Span::raw("perfis indisponíveis").dim()),
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).block(Block::bordered().title(" PERFIL ")),
        area,
    );
}

/// Fan mode selector and the two custom-duty sliders.
fn render_fans(frame: &mut Frame, app: &App, area: Rect) {
    let highlight = |active: bool, text: &str| {
        if active {
            Span::raw(text.to_uppercase()).fg(ACCENT).bold()
        } else {
            Span::raw(text.to_string())
        }
    };
    let modes = Line::from(vec![
        Span::raw("modo: "),
        highlight(app.fan == FanMode::Auto, "auto"),
        Span::raw("  "),
        highlight(app.fan == FanMode::Max, "max"),
        Span::raw("  "),
        highlight(matches!(app.fan, FanMode::Custom { .. }), "custom"),
    ]);

    let focus = |focused: bool, name: &str| {
        if focused {
            Span::raw(name.to_string()).fg(ACCENT).bold()
        } else {
            Span::raw(name.to_string())
        }
    };
    let sliders = Line::from(vec![
        focus(!app.focus_gpu, "cpu"),
        Span::raw(format!(
            " {} {:>3}%    ",
            slider_bar(app.custom_cpu),
            app.custom_cpu
        )),
        focus(app.focus_gpu, "gpu"),
        Span::raw(format!(
            " {} {:>3}%",
            slider_bar(app.custom_gpu),
            app.custom_gpu
        )),
    ]);

    frame.render_widget(
        Paragraph::new(vec![modes, sliders]).block(Block::bordered().title(" FANS ")),
        area,
    );
}

/// Text slider: 11 slots (step 10), knob at the current position.
fn slider_bar(value: u8) -> String {
    let pos = usize::from(value / 10);
    (0..11).map(|i| if i == pos { '○' } else { '─' }).collect()
}

/// Power toggles plus battery state from the latest telemetry sample.
fn render_power(frame: &mut Frame, app: &App, area: Rect) {
    let on_off = |v: bool| {
        if v {
            Span::raw("ON").fg(ACCENT).bold()
        } else {
            Span::raw("OFF").dim()
        }
    };
    let toggles = match &app.power {
        Some(p) => Line::from(vec![
            Span::raw("[b] limite 80%: ").dim(),
            on_off(p.limiter),
            Span::raw("   [u] usb: ").dim(),
            Span::raw(format!("{}%", p.usb.get())),
            Span::raw("   [k] backlight: ").dim(),
            on_off(p.backlight_timeout),
        ]),
        None => Line::from(Span::raw("energia indisponível").dim()),
    };
    let battery = app.history.latest().map_or_else(
        || Line::from(Span::raw("bateria: —").dim()),
        |t| {
            Line::from(format!(
                "bateria: {}% · {}",
                t.battery_pct,
                t.battery_status.to_lowercase()
            ))
        },
    );
    frame.render_widget(
        Paragraph::new(vec![toggles, battery]).block(Block::bordered().title(" ENERGIA ")),
        area,
    );
}

/// Keybinding summary, replaced by the last error (verbatim, red).
fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let line = match &app.last_error {
        Some((_, msg)) => Line::from(Span::raw(msg.as_str()).fg(Color::Red).bold()),
        None => {
            let mut hints: Vec<String> = Vec::new();
            if let Some(set) = &app.profiles {
                hints.push(match set.available.len() {
                    1 => "1 perfil".to_string(),
                    n => format!("1-{n} perfil"),
                });
            }
            if app.caps.fan_control {
                hints.push("a/m/c fans".to_string());
                hints.push("tab/←→ ajusta".to_string());
            }
            if app.caps.power {
                hints.push("b/u/k energia".to_string());
            }
            hints.push("q sai".to_string());
            Line::from(Span::raw(hints.join(" · ")).dim())
        }
    };
    frame.render_widget(Paragraph::new(line).wrap(Wrap { trim: true }), area);
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use rusense_core::{
        connect, Capabilities, FanMode, MockSense, PowerSettings, Profile, ProfileSet, SenseError,
        SensePort, Telemetry,
    };

    use super::*;
    use crate::app::{App, ErrorOrigin};

    /// Render `app` at the given size and return the buffer as plain text.
    fn render_sized(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    /// Render `app` at 80x24 and return the buffer as plain text.
    fn render_to_text(app: &App) -> String {
        render_sized(app, 80, 24)
    }

    #[test]
    fn renders_mock_app_without_panicking() {
        let mut app = App::new(connect(true).unwrap(), true);
        app.on_tick();
        let text = render_to_text(&app);
        assert!(text.contains("RUSENSE"));
        assert!(text.contains("balanced-performance"));
        assert!(text.contains("rpm"));
        assert!(text.contains("PERFIL"));
        assert!(text.contains("FANS"));
        assert!(text.contains("ENERGIA"));
        assert!(text.contains("41°C cpu"));
        assert!(text.contains("bateria: 80%"));
    }

    #[test]
    fn renders_before_first_tick_without_telemetry() {
        let app = App::new(connect(true).unwrap(), true);
        let text = render_to_text(&app);
        assert!(text.contains("aguardando leitura"));
        assert!(text.contains("0 rpm"));
    }

    #[test]
    fn error_replaces_footer_hints() {
        let mut app = App::new(connect(true).unwrap(), true);
        app.last_error = Some((
            ErrorOrigin::Action,
            "sem permissão de escrita — rode o install.sh (regra udev)".into(),
        ));
        let text = render_to_text(&app);
        assert!(text.contains("sem permissão de escrita"));
        assert!(!text.contains("q sai"));
    }

    /// Mock wrapper reporting no optional capabilities.
    struct BareCaps(MockSense);

    impl SensePort for BareCaps {
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                fan_control: false,
                power: false,
                four_zone_kb: false,
            }
        }
        fn telemetry(&self) -> Result<Telemetry, SenseError> {
            self.0.telemetry()
        }
        fn profiles(&self) -> Result<ProfileSet, SenseError> {
            self.0.profiles()
        }
        fn set_profile(&mut self, p: &Profile) -> Result<(), SenseError> {
            self.0.set_profile(p)
        }
        fn fan_mode(&self) -> Result<FanMode, SenseError> {
            self.0.fan_mode()
        }
        fn set_fan_mode(&mut self, m: FanMode) -> Result<(), SenseError> {
            self.0.set_fan_mode(m)
        }
        fn power(&self) -> Result<PowerSettings, SenseError> {
            self.0.power()
        }
        fn set_power(&mut self, s: PowerSettings) -> Result<(), SenseError> {
            self.0.set_power(s)
        }
    }

    #[test]
    fn header_badge_reflects_backend_kind() {
        let mock_app = App::new(connect(true).unwrap(), true);
        let text = render_to_text(&mock_app);
        assert!(text.contains(" mock "));
        assert!(!text.contains("driver ok"));

        let driver_app = App::new(connect(true).unwrap(), false);
        let text = render_to_text(&driver_app);
        assert!(text.contains("driver ok"));
    }

    #[test]
    fn footer_survives_small_heights() {
        let mut app = App::new(connect(true).unwrap(), true);
        app.on_tick();
        let text = render_sized(&app, 80, 10);
        assert!(text.contains("RUSENSE"));
        assert!(text.contains("q sai"));
        // Even at 2 rows the status line keeps its slot.
        assert!(render_sized(&app, 80, 2).contains("q sai"));
    }

    #[test]
    fn long_error_wraps_in_footer() {
        let mut app = App::new(connect(true).unwrap(), true);
        app.last_error = Some((
            ErrorOrigin::Action,
            "sem permissão de escrita — rode o install.sh (regra udev)".into(),
        ));
        let text = render_sized(&app, 46, 24);
        // Wider than 46 columns: without wrapping the tail is truncated.
        assert!(text.contains("sem permissão"));
        assert!(text.contains("udev"));
    }

    #[test]
    fn footer_wrap_gets_slack_for_word_breaks() {
        let mut app = App::new(connect(true).unwrap(), true);
        // 39 chars → 2 lines by char count, but word wrap at 20
        // columns needs 3; the slack line keeps the tail visible.
        app.last_error = Some((
            ErrorOrigin::Action,
            "desconfiguração inesperada reencontrada".into(),
        ));
        let text = render_sized(&app, 20, 24);
        assert!(text.contains("reencontrada"));
    }

    /// Mock wrapper reporting a single platform profile.
    struct SingleProfile(MockSense);

    impl SensePort for SingleProfile {
        fn capabilities(&self) -> Capabilities {
            self.0.capabilities()
        }
        fn telemetry(&self) -> Result<Telemetry, SenseError> {
            self.0.telemetry()
        }
        fn profiles(&self) -> Result<ProfileSet, SenseError> {
            ProfileSet::parse("balanced", "balanced")
        }
        fn set_profile(&mut self, p: &Profile) -> Result<(), SenseError> {
            self.0.set_profile(p)
        }
        fn fan_mode(&self) -> Result<FanMode, SenseError> {
            self.0.fan_mode()
        }
        fn set_fan_mode(&mut self, m: FanMode) -> Result<(), SenseError> {
            self.0.set_fan_mode(m)
        }
        fn power(&self) -> Result<PowerSettings, SenseError> {
            self.0.power()
        }
        fn set_power(&mut self, s: PowerSettings) -> Result<(), SenseError> {
            self.0.set_power(s)
        }
    }

    #[test]
    fn single_profile_hint_drops_the_range() {
        let app = App::new(Box::new(SingleProfile(MockSense::new())), true);
        let text = render_to_text(&app);
        assert!(text.contains("1 perfil"));
        assert!(!text.contains("1-1"));
    }

    #[test]
    fn sections_without_capability_are_hidden() {
        let mut app = App::new(Box::new(BareCaps(MockSense::new())), true);
        app.on_tick();
        let text = render_to_text(&app);
        assert!(text.contains("RUSENSE"));
        assert!(text.contains("PERFIL"));
        assert!(!text.contains("FANS"));
        assert!(!text.contains("ENERGIA"));
        assert!(!text.contains("a/m/c"));
        assert!(!text.contains("b/u/k"));
    }
}
