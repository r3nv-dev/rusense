//! The hexagonal boundary: what any Sense backend must provide.

use crate::domain::{
    Capabilities, FanMode, PowerSettings, Profile, ProfileSet, SenseError, Telemetry,
};

/// Port implemented by every backend (real sysfs driver, mock, ...).
///
/// Frontends depend only on this trait, never on a concrete adapter.
/// The `# Errors` sections below are the error contract every adapter
/// must honor.
pub trait SensePort {
    /// Feature groups the backend supports. Infallible: detected once at
    /// construction and returned as a snapshot.
    fn capabilities(&self) -> Capabilities;

    /// Live sensor readings.
    ///
    /// # Errors
    /// [`SenseError::Io`] on read failure; [`SenseError::Malformed`] when
    /// the driver content cannot be parsed.
    fn telemetry(&self) -> Result<Telemetry, SenseError>;

    /// Available platform profiles and the active one.
    ///
    /// # Errors
    /// [`SenseError::Io`] / [`SenseError::Malformed`] when choices or the
    /// active value are unreadable or unparseable;
    /// [`SenseError::UnknownProfile`] when the driver reports an active
    /// profile that is not among the choices.
    fn profiles(&self) -> Result<ProfileSet, SenseError>;

    /// Activate a platform profile.
    ///
    /// # Errors
    /// [`SenseError::UnknownProfile`] when `p` is not in `available`;
    /// [`SenseError::ReadOnly`] when lacking write permission;
    /// [`SenseError::Io`] on other write failure.
    fn set_profile(&mut self, p: &Profile) -> Result<(), SenseError>;

    /// Current fan mode.
    ///
    /// # Errors
    /// [`SenseError::Io`] on read failure; [`SenseError::Malformed`] when
    /// the driver content cannot be parsed.
    fn fan_mode(&self) -> Result<FanMode, SenseError>;

    /// Apply a fan mode.
    ///
    /// # Errors
    /// [`SenseError::ReadOnly`] when lacking write permission;
    /// [`SenseError::Io`] on other write failure.
    fn set_fan_mode(&mut self, m: FanMode) -> Result<(), SenseError>;

    /// Current power-related toggles.
    ///
    /// # Errors
    /// [`SenseError::Io`] on read failure; [`SenseError::Malformed`] when
    /// the driver content cannot be parsed.
    fn power(&self) -> Result<PowerSettings, SenseError>;

    /// Apply power-related toggles.
    ///
    /// # Errors
    /// [`SenseError::ReadOnly`] when lacking write permission;
    /// [`SenseError::Io`] on other write failure.
    fn set_power(&mut self, s: PowerSettings) -> Result<(), SenseError>;
}
