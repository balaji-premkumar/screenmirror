//! macOS: no driver to install, but OBS keeps its plugins somewhere specific.
//!
//! IOKit lets a user-space process claim a USB interface that no kernel driver
//! has taken, and nothing claims an Android accessory interface, so libusb can
//! open it with no setup at all. That is why [`install_driver`] is a no-op
//! rather than a stub waiting to be written.
//!
//! [`obs_plugin_dir`] previously returned `None` on this platform, which made
//! the desktop app report "OBS: Plugin Missing" forever with no way to fix it
//! (issue #7). The path below is the standard OBS 28+ user plugin location.

use super::{DriverStatus, InstallOutcome};

pub(super) fn driver_status() -> DriverStatus {
    DriverStatus::Ready
}

pub(super) fn install_driver() -> InstallOutcome {
    InstallOutcome::NotNeeded
}

fn obs_config_root() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        std::path::PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("obs-studio"),
    )
}

pub(super) fn obs_plugin_dir() -> Option<std::path::PathBuf> {
    let root = obs_config_root()?;
    root.exists().then(|| root.join("plugins"))
}

pub(super) fn default_obs_plugin_dir() -> Option<std::path::PathBuf> {
    Some(obs_config_root()?.join("plugins"))
}
