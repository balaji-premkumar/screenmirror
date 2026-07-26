//! Everything that differs between operating systems, behind one interface.
//!
//! Talking to a USB accessory needs the OS to grant access first, and every
//! platform does it differently: Linux writes a udev rule, Windows binds a
//! WinUSB driver, macOS needs neither. That difference used to be expressed as
//! `#[cfg]` blocks scattered through `lib.rs`, so adding a platform meant
//! editing whichever functions happened to have grown one.
//!
//! Now each OS gets a file that provides three functions, and this module
//! picks one at compile time. Supporting a new platform is: add the file, add
//! the `cfg` line below, implement three functions. The compiler tells you if
//! you missed one.
//!
//! | Function                | Linux           | Windows            | macOS |
//! |-------------------------|-----------------|--------------------|-------|
//! | [`driver_status`]       | udev rule file  | probe via libusb   | always ready |
//! | [`install_driver`]      | pkexec + udev   | libwdi or pnputil  | no-op |
//! | [`obs_plugin_dir`]      | `~/.config/obs-studio` | `%APPDATA%` | `~/Library` |

#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(target_os = "windows", path = "windows.rs")]
#[cfg_attr(target_os = "macos", path = "macos.rs")]
#[cfg_attr(
    not(any(target_os = "linux", target_os = "windows", target_os = "macos")),
    path = "unsupported.rs"
)]
mod imp;

/// Whether USB access has been granted on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverStatus {
    /// The accessory can be opened; nothing to install.
    Ready,
    /// The user must authorise an installation before streaming can work.
    NeedsInstall,
}

impl DriverStatus {
    /// The C ABI representation the interface polls: 1 ready, 0 not.
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        match self {
            DriverStatus::Ready => 1,
            DriverStatus::NeedsInstall => 0,
        }
    }
}

/// What an install attempt did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallOutcome {
    /// Something was installed and USB access should now work.
    Installed,
    /// Nothing needed doing, or this platform requires no driver at all.
    NotNeeded,
    /// The attempt failed, or the user dismissed the elevation prompt.
    Failed,
}

impl InstallOutcome {
    /// The C ABI representation: 1 installed, 0 not needed, -1 failed.
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        match self {
            InstallOutcome::Installed => 1,
            InstallOutcome::NotNeeded => 0,
            InstallOutcome::Failed => -1,
        }
    }
}

/// Can this machine already open the accessory?
///
/// Called on every status poll, so it must stay cheap — a file check or a
/// cached probe, never an elevation prompt.
#[must_use]
pub fn driver_status() -> DriverStatus {
    imp::driver_status()
}

/// Grants USB access, prompting for elevation if the platform needs it.
///
/// Call only when [`driver_status`] reports [`DriverStatus::NeedsInstall`]:
/// every implementation that does something raises a system prompt, and
/// re-running it on each launch nags the user for nothing.
pub fn install_driver() -> InstallOutcome {
    imp::install_driver()
}

/// Where OBS Studio loads user plugins from, if its config directory exists.
///
/// `None` means OBS has never been run on this machine, or this platform has
/// no implementation. Use [`default_obs_plugin_dir`] to install anyway.
#[must_use]
pub fn obs_plugin_dir() -> Option<std::path::PathBuf> {
    imp::obs_plugin_dir()
}

/// Where OBS Studio *would* load user plugins from, whether or not it has ever
/// been run.
///
/// Installing here is correct when OBS is present but has not yet created its
/// config directory: it reads the directory at startup, so a plugin placed
/// there in advance is picked up on first launch.
#[must_use]
pub fn default_obs_plugin_dir() -> Option<std::path::PathBuf> {
    imp::default_obs_plugin_dir()
}

/// The file name a compiled OBS plugin has on this platform.
///
/// macOS uses `.so` rather than `.dylib`: libobs loads plugins with `dlopen`
/// and looks for that suffix, which is what the OBS plugin template produces
/// there too.
#[must_use]
pub const fn obs_plugin_filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "mirror-source.dll"
    } else {
        "mirror-source.so"
    }
}

/// The executable name of ffplay on this platform.
#[must_use]
pub const fn ffplay_filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "ffplay.exe"
    } else {
        "ffplay"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_abi_values_are_what_the_interface_expects() {
        // src/bun/index.ts checks `=== 1` for readiness and treats a negative
        // return from an install as failure. These three values are that
        // contract.
        assert_eq!(DriverStatus::Ready.as_i32(), 1);
        assert_eq!(DriverStatus::NeedsInstall.as_i32(), 0);
        assert_eq!(InstallOutcome::Installed.as_i32(), 1);
        assert_eq!(InstallOutcome::NotNeeded.as_i32(), 0);
        assert_eq!(InstallOutcome::Failed.as_i32(), -1);
    }

    #[test]
    fn platform_filenames_match_the_host() {
        if cfg!(target_os = "windows") {
            assert_eq!(ffplay_filename(), "ffplay.exe");
            assert_eq!(obs_plugin_filename(), "mirror-source.dll");
        } else {
            assert_eq!(ffplay_filename(), "ffplay");
            assert_eq!(obs_plugin_filename(), "mirror-source.so");
        }
    }

    #[test]
    fn driver_status_never_prompts_and_always_answers() {
        // Cheap enough to call on a status poll, which is what the UI does
        // twice a second.
        let _ = driver_status();
    }
}
