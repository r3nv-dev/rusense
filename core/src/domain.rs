//! Domain value objects and enums for RuSense.

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SenseError {
    #[error("valor fora do intervalo 0–100: {0}")]
    InvalidDuty(u8),
    #[error("nível usb inválido (0/10/20/30): {0}")]
    InvalidUsbLevel(u8),
    #[error("perfil desconhecido: {0}")]
    UnknownProfile(String),
    #[error("sem permissão de escrita — rode o install.sh (regra udev)")]
    ReadOnly,
    #[error("driver linuwu_sense não encontrado em {0}")]
    DriverMissing(String),
    #[error("conteúdo inesperado do driver: {0}")]
    Malformed(String),
    #[error("io: {0}")]
    Io(String),
}

/// Fan duty cycle in percent (0–100).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FanDuty(u8);

impl FanDuty {
    pub fn new(v: u8) -> Result<Self, SenseError> {
        if v > 100 {
            Err(SenseError::InvalidDuty(v))
        } else {
            Ok(Self(v))
        }
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

/// Fan operating mode, serialized to/from the driver's `fan_speed` sysfs pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanMode {
    Auto,
    Max,
    Custom { cpu: FanDuty, gpu: FanDuty },
}

impl FanMode {
    /// Build a fan mode from explicit duties, normalizing the aliased states
    /// (0,0) -> `Auto` and (100,100) -> `Max` so they never coexist with
    /// equivalent `Custom` values.
    pub fn custom(cpu: FanDuty, gpu: FanDuty) -> Self {
        match (cpu.get(), gpu.get()) {
            (0, 0) => Self::Auto,
            (100, 100) => Self::Max,
            _ => Self::Custom { cpu, gpu },
        }
    }

    pub fn to_sysfs(self) -> String {
        match self {
            Self::Auto => "0,0".into(),
            Self::Max => "100,100".into(),
            Self::Custom { cpu, gpu } => format!("{},{}", cpu.get(), gpu.get()),
        }
    }

