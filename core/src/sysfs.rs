//! [`SysfsSense`]: real backend over the linuwu_sense driver sysfs tree.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::domain::{
    Capabilities, FanMode, PowerSettings, Profile, ProfileSet, SenseError, Telemetry,
    UsbChargeLevel,
};
use crate::port::SensePort;

/// Production base of the linuwu_sense driver tree.
const DRIVER_BASE: &str = "/sys/devices/platform/acer-wmi";
/// Production base for battery discovery.
const POWER_SUPPLY: &str = "/sys/class/power_supply";

/// Adapter that talks to the linuwu_sense driver through sysfs.
///
/// Capabilities are probed once at construction. The acer `hwmon` entry
/// (the one under `hwmon/` whose `name` file reads `acer`) and the battery
/// are re-discovered on every read. Missing pieces degrade gracefully in
/// [`telemetry`] instead of erroring — mirroring the "hide what doesn't
/// exist" capability policy: without an acer hwmon, fan RPMs read as 0 and
/// temperatures as `[0.0; 3]`; without a `BAT*` entry under the
/// power-supply base (or when it vanishes mid-read), battery reads as 0% /
/// `"Unknown"` so desktops without battery still show fans.
///
/// [`telemetry`]: SensePort::telemetry
#[derive(Debug)]
pub struct SysfsSense {
    base: PathBuf,
    power_supply: PathBuf,
    caps: Capabilities,
}

/// Read a sysfs file, trimming surrounding whitespace/newlines.
fn read_trimmed(path: &Path) -> Result<String, SenseError> {
    fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .map_err(|e| SenseError::Io(format!("{}: {e}", path.display())))
}

/// Like [`read_trimmed`], but a vanished file yields `None` instead of an
/// error (TOCTOU: the device was discovered, then removed before the read).
fn read_if_present(path: &Path) -> Result<Option<String>, SenseError> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(Some(s.trim().to_string())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(SenseError::Io(format!("{}: {e}", path.display()))),
    }
}

/// Write a sysfs value, mapping permission failures to [`SenseError::ReadOnly`].
fn write_value(path: &Path, value: &str) -> Result<(), SenseError> {
    fs::write(path, value).map_err(|e| match e.kind() {
        ErrorKind::PermissionDenied => SenseError::ReadOnly,
        _ => SenseError::Io(format!("{}: {e}", path.display())),
    })
}

fn malformed(label: &str, raw: &str) -> SenseError {
    SenseError::Malformed(format!("{label}: {raw}"))
}

fn parse_u32(raw: &str, label: &str) -> Result<u32, SenseError> {
    raw.parse().map_err(|_| malformed(label, raw))
}

/// Parse a `"0"`/`"1"` sysfs toggle.
fn parse_flag(raw: &str, label: &str) -> Result<bool, SenseError> {
    match raw {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(malformed(label, raw)),
    }
}

fn flag(v: bool) -> &'static str {
    if v {
        "1"
    } else {
        "0"
    }
}

/// Millidegrees Celsius to degrees.
fn parse_millideg(raw: &str, label: &str) -> Result<f32, SenseError> {
    raw.parse::<i32>()
        .map(|v| v as f32 / 1000.0)
        .map_err(|_| malformed(label, raw))
}

/// The entry under `dir` whose `name` file reads `acer`, if any.
fn find_hwmon(dir: &Path) -> Option<PathBuf> {
    fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(
        |path| matches!(fs::read_to_string(path.join("name")), Ok(name) if name.trim() == "acer"),
    )
}

/// The first `BAT*` entry under `dir` (sorted for determinism), if any.
fn find_battery(dir: &Path) -> Option<PathBuf> {
    let mut bats: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("BAT"))
        .map(|e| e.path())
        .collect();
    bats.sort();
    bats.into_iter().next()
}

