//! rusense — TUI NitroSense clone over the Linuwu-Sense driver.
//!
//! `rusense` talks to the real driver; `rusense --mock` runs anywhere;
//! `rusense --once` prints one telemetry sample as JSON (waybar/scripts).

mod app;
mod once;
mod ui;

use std::env;
use std::io;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyCode};
use ratatui::DefaultTerminal;
use rusense_core::connect;

use crate::app::App;

/// Telemetry refresh period.
const TICK: Duration = Duration::from_secs(2);

const USAGE: &str = "\
uso: rusense [--mock] [--once]

  --mock      backend simulado (roda em qualquer máquina, sem driver)
  --once      imprime uma leitura de telemetria em JSON e sai
  -h, --help  mostra esta ajuda";

fn main() -> ExitCode {
    let mut mock = false;
    let mut once = false;
    for arg in env::args().skip(1) {
        match arg.as_str() {
            "--mock" => mock = true,
            "--once" => once = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("argumento desconhecido: {other}");
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }

    if once {
        return run_once(mock);
    }

    // Fail before touching the terminal so the PT-BR message (e.g. the
    // DriverMissing install hint) lands readable on stderr.
    let port = match connect(mock) {
        Ok(port) => port,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let mut app = App::new(port, mock);

    // init() also installs a panic hook that restores the terminal.
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app);
    ratatui::restore();

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// `--once`: no terminal UI — read one sample, print JSON, exit.
fn run_once(mock: bool) -> ExitCode {
    match connect(mock).and_then(|port| port.telemetry()) {
        Ok(t) => {
            println!("{}", once::telemetry_json(&t));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// Event loop: draw, poll input with the remaining tick budget as the
/// timeout, refresh telemetry every [`TICK`]. `q`/`Esc` quit.
fn run(terminal: &mut DefaultTerminal, app: &mut App) -> io::Result<()> {
    app.on_tick(); // First sample right away, not 2s in.
    let mut last_tick = Instant::now();
    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        let timeout = TICK.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.is_press() {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        code => app.on_key(code),
                    }
                }
            }
        }

        if last_tick.elapsed() >= TICK {
            app.on_tick();
            last_tick = Instant::now();
        }
    }
}
