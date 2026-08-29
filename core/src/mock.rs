//! In-memory [`SensePort`] backend for tests, demos (`--mock`) and CI.

use std::cell::Cell;

use crate::domain::{
    Capabilities, FanMode, PowerSettings, Profile, ProfileSet, SenseError, Telemetry,
    UsbChargeLevel,
};
use crate::port::SensePort;

/// Baseline fan RPM readings (ANV15-52, balanced-performance, idle).
const BASE_CPU_RPM: u32 = 2348;
const BASE_GPU_RPM: u32 = 2081;

/// Mock backend whose default state mirrors a real Acer Nitro ANV15-52.
///
/// Fully deterministic: no clock, no randomness. Successive [`telemetry`]
/// calls drift the RPM readings via an internal call counter so the mock
/// feels alive while staying reproducible in tests.
///
/// [`telemetry`]: SensePort::telemetry
pub struct MockSense {
    caps: Capabilities,
    profiles: ProfileSet,
    fan: FanMode,
    power: PowerSettings,
    telemetry_calls: Cell<u32>,
}

impl MockSense {
    pub fn new() -> Self {
        Self {
            caps: Capabilities {
                fan_control: true,
                power: true,
                four_zone_kb: false,
            },
            profiles: ProfileSet::parse(
                "low-power quiet balanced balanced-performance",
                "balanced-performance",
            )
            .expect("literais de perfil válidos"),
            fan: FanMode::Auto,
            power: PowerSettings {
                limiter: true,
                usb: UsbChargeLevel::new(30).expect("30 é nível usb válido"),
                backlight_timeout: true,
            },
            telemetry_calls: Cell::new(0),
        }
    }

    /// Deterministic RPM drift: a small sawtooth in `-50..=49` derived from
    /// the call counter. The step (37 or 53) is coprime with 100, so two
    /// consecutive calls never yield the same offset.
    fn drift(n: u32, step: u32) -> i32 {
        (n.wrapping_mul(step) % 100) as i32 - 50
    }
}

impl Default for MockSense {
    fn default() -> Self {
        Self::new()
    }
}

impl SensePort for MockSense {
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    fn telemetry(&self) -> Result<Telemetry, SenseError> {
        let n = self.telemetry_calls.get();
        self.telemetry_calls.set(n.wrapping_add(1));
        Ok(Telemetry {
            fan_cpu_rpm: BASE_CPU_RPM.saturating_add_signed(Self::drift(n, 37)),
            fan_gpu_rpm: BASE_GPU_RPM.saturating_add_signed(Self::drift(n, 53)),
            temps: [41.0, 35.0, 40.0],
            battery_pct: 80,
            battery_status: "Not charging".to_string(),
        })
    }

    fn profiles(&self) -> Result<ProfileSet, SenseError> {
        Ok(self.profiles.clone())
    }

    fn set_profile(&mut self, p: &Profile) -> Result<(), SenseError> {
        if !self.profiles.available.contains(p) {
            return Err(SenseError::UnknownProfile(p.as_str().to_string()));
        }
        self.profiles.active = p.clone();
        Ok(())
    }

    fn fan_mode(&self) -> Result<FanMode, SenseError> {
        Ok(self.fan)
    }

    fn set_fan_mode(&mut self, m: FanMode) -> Result<(), SenseError> {
        self.fan = m;
        Ok(())
    }

    fn power(&self) -> Result<PowerSettings, SenseError> {
        Ok(self.power.clone())
    }

