//! DTOs and command logic for the Tauri bridge.
//!
//! The free functions here take `&dyn SensePort` / `&mut dyn SensePort`,
//! so every behavior is unit-testable against [`MockSense`] without a
//! Tauri runtime; the `#[tauri::command]` wrappers in `main.rs` stay one
//! line each. Errors cross the bridge as the PT-BR [`SenseError`] Display
//! strings — the frontend shows them verbatim in the toast.
//!
//! [`MockSense`]: rusense_core::MockSense

use rusense_core::{
    Capabilities, FanDuty, FanMode, PowerSettings, SenseError, SensePort, Telemetry, UsbChargeLevel,
};
use serde::Serialize;

/// Everything the frontend needs to paint one frame of the cockpit.
#[derive(Debug, Clone, Serialize)]
pub struct UiState {
    pub telemetry: TelemetryDto,
    pub profiles: ProfilesDto,
    pub fan: FanDto,
    pub power: PowerDto,
    pub caps: CapsDto,
    /// True when running against the mock backend (`--mock` / `RUSENSE_MOCK=1`).
    pub mock: bool,
}

/// Live sensor readings.
#[derive(Debug, Clone, Serialize)]
pub struct TelemetryDto {
    pub cpu_rpm: u32,
    pub gpu_rpm: u32,
    /// `[cpu, gpu, system]` in °C.
    pub temps: [f32; 3],
    pub battery_pct: u8,
    pub battery_status: String,
}

/// Available platform profiles and the active one, by name.
#[derive(Debug, Clone, Serialize)]
pub struct ProfilesDto {
    pub available: Vec<String>,
    pub active: String,
}

/// Fan mode flattened for the frontend: `"auto"` / `"max"` / `"custom"`
/// plus the duty pair (aliased to `0,0` and `100,100` for auto/max).
#[derive(Debug, Clone, Serialize)]
pub struct FanDto {
    pub mode: String,
    pub cpu: u8,
    pub gpu: u8,
}

/// Power-related toggles.
#[derive(Debug, Clone, Serialize)]
pub struct PowerDto {
    pub limiter: bool,
    pub usb: u8,
    pub backlight: bool,
}

/// Feature groups the backend supports; the frontend hides absent cards.
#[derive(Debug, Clone, Serialize)]
pub struct CapsDto {
    pub fan_control: bool,
    pub power: bool,
    pub four_zone_kb: bool,
}

fn err_str(e: SenseError) -> String {
    e.to_string()
}

impl From<Telemetry> for TelemetryDto {
    fn from(t: Telemetry) -> Self {
        Self {
            cpu_rpm: t.fan_cpu_rpm,
            gpu_rpm: t.fan_gpu_rpm,
            temps: t.temps,
            battery_pct: t.battery_pct,
            battery_status: t.battery_status,
        }
    }
}

impl From<FanMode> for FanDto {
    fn from(m: FanMode) -> Self {
        match m {
            FanMode::Auto => Self {
                mode: "auto".into(),
                cpu: 0,
                gpu: 0,
            },
            FanMode::Max => Self {
                mode: "max".into(),
                cpu: 100,
                gpu: 100,
            },
            FanMode::Custom { cpu, gpu } => Self {
                mode: "custom".into(),
                cpu: cpu.get(),
                gpu: gpu.get(),
            },
        }
    }
}

impl From<PowerSettings> for PowerDto {
    fn from(p: PowerSettings) -> Self {
        Self {
            limiter: p.limiter,
            usb: p.usb.get(),
            backlight: p.backlight_timeout,
        }
    }
}

impl From<Capabilities> for CapsDto {
    fn from(c: Capabilities) -> Self {
        Self {
            fan_control: c.fan_control,
            power: c.power,
            four_zone_kb: c.four_zone_kb,
        }
    }
}

