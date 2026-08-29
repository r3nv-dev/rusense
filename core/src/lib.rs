//! rusense-core: domain types and ports for RuSense.

pub mod domain;
pub mod history;
pub mod mock;
pub mod port;
pub mod sysfs;

pub use domain::*;
pub use history::*;
pub use mock::*;
pub use port::*;
pub use sysfs::*;

/// Connect to a backend: [`MockSense`] when `mock` is true, otherwise the
/// real driver via [`SysfsSense::discover`].
///
/// # Errors
/// [`SenseError::DriverMissing`] when `mock` is false and the linuwu_sense
/// driver is not loaded.
pub fn connect(mock: bool) -> Result<Box<dyn SensePort + Send>, SenseError> {
    if mock {
        Ok(Box::new(MockSense::new()))
    } else {
        Ok(Box::new(SysfsSense::discover()?))
    }
}