    fn set_power(&mut self, s: PowerSettings) -> Result<(), SenseError> {
        self.power = s;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::FanDuty;

    // --- default state ---

    #[test]
    fn default_state_mirrors_anv15_52() {
        let mock = MockSense::new();

        let profiles = mock.profiles().unwrap();
        let names: Vec<&str> = profiles.available.iter().map(|p| p.as_str()).collect();
        assert_eq!(
            names,
            ["low-power", "quiet", "balanced", "balanced-performance"]
        );
        assert_eq!(profiles.active.as_str(), "balanced-performance");

        assert_eq!(mock.fan_mode().unwrap(), FanMode::Auto);

        let power = mock.power().unwrap();
        assert!(power.limiter);
        assert_eq!(power.usb.get(), 30);
        assert!(power.backlight_timeout);

        let t = mock.telemetry().unwrap();
        assert_eq!(t.temps, [41.0, 35.0, 40.0]);
        assert_eq!(t.battery_pct, 80);
        assert_eq!(t.battery_status, "Not charging");
    }

    // --- capabilities ---

    #[test]
    fn capabilities_reflect_construction() {
        let mock = MockSense::new();
        let caps = mock.capabilities();
        assert!(caps.fan_control);
        assert!(caps.power);
        assert!(!caps.four_zone_kb);
    }

    // --- profiles ---

    #[test]
    fn set_profile_roundtrips_through_active() {
        let mut mock = MockSense::new();
        let quiet = mock
            .profiles()
            .unwrap()
            .available
            .iter()
            .find(|p| p.as_str() == "quiet")
            .unwrap()
            .clone();
        mock.set_profile(&quiet).unwrap();
        assert_eq!(mock.profiles().unwrap().active, quiet);
    }

    #[test]
    fn set_profile_rejects_unknown_profile() {
        let mut mock = MockSense::new();
        let err = mock.set_profile(&Profile::new("turbo")).unwrap_err();
        assert_eq!(err, SenseError::UnknownProfile("turbo".into()));
        // State stays untouched.
        assert_eq!(
            mock.profiles().unwrap().active.as_str(),
            "balanced-performance"
        );
    }

    // --- fan mode ---

    #[test]
    fn set_fan_mode_roundtrips() {
        let mut mock = MockSense::new();
        let custom = FanMode::custom(FanDuty::new(40).unwrap(), FanDuty::new(70).unwrap());
        mock.set_fan_mode(custom).unwrap();
        assert_eq!(mock.fan_mode().unwrap(), custom);

        mock.set_fan_mode(FanMode::Max).unwrap();
        assert_eq!(mock.fan_mode().unwrap(), FanMode::Max);
    }

    // --- power ---

    #[test]
    fn set_power_roundtrips() {
        let mut mock = MockSense::new();
        let settings = PowerSettings {
            limiter: false,
            usb: UsbChargeLevel::new(10).unwrap(),
            backlight_timeout: false,
        };
        mock.set_power(settings.clone()).unwrap();
        assert_eq!(mock.power().unwrap(), settings);
    }

    // --- telemetry drift ---

    #[test]
    fn telemetry_rpm_drifts_deterministically_within_band() {
        let mock = MockSense::new();
        let t1 = mock.telemetry().unwrap();
        let t2 = mock.telemetry().unwrap();

        // Successive reads differ (the mock feels alive)...
        assert_ne!(t1.fan_cpu_rpm, t2.fan_cpu_rpm);
        assert_ne!(t1.fan_gpu_rpm, t2.fan_gpu_rpm);

        // ...but stay in a plausible band around the ANV15-52 baseline.
        for t in [&t1, &t2] {
            assert!(
                (2248..=2448).contains(&t.fan_cpu_rpm),
                "cpu rpm fora da banda: {}",
                t.fan_cpu_rpm
            );
            assert!(
                (1981..=2181).contains(&t.fan_gpu_rpm),
                "gpu rpm fora da banda: {}",
                t.fan_gpu_rpm
            );
        }

        // Deterministic: a fresh mock replays the same sequence.
        let replay = MockSense::new();
        assert_eq!(replay.telemetry().unwrap().fan_cpu_rpm, t1.fan_cpu_rpm);
        assert_eq!(replay.telemetry().unwrap().fan_cpu_rpm, t2.fan_cpu_rpm);
    }
}