/// Read the full UI state from the port. Fan mode and power are only read
/// when the matching capability is present (mirroring the TUI): without it
/// the sysfs files are absent and the read would fail, and the frontend
/// hides those cards anyway.
pub fn read_state(port: &dyn SensePort, mock: bool) -> Result<UiState, String> {
    let caps = port.capabilities();
    let telemetry = port.telemetry().map_err(err_str)?.into();
    let profiles = port.profiles().map_err(err_str)?;
    let fan = if caps.fan_control {
        port.fan_mode().map_err(err_str)?.into()
    } else {
        FanDto::from(FanMode::Auto)
    };
    let power = if caps.power {
        port.power().map_err(err_str)?.into()
    } else {
        PowerDto {
            limiter: false,
            usb: 0,
            backlight: false,
        }
    };
    Ok(UiState {
        telemetry,
        profiles: ProfilesDto {
            available: profiles
                .available
                .iter()
                .map(|p| p.as_str().to_string())
                .collect(),
            active: profiles.active.as_str().to_string(),
        },
        fan,
        power,
        caps: caps.into(),
        mock,
    })
}

/// Resolve `name` against the port's available profiles and activate it.
pub fn apply_profile(port: &mut dyn SensePort, name: &str) -> Result<(), String> {
    let profile = port
        .profiles()
        .map_err(err_str)?
        .available
        .into_iter()
        .find(|p| p.as_str() == name)
        .ok_or_else(|| err_str(SenseError::UnknownProfile(name.to_string())))?;
    port.set_profile(&profile).map_err(err_str)
}

/// Apply a fan mode from its frontend encoding (`"auto"`/`"max"`/`"custom"`).
/// Duties are only meaningful for `"custom"` and validated by [`FanDuty`].
pub fn apply_fan(port: &mut dyn SensePort, mode: &str, cpu: u8, gpu: u8) -> Result<(), String> {
    let mode = match mode {
        "auto" => FanMode::Auto,
        "max" => FanMode::Max,
        "custom" => FanMode::custom(
            FanDuty::new(cpu).map_err(err_str)?,
            FanDuty::new(gpu).map_err(err_str)?,
        ),
        other => return Err(format!("modo de fan inválido: {other}")),
    };
    port.set_fan_mode(mode).map_err(err_str)
}

