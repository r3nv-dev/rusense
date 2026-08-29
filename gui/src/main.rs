//! rusense-gui — GUI NitroSense clone (Tauri 2) over the Linuwu-Sense driver.
//!
//! `rusense-gui` talks to the real driver; `rusense-gui --mock` (or
//! `RUSENSE_MOCK=1`) runs anywhere. The frontend is the static cockpit in
//! `frontend/`; all logic lives in [`mod@state`] behind thin
//! `#[tauri::command]` wrappers.

mod state;

use std::env;
use std::process::ExitCode;
use std::sync::Mutex;

use rusense_core::{connect, SensePort};

use crate::state::UiState;

/// Managed Tauri state: the Sense backend plus session flags.
struct AppState {
    port: Mutex<Box<dyn SensePort + Send>>,
    mock: bool,
}

impl AppState {
    /// Lock the port. A poisoned mutex means a command panicked mid-write;
    /// there is no state to salvage, so panic with a PT-BR message.
    fn port(&self) -> std::sync::MutexGuard<'_, Box<dyn SensePort + Send>> {
        self.port.lock().expect("estado interno corrompido (mutex)")
    }
}

#[tauri::command]
fn state(app: tauri::State<'_, AppState>) -> Result<UiState, String> {
    let port = app.port();
    state::read_state(&**port, app.mock)
}

#[tauri::command]
fn set_profile(name: String, app: tauri::State<'_, AppState>) -> Result<UiState, String> {
    let mut port = app.port();
    state::apply_profile(&mut **port, &name)?;
    state::read_state(&**port, app.mock)
}

#[tauri::command]
fn set_fan(
    mode: String,
    cpu: u8,
    gpu: u8,
    app: tauri::State<'_, AppState>,
) -> Result<UiState, String> {
    let mut port = app.port();
    state::apply_fan(&mut **port, &mode, cpu, gpu)?;
    state::read_state(&**port, app.mock)
}

#[tauri::command]
fn set_power(
    limiter: bool,
    usb: u8,
    backlight: bool,
    app: tauri::State<'_, AppState>,
) -> Result<UiState, String> {
    let mut port = app.port();
    state::apply_power(&mut **port, limiter, usb, backlight)?;
    state::read_state(&**port, app.mock)
}

const USAGE: &str = "\
uso: rusense-gui [--mock]

  --mock      backend simulado (roda em qualquer máquina, sem driver)
              (RUSENSE_MOCK=1 no ambiente tem o mesmo efeito)
  -h, --help  mostra esta ajuda";

fn main() -> ExitCode {
    let mut mock = env::var("RUSENSE_MOCK").is_ok_and(|v| v == "1");
    for arg in env::args().skip(1) {
        match arg.as_str() {
            "--mock" => mock = true,
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

    // Fail before opening any window so the PT-BR message (e.g. the
    // DriverMissing install hint) lands readable on stderr.
    let port = match connect(mock) {
        Ok(port) => port,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let result = tauri::Builder::default()
        .manage(AppState {
            port: Mutex::new(port),
            mock,
        })
        .invoke_handler(tauri::generate_handler![
            state,
            set_profile,
            set_fan,
            set_power
        ])
        .run(tauri::generate_context!());

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
