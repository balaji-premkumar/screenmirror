//! Fallback for any target with no platform module of its own.
//!
//! Reporting [`DriverStatus::Ready`] is deliberate: it says "there is nothing
//! this app knows how to install here", which lets the rest of the app run and
//! fail at the point where it actually cannot open the device, with a real
//! error. The alternative — reporting `NeedsInstall` — would show the user a
//! "Fix USB Permissions" button that cannot do anything.
//!
//! To add a platform: write `platform/<os>.rs` with these four functions and
//! add a `cfg_attr` line in `platform/mod.rs`.

use super::{DriverStatus, InstallOutcome};

pub(super) fn driver_status() -> DriverStatus {
    DriverStatus::Ready
}

pub(super) fn install_driver() -> InstallOutcome {
    InstallOutcome::NotNeeded
}

pub(super) fn obs_plugin_dir() -> Option<std::path::PathBuf> {
    None
}

pub(super) fn default_obs_plugin_dir() -> Option<std::path::PathBuf> {
    None
}