/// Apply the power toggles. `usb` is validated by [`UsbChargeLevel`]
/// (0/10/20/30). On failure the port may have applied earlier fields —
/// the frontend re-polls `state` after any error.
pub fn apply_power(
    port: &mut dyn SensePort,
    limiter: bool,
    usb: u8,
    backlight: bool,
) -> Result<(), String> {
    let usb = UsbChargeLevel::new(usb).map_err(err_str)?;
    port.set_power(PowerSettings {
        limiter,
        usb,
        backlight_timeout: backlight,
    })
    .map_err(err_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusense_core::MockSense;

    // --- read_state / DTO mapping ---

    #[test]
    fn ui_state_maps_mock_defaults() {
        let mock = MockSense::new();
        let s = read_state(&mock, true).unwrap();

        assert_eq!(
            s.profiles.available,
            ["low-power", "quiet", "balanced", "balanced-performance"]
        );
        assert_eq!(s.profiles.active, "balanced-performance");

        assert_eq!(s.fan.mode, "auto");
        assert_eq!((s.fan.cpu, s.fan.gpu), (0, 0));

        assert!(s.power.limiter);
        assert_eq!(s.power.usb, 30);
        assert!(s.power.backlight);

        assert!(s.caps.fan_control);
        assert!(s.caps.power);
        assert!(!s.caps.four_zone_kb);

        assert_eq!(s.telemetry.temps, [41.0, 35.0, 40.0]);
        assert_eq!(s.telemetry.battery_pct, 80);
        assert_eq!(s.telemetry.battery_status, "Not charging");
        assert!(s.telemetry.cpu_rpm > 0 && s.telemetry.gpu_rpm > 0);

        assert!(s.mock);
    }

    #[test]
    fn ui_state_reports_real_backend_without_mock_badge() {
        let mock = MockSense::new();
        assert!(!read_state(&mock, false).unwrap().mock);
    }

    #[test]
    fn ui_state_serializes_frontend_field_names() {
        let mock = MockSense::new();
        let v = serde_json::to_value(read_state(&mock, true).unwrap()).unwrap();
        assert_eq!(v["profiles"]["active"], "balanced-performance");
        assert_eq!(v["fan"]["mode"], "auto");
        assert_eq!(v["power"]["usb"], 30);
        assert_eq!(v["caps"]["four_zone_kb"], false);
        assert_eq!(v["telemetry"]["battery_pct"], 80);
        assert_eq!(v["mock"], true);
    }

    // --- apply_profile ---

    #[test]
    fn apply_profile_roundtrips_through_state() {
        let mut mock = MockSense::new();
        apply_profile(&mut mock, "quiet").unwrap();
        assert_eq!(read_state(&mock, true).unwrap().profiles.active, "quiet");
    }

    #[test]
    fn apply_profile_rejects_unknown_name_with_ptbr_message() {
        let mut mock = MockSense::new();
        let err = apply_profile(&mut mock, "turbo").unwrap_err();
        assert_eq!(err, "perfil desconhecido: turbo");
        // State stays untouched.
        assert_eq!(
            read_state(&mock, true).unwrap().profiles.active,
            "balanced-performance"
        );
    }

    // --- apply_fan ---

    #[test]
    fn apply_fan_custom_reaches_port_and_state() {
        let mut mock = MockSense::new();
        apply_fan(&mut mock, "custom", 50, 70).unwrap();
        assert_eq!(
            mock.fan_mode().unwrap(),
            FanMode::custom(FanDuty::new(50).unwrap(), FanDuty::new(70).unwrap())
        );
        let s = read_state(&mock, true).unwrap();
        assert_eq!(s.fan.mode, "custom");
        assert_eq!((s.fan.cpu, s.fan.gpu), (50, 70));
    }

    #[test]
    fn apply_fan_auto_and_max_ignore_duties() {
        let mut mock = MockSense::new();
        apply_fan(&mut mock, "max", 10, 20).unwrap();
        assert_eq!(mock.fan_mode().unwrap(), FanMode::Max);
        apply_fan(&mut mock, "auto", 10, 20).unwrap();
        assert_eq!(mock.fan_mode().unwrap(), FanMode::Auto);
    }

    #[test]
    fn apply_fan_custom_aliases_normalize() {
        let mut mock = MockSense::new();
        apply_fan(&mut mock, "custom", 100, 100).unwrap();
        assert_eq!(mock.fan_mode().unwrap(), FanMode::Max);
        assert_eq!(read_state(&mock, true).unwrap().fan.mode, "max");
    }

    #[test]
    fn apply_fan_rejects_unknown_mode_with_ptbr_message() {
        let mut mock = MockSense::new();
        let err = apply_fan(&mut mock, "turbo", 0, 0).unwrap_err();
        assert_eq!(err, "modo de fan inválido: turbo");
    }

    #[test]
    fn apply_fan_rejects_out_of_range_duty() {
        let mut mock = MockSense::new();
        let err = apply_fan(&mut mock, "custom", 150, 50).unwrap_err();
        assert_eq!(err, "valor fora do intervalo 0–100: 150");
        assert_eq!(mock.fan_mode().unwrap(), FanMode::Auto);
    }

    // --- apply_power ---

    #[test]
    fn apply_power_toggles_limiter() {
        let mut mock = MockSense::new();
        apply_power(&mut mock, false, 30, true).unwrap();
        let s = read_state(&mock, true).unwrap();
        assert!(!s.power.limiter);
        assert_eq!(s.power.usb, 30);
        assert!(s.power.backlight);
    }

    #[test]
    fn apply_power_rejects_invalid_usb_level() {
        let mut mock = MockSense::new();
        let err = apply_power(&mut mock, true, 15, true).unwrap_err();
        assert_eq!(err, "nível usb inválido (0/10/20/30): 15");
        // Nothing applied: the level is validated before touching the port.
        assert!(read_state(&mock, true).unwrap().power.limiter);
    }
}