    /// Parse the driver's `fan_speed` pair. Any invalid content — including
    /// out-of-range duties — maps to [`SenseError::Malformed`]: the driver
    /// is at fault here, not user input (unlike [`FanDuty::new`]).
    pub fn from_sysfs(raw: &str) -> Result<Self, SenseError> {
        let trimmed = raw.trim();
        let malformed = || SenseError::Malformed(format!("fan_speed: {trimmed}"));
        let (cpu, gpu) = trimmed.split_once(',').ok_or_else(malformed)?;
        let parse = |s: &str| {
            let v = s.trim().parse::<u8>().map_err(|_| malformed())?;
            FanDuty::new(v).map_err(|_| malformed())
        };
        Ok(Self::custom(parse(cpu)?, parse(gpu)?))
    }
}

/// A platform power profile name as reported by the driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile(String);

impl Profile {
    /// Crate-internal constructor; frontends obtain profiles from
    /// [`ProfileSet::parse`] and clone from `available`.
    pub(crate) fn new(name: &str) -> Self {
        Self(name.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The set of available platform profiles and the currently active one.
#[derive(Debug, Clone)]
pub struct ProfileSet {
    pub available: Vec<Profile>,
    pub active: Profile,
}

impl ProfileSet {
    /// Parse the sysfs `platform_profile_choices` list and the active
    /// `platform_profile` value. The active profile must be one of the choices.
    pub fn parse(choices: &str, active: &str) -> Result<Self, SenseError> {
        let available: Vec<Profile> = choices.split_whitespace().map(Profile::new).collect();
        let active = active.trim();
        match available.iter().find(|p| p.as_str() == active) {
            Some(profile) => Ok(Self {
                active: profile.clone(),
                available,
            }),
            None => Err(SenseError::UnknownProfile(active.to_string())),
        }
    }
}

/// USB charge level while the laptop is off: percent of battery reserved (0 disables).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbChargeLevel(u8);

impl UsbChargeLevel {
    pub fn new(v: u8) -> Result<Self, SenseError> {
        match v {
            0 | 10 | 20 | 30 => Ok(Self(v)),
            _ => Err(SenseError::InvalidUsbLevel(v)),
        }
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

/// Live sensor readings.
#[derive(Debug, Clone)]
pub struct Telemetry {
    pub fan_cpu_rpm: u32,
    pub fan_gpu_rpm: u32,
    pub temps: [f32; 3],
    pub battery_pct: u8,
    pub battery_status: String,
}

/// Power-related toggles exposed by the driver.
#[derive(Debug, Clone, PartialEq)]
pub struct PowerSettings {
    pub limiter: bool,
    pub usb: UsbChargeLevel,
    pub backlight_timeout: bool,
}

/// Which feature groups the detected hardware supports.
#[derive(Debug, Clone, Copy)]
pub struct Capabilities {
    pub fan_control: bool,
    pub power: bool,
    pub four_zone_kb: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- FanDuty ---

    #[test]
    fn fan_duty_rejects_over_100() {
        assert!(FanDuty::new(101).is_err());
    }

    #[test]
    fn fan_duty_accepts_bounds() {
        assert_eq!(FanDuty::new(0).unwrap().get(), 0);
        assert_eq!(FanDuty::new(100).unwrap().get(), 100);
    }

    #[test]
    fn fan_duty_error_carries_value_and_ptbr_message() {
        let err = FanDuty::new(101).unwrap_err();
        assert_eq!(err, SenseError::InvalidDuty(101));
        assert_eq!(err.to_string(), "valor fora do intervalo 0–100: 101");
    }

    // --- FanMode ---

    #[test]
    fn fan_speed_serializes_as_pair() {
        assert_eq!(FanMode::Auto.to_sysfs(), "0,0");
        assert_eq!(FanMode::Max.to_sysfs(), "100,100");
        assert_eq!(
            FanMode::Custom {
                cpu: FanDuty::new(50).unwrap(),
                gpu: FanDuty::new(70).unwrap()
            }
            .to_sysfs(),
            "50,70"
        );
    }

    #[test]
    fn fan_mode_parses_from_sysfs() {
        assert_eq!(FanMode::from_sysfs("0,0").unwrap(), FanMode::Auto);
        assert_eq!(FanMode::from_sysfs("100,100").unwrap(), FanMode::Max);
        assert_eq!(
            FanMode::from_sysfs("50,70").unwrap(),
            FanMode::Custom {
                cpu: FanDuty::new(50).unwrap(),
                gpu: FanDuty::new(70).unwrap()
            }
        );
    }

    #[test]
    fn fan_mode_from_sysfs_trims_whitespace() {
        assert_eq!(FanMode::from_sysfs(" 100,100\n").unwrap(), FanMode::Max);
    }

    #[test]
    fn fan_mode_from_sysfs_rejects_malformed_input() {
        assert!(matches!(
            FanMode::from_sysfs("banana").unwrap_err(),
            SenseError::Malformed(_)
        ));
        assert!(matches!(
            FanMode::from_sysfs("50").unwrap_err(),
            SenseError::Malformed(_)
        ));
        assert!(matches!(
            FanMode::from_sysfs("50,70,90").unwrap_err(),
            SenseError::Malformed(_)
        ));
    }

    #[test]
    fn fan_mode_from_sysfs_rejects_out_of_range_duty() {
        // Driver content out of range is the driver's fault: Malformed,
        // never InvalidDuty (that one is for direct user construction).
        assert_eq!(
            FanMode::from_sysfs("150,50").unwrap_err(),
            SenseError::Malformed("fan_speed: 150,50".into())
        );
    }

    #[test]
    fn fan_mode_from_sysfs_classifies_by_value_not_raw_string() {
        assert_eq!(FanMode::from_sysfs("0, 0").unwrap(), FanMode::Auto);
        assert_eq!(FanMode::from_sysfs("100, 100").unwrap(), FanMode::Max);
    }

    #[test]
    fn fan_mode_custom_normalizes_aliased_states() {
        let d = |v| FanDuty::new(v).unwrap();
        assert_eq!(FanMode::custom(d(0), d(0)), FanMode::Auto);
        assert_eq!(FanMode::custom(d(100), d(100)), FanMode::Max);
        assert_eq!(
            FanMode::custom(d(50), d(70)),
            FanMode::Custom {
                cpu: d(50),
                gpu: d(70)
            }
        );
    }

    #[test]
    fn fan_mode_aliased_custom_parses_back_normalized() {
        let d = |v| FanDuty::new(v).unwrap();
        let zero = FanMode::Custom {
            cpu: d(0),
            gpu: d(0),
        };
        assert_eq!(
            FanMode::from_sysfs(&zero.to_sysfs()).unwrap(),
            FanMode::Auto
        );
        let full = FanMode::Custom {
            cpu: d(100),
            gpu: d(100),
        };
        assert_eq!(FanMode::from_sysfs(&full.to_sysfs()).unwrap(), FanMode::Max);
    }

    #[test]
    fn fan_mode_sysfs_roundtrip() {
        let modes = [
            FanMode::Auto,
            FanMode::Max,
            FanMode::Custom {
                cpu: FanDuty::new(30).unwrap(),
                gpu: FanDuty::new(60).unwrap(),
            },
        ];
        for mode in modes {
            assert_eq!(FanMode::from_sysfs(&mode.to_sysfs()).unwrap(), mode);
        }
    }

    // --- Profile / ProfileSet ---

    #[test]
    fn profile_parses_dynamic_choices() {
        let p = ProfileSet::parse(
            "low-power quiet balanced balanced-performance",
            "balanced-performance",
        )
        .unwrap();
        assert_eq!(p.available.len(), 4);
        assert_eq!(p.active.as_str(), "balanced-performance");
    }

    #[test]
    fn profile_parse_trims_sysfs_newlines() {
        let p = ProfileSet::parse("quiet balanced\n", "balanced\n").unwrap();
        assert_eq!(p.available.len(), 2);
        assert_eq!(p.active.as_str(), "balanced");
    }

    #[test]
    fn profile_parse_rejects_active_outside_choices() {
        let err = ProfileSet::parse("quiet balanced", "turbo").unwrap_err();
        assert_eq!(err, SenseError::UnknownProfile("turbo".into()));
    }

    #[test]
    fn profile_parse_empty_choices_rejects_any_active() {
        assert_eq!(
            ProfileSet::parse("", "x").unwrap_err(),
            SenseError::UnknownProfile("x".into())
        );
    }

    #[test]
    fn profile_new_constructs_named_profile() {
        assert_eq!(Profile::new("quiet").as_str(), "quiet");
    }

    // --- SenseError ---

    #[test]
    fn sense_error_is_cloneable_eq_with_ptbr_malformed_message() {
        let e = SenseError::Malformed("fan_speed: banana".into());
        assert_eq!(e.clone(), e);
        assert_eq!(
            e.to_string(),
            "conteúdo inesperado do driver: fan_speed: banana"
        );
    }

    // --- UsbChargeLevel ---

    #[test]
    fn usb_charge_level_accepts_valid_levels() {
        for v in [0u8, 10, 20, 30] {
            assert_eq!(UsbChargeLevel::new(v).unwrap().get(), v);
        }
    }

    #[test]
    fn usb_charge_level_rejects_other_values() {
        let err = UsbChargeLevel::new(15).unwrap_err();
        assert_eq!(err, SenseError::InvalidUsbLevel(15));
        assert_eq!(err.to_string(), "nível usb inválido (0/10/20/30): 15");
    }

    // --- Telemetry ---

    #[test]
    fn telemetry_holds_plain_data_and_clones() {
        let t = Telemetry {
            fan_cpu_rpm: 3200,
            fan_gpu_rpm: 2800,
            temps: [55.0, 62.5, 48.0],
            battery_pct: 80,
            battery_status: "Charging".to_string(),
        };
        let c = t.clone();
        assert_eq!(c.fan_cpu_rpm, 3200);
        assert_eq!(c.temps[1], 62.5);
        assert_eq!(c.battery_status, "Charging");
    }

    // --- PowerSettings ---

    #[test]
    fn power_settings_compares_by_value() {
        let a = PowerSettings {
            limiter: true,
            usb: UsbChargeLevel::new(10).unwrap(),
            backlight_timeout: false,
        };
        let b = a.clone();
        assert_eq!(a, b);
    }

    // --- Capabilities ---

    #[test]
    fn capabilities_is_copy() {
        let a = Capabilities {
            fan_control: true,
            power: true,
            four_zone_kb: false,
        };
        let b = a; // Copy: `a` stays usable.
        assert!(a.fan_control);
        assert!(b.power);
        assert!(!b.four_zone_kb);
    }
}