impl SysfsSense {
    /// Open the driver tree at `base`, discovering batteries under
    /// `power_supply`. Probes capabilities here.
    ///
    /// # Errors
    /// [`SenseError::DriverMissing`] when `base` does not exist, or exists
    /// without a `nitro_sense`/`predator_sense` attribute group — stock
    /// acer-wmi creates the same platform dir, and those users need the
    /// install hint, not zeroed capabilities.
    pub fn new(base: PathBuf, power_supply: PathBuf) -> Result<Self, SenseError> {
        if !base.is_dir() {
            return Err(SenseError::DriverMissing(base.display().to_string()));
        }
        let nitro = base.join("nitro_sense");
        if !nitro.is_dir() && !base.join("predator_sense").is_dir() {
            return Err(SenseError::DriverMissing(base.display().to_string()));
        }
        let caps = Capabilities {
            fan_control: nitro.join("fan_speed").is_file(),
            // battery_limiter stands in for the whole nitro_sense attribute
            // group — the driver creates the group atomically.
            power: nitro.join("battery_limiter").is_file(),
            four_zone_kb: base.join("four_zoned_kb").is_dir(),
        };
        Ok(Self {
            base,
            power_supply,
            caps,
        })
    }

    /// Open the production paths (`/sys/devices/platform/acer-wmi` and
    /// `/sys/class/power_supply`).
    ///
    /// # Errors
    /// [`SenseError::DriverMissing`] when the driver is not loaded.
    pub fn discover() -> Result<Self, SenseError> {
        Self::new(PathBuf::from(DRIVER_BASE), PathBuf::from(POWER_SUPPLY))
    }

    fn nitro(&self, file: &str) -> PathBuf {
        self.base.join("nitro_sense").join(file)
    }

    /// The single `platform-profile/platform-profile-N` entry. `N` comes
    /// from a kernel-global IDA, so it depends on registration order
    /// (amd-pmf etc. may claim 0 first) — discovered, never hardcoded.
    fn profile_dir(&self) -> Result<PathBuf, SenseError> {
        let dir = self.base.join("platform-profile");
        fs::read_dir(&dir)
            .map_err(|e| SenseError::Io(format!("{}: {e}", dir.display())))?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.is_dir())
            .ok_or_else(|| SenseError::Io(format!("{}: sem entradas", dir.display())))
    }

    /// Battery percent and status, defaulting to `(0, "Unknown")` when no
    /// `BAT*` entry exists — or when it vanishes between discovery and read.
    fn battery(&self) -> Result<(u8, String), SenseError> {
        let absent = || (0, "Unknown".to_string());
        let Some(bat) = find_battery(&self.power_supply) else {
            return Ok(absent());
        };
        let Some(raw) = read_if_present(&bat.join("capacity"))? else {
            return Ok(absent());
        };
        let pct = raw.parse().map_err(|_| malformed("capacity", &raw))?;
        let Some(status) = read_if_present(&bat.join("status"))? else {
            return Ok(absent());
        };
        Ok((pct, status))
    }
}

impl SensePort for SysfsSense {
    fn capabilities(&self) -> Capabilities {
        self.caps
    }

    fn telemetry(&self) -> Result<Telemetry, SenseError> {
        let (fan_cpu_rpm, fan_gpu_rpm, temps) = match find_hwmon(&self.base.join("hwmon")) {
            Some(hw) => {
                let rpm = |file: &str| parse_u32(&read_trimmed(&hw.join(file))?, file);
                let temp = |file: &str| parse_millideg(&read_trimmed(&hw.join(file))?, file);
                (
                    rpm("fan1_input")?,
                    rpm("fan2_input")?,
                    [
                        temp("temp1_input")?,
                        temp("temp2_input")?,
                        temp("temp3_input")?,
                    ],
                )
            }
            None => (0, 0, [0.0; 3]),
        };
        let (battery_pct, battery_status) = self.battery()?;
        Ok(Telemetry {
            fan_cpu_rpm,
            fan_gpu_rpm,
            temps,
            battery_pct,
            battery_status,
        })
    }

    fn profiles(&self) -> Result<ProfileSet, SenseError> {
        let dir = self.profile_dir()?;
        let choices = read_trimmed(&dir.join("choices"))?;
        let active = read_trimmed(&dir.join("profile"))?;
        ProfileSet::parse(&choices, &active)
    }

    fn set_profile(&mut self, p: &Profile) -> Result<(), SenseError> {
        let dir = self.profile_dir()?;
        let choices = read_trimmed(&dir.join("choices"))?;
        if !choices.split_whitespace().any(|c| c == p.as_str()) {
            return Err(SenseError::UnknownProfile(p.as_str().to_string()));
        }
        write_value(&dir.join("profile"), p.as_str())
    }

