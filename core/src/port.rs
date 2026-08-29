//! The hexagonal boundary: what any Sense backend must provide.

use crate::domain::{
    Capabilities, FanMode, PowerSettings, Profile, ProfileSet, SenseError, Telemetry,
};

/// Port implemented by every backend (real sysfs driver, mock, ...).
///
/// Frontends depend only on this trait, never on a concrete adapter.
pub trait SensePort {
    fn capabilities(&self) -> &Capabilities;
    fn telemetry(&self) -> Result<Telemetry, SenseError>;
    fn profiles(&self) -> Result<ProfileSet, SenseError>;
    fn set_profile(&mut self, p: &Profile) -> Result<(), SenseError>;
    fn fan_mode(&self) -> Result<FanMode, SenseError>;
    fn set_fan_mode(&mut self, m: FanMode) -> Result<(), SenseError>;
    fn power(&self) -> Result<PowerSettings, SenseError>;
    fn set_power(&mut self, s: PowerSettings) -> Result<(), SenseError>;
}