    fn fan_mode(&self) -> Result<FanMode, SenseError> {
        FanMode::from_sysfs(&read_trimmed(&self.nitro("fan_speed"))?)
    }

    fn set_fan_mode(&mut self, m: FanMode) -> Result<(), SenseError> {
        write_value(&self.nitro("fan_speed"), &m.to_sysfs())
    }

    fn power(&self) -> Result<PowerSettings, SenseError> {
        let limiter = parse_flag(
            &read_trimmed(&self.nitro("battery_limiter"))?,
            "battery_limiter",
        )?;
        let raw = read_trimmed(&self.nitro("usb_charging"))?;
        let usb = raw
            .parse()
            .ok()
            .and_then(|v| UsbChargeLevel::new(v).ok())
            .ok_or_else(|| malformed("usb_charging", &raw))?;
        let backlight_timeout = parse_flag(
            &read_trimmed(&self.nitro("backlight_timeout"))?,
            "backlight_timeout",
        )?;
        Ok(PowerSettings {
            limiter,
            usb,
            backlight_timeout,
        })
    }

    fn set_power(&mut self, s: PowerSettings) -> Result<(), SenseError> {
        write_value(&self.nitro("battery_limiter"), flag(s.limiter))?;
        write_value(&self.nitro("usb_charging"), &s.usb.get().to_string())?;
        write_value(&self.nitro("backlight_timeout"), flag(s.backlight_timeout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{FanDuty, UsbChargeLevel};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use tempfile::TempDir;

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    /// Fake driver tree mirroring the real ANV15-52 layout, plus decoys
    /// (a non-acer hwmon and a non-battery power_supply entry) so the
    /// adapter must resolve by name, not by directory order.
    fn fake_sysfs() -> (TempDir, PathBuf, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("acer-wmi");
        let power = tmp.path().join("power_supply");

        write(&base.join("nitro_sense/fan_speed"), "0,0\n");
        write(&base.join("nitro_sense/battery_limiter"), "1\n");
        write(&base.join("nitro_sense/battery_calibration"), "0\n");
        write(&base.join("nitro_sense/usb_charging"), "30\n");
        write(&base.join("nitro_sense/backlight_timeout"), "1\n");
        write(
            &base.join("platform-profile/platform-profile-0/choices"),
            "low-power quiet balanced balanced-performance\n",
        );
        write(
            &base.join("platform-profile/platform-profile-0/profile"),
            "balanced-performance\n",
        );
        write(&base.join("hwmon/hwmon0/name"), "nvme\n");
        write(&base.join("hwmon/hwmon4/name"), "acer\n");
        write(&base.join("hwmon/hwmon4/fan1_input"), "2348\n");
        write(&base.join("hwmon/hwmon4/fan2_input"), "2081\n");
        write(&base.join("hwmon/hwmon4/temp1_input"), "41000\n");
        write(&base.join("hwmon/hwmon4/temp2_input"), "35000\n");
        write(&base.join("hwmon/hwmon4/temp3_input"), "40000\n");
        write(&power.join("ACAD/type"), "Mains\n");
        write(&power.join("BAT1/capacity"), "80\n");
        write(&power.join("BAT1/status"), "Not charging\n");

        (tmp, base, power)
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn reads_telemetry_from_hwmon() {
        let (_tmp, base, power) = fake_sysfs();
        let sense = SysfsSense::new(base, power).unwrap();
        let t = sense.telemetry().unwrap();
        assert_eq!(t.fan_cpu_rpm, 2348);
        assert_eq!(t.fan_gpu_rpm, 2081);
        assert_eq!(t.temps, [41.0, 35.0, 40.0]);
        assert_eq!(t.battery_pct, 80);
        assert_eq!(t.battery_status, "Not charging");
    }

    #[test]
    fn set_fan_writes_pair() {
        let (_tmp, base, power) = fake_sysfs();
        let mut sense = SysfsSense::new(base.clone(), power).unwrap();
        let custom = FanMode::custom(FanDuty::new(50).unwrap(), FanDuty::new(70).unwrap());
        sense.set_fan_mode(custom).unwrap();
        assert_eq!(read(&base.join("nitro_sense/fan_speed")), "50,70");
        assert_eq!(sense.fan_mode().unwrap(), custom);

        sense.set_fan_mode(FanMode::Max).unwrap();
        assert_eq!(read(&base.join("nitro_sense/fan_speed")), "100,100");
        assert_eq!(sense.fan_mode().unwrap(), FanMode::Max);
    }

    #[test]
    fn set_profile_writes_and_validates() {
        let (_tmp, base, power) = fake_sysfs();
        let mut sense = SysfsSense::new(base.clone(), power).unwrap();
        let quiet = sense
            .profiles()
            .unwrap()
            .available
            .iter()
            .find(|p| p.as_str() == "quiet")
            .unwrap()
            .clone();
        sense.set_profile(&quiet).unwrap();
        let profile_file = base.join("platform-profile/platform-profile-0/profile");
        assert_eq!(read(&profile_file), "quiet");
        assert_eq!(sense.profiles().unwrap().active, quiet);

        let err = sense.set_profile(&Profile::new("turbo")).unwrap_err();
        assert_eq!(err, SenseError::UnknownProfile("turbo".into()));
        // Rejected profile must not be written.
        assert_eq!(read(&profile_file), "quiet");
    }

    #[test]
    fn missing_four_zone_dir_disables_capability() {
        let (_tmp, base, power) = fake_sysfs();
        let sense = SysfsSense::new(base.clone(), power.clone()).unwrap();
        let caps = sense.capabilities();
        assert!(caps.fan_control);
        assert!(caps.power);
        assert!(!caps.four_zone_kb);

        fs::create_dir_all(base.join("four_zoned_kb")).unwrap();
        let sense = SysfsSense::new(base, power).unwrap();
        assert!(sense.capabilities().four_zone_kb);
    }

    #[test]
    fn write_to_readonly_file_maps_to_readonly_error() {
        let (_tmp, base, power) = fake_sysfs();
        let mut sense = SysfsSense::new(base.clone(), power).unwrap();
        let fan_file = base.join("nitro_sense/fan_speed");
        fs::set_permissions(&fan_file, fs::Permissions::from_mode(0o444)).unwrap();
        let err = sense.set_fan_mode(FanMode::Max).unwrap_err();
        assert_eq!(err, SenseError::ReadOnly);
    }

    #[test]
    fn missing_driver_dir_yields_driver_missing() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("nope");
        let err = SysfsSense::new(missing.clone(), tmp.path().to_path_buf()).unwrap_err();
        assert_eq!(
            err,
            SenseError::DriverMissing(missing.display().to_string())
        );
    }

    #[test]
    fn power_roundtrip() {
        let (_tmp, base, power) = fake_sysfs();
        let mut sense = SysfsSense::new(base.clone(), power).unwrap();
        let current = sense.power().unwrap();
        assert!(current.limiter);
        assert_eq!(current.usb.get(), 30);
        assert!(current.backlight_timeout);

        let next = PowerSettings {
            limiter: false,
            usb: UsbChargeLevel::new(10).unwrap(),
            backlight_timeout: false,
        };
        sense.set_power(next.clone()).unwrap();
        assert_eq!(read(&base.join("nitro_sense/battery_limiter")), "0");
        assert_eq!(read(&base.join("nitro_sense/usb_charging")), "10");
        assert_eq!(read(&base.join("nitro_sense/backlight_timeout")), "0");
        assert_eq!(sense.power().unwrap(), next);
    }

    #[test]
    fn no_battery_dir_defaults() {
        let (_tmp, base, power) = fake_sysfs();
        fs::remove_dir_all(&power).unwrap();
        fs::create_dir_all(&power).unwrap();
        let sense = SysfsSense::new(base, power).unwrap();
        let t = sense.telemetry().unwrap();
        assert_eq!(t.battery_pct, 0);
        assert_eq!(t.battery_status, "Unknown");
        // Fans still report — desktops without battery must show them.
        assert_eq!(t.fan_cpu_rpm, 2348);
    }

    #[test]
    fn missing_hwmon_reports_zeros() {
        let (_tmp, base, power) = fake_sysfs();
        let sense = SysfsSense::new(base.clone(), power).unwrap();
        // Removed after construction: hwmon resolution happens per call.
        fs::remove_dir_all(base.join("hwmon")).unwrap();
        let t = sense.telemetry().unwrap();
        assert_eq!(t.fan_cpu_rpm, 0);
        assert_eq!(t.fan_gpu_rpm, 0);
        assert_eq!(t.temps, [0.0; 3]);
        // Battery still reports.
        assert_eq!(t.battery_pct, 80);
        assert_eq!(t.battery_status, "Not charging");
    }

    #[test]
    fn battery_vanishing_mid_read_defaults() {
        let (_tmp, base, power) = fake_sysfs();
        // TOCTOU: BAT1 is discovered but its files vanish before the read.
        fs::remove_file(power.join("BAT1/capacity")).unwrap();
        let sense = SysfsSense::new(base, power).unwrap();
        let t = sense.telemetry().unwrap();
        assert_eq!(t.battery_pct, 0);
        assert_eq!(t.battery_status, "Unknown");
        assert_eq!(t.fan_cpu_rpm, 2348);
    }

    #[test]
    fn profile_dir_index_is_discovered_not_hardcoded() {
        // The platform-profile index comes from a kernel-global IDA and is
        // nonzero when another driver (amd-pmf, ...) registers first.
        let (_tmp, base, power) = fake_sysfs();
        fs::rename(
            base.join("platform-profile/platform-profile-0"),
            base.join("platform-profile/platform-profile-3"),
        )
        .unwrap();
        let mut sense = SysfsSense::new(base.clone(), power).unwrap();
        let profiles = sense.profiles().unwrap();
        assert_eq!(profiles.active.as_str(), "balanced-performance");

        let quiet = profiles
            .available
            .iter()
            .find(|p| p.as_str() == "quiet")
            .unwrap()
            .clone();
        sense.set_profile(&quiet).unwrap();
        assert_eq!(
            read(&base.join("platform-profile/platform-profile-3/profile")),
            "quiet"
        );
    }

    #[test]
    fn stock_acer_wmi_without_sense_group_yields_driver_missing() {
        // Stock acer-wmi creates the same platform dir, minus the sense
        // attribute groups — that still counts as driver missing.
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("acer-wmi");
        fs::create_dir_all(&base).unwrap();
        let err = SysfsSense::new(base.clone(), tmp.path().to_path_buf()).unwrap_err();
        assert_eq!(err, SenseError::DriverMissing(base.display().to_string()));
    }

    #[test]
    fn predator_sense_dir_is_recognized_as_driver_present() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("acer-wmi");
        fs::create_dir_all(base.join("predator_sense")).unwrap();
        let sense = SysfsSense::new(base, tmp.path().to_path_buf()).unwrap();
        // Functional support stays nitro_sense-only for now.
        let caps = sense.capabilities();
        assert!(!caps.fan_control);
        assert!(!caps.power);
    }

    #[test]
    fn malformed_fan_input_maps_to_malformed() {
        let (_tmp, base, power) = fake_sysfs();
        write(&base.join("hwmon/hwmon4/fan1_input"), "banana\n");
        let sense = SysfsSense::new(base, power).unwrap();
        assert!(matches!(
            sense.telemetry().unwrap_err(),
            SenseError::Malformed(_)
        ));
    }

    #[test]
    fn malformed_limiter_flag_maps_to_malformed() {
        let (_tmp, base, power) = fake_sysfs();
        write(&base.join("nitro_sense/battery_limiter"), "2\n");
        let sense = SysfsSense::new(base, power).unwrap();
        assert!(matches!(
            sense.power().unwrap_err(),
            SenseError::Malformed(_)
        ));
    }

    #[test]
    fn usb_level_outside_set_maps_to_malformed() {
        let (_tmp, base, power) = fake_sysfs();
        write(&base.join("nitro_sense/usb_charging"), "15\n");
        let sense = SysfsSense::new(base, power).unwrap();
        assert!(matches!(
            sense.power().unwrap_err(),
            SenseError::Malformed(_)
        ));
    }

    #[test]
    fn fan_speed_out_of_range_maps_to_malformed() {
        // Regression for the error contract: out-of-range driver content is
        // Malformed (driver's fault), never InvalidDuty (user's fault).
        let (_tmp, base, power) = fake_sysfs();
        write(&base.join("nitro_sense/fan_speed"), "150,50\n");
        let sense = SysfsSense::new(base, power).unwrap();
        assert!(matches!(
            sense.fan_mode().unwrap_err(),
            SenseError::Malformed(_)
        ));
    }
}
