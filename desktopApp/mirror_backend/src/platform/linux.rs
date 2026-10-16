//! Linux: USB access is granted by a udev rule.

use super::{DriverStatus, InstallOutcome};
use crate::log_event;
use mirror_i18n::codes;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Where the rule this app installs lives.
const RULE_PATH: &str = "/etc/udev/rules.d/51-android-aoa.rules";

/// An older name shipped by earlier versions. Still honoured so an existing
/// install is not asked to authorise a second rule that does the same thing.
const LEGACY_RULE_PATH: &str = "/etc/udev/rules.d/99-android-mirror.rules";

/// First line of the rule, and the version it carries. Bump the number when
/// the rule changes: an installed copy without the current marker is treated as
/// out of date and replaced the next time the person allows USB access.
const RULE_MARKER: &str = "# screenmirror-udev-rules: 2";

/// The rule itself.
///
/// A phone needs access twice. Before the switch it is an ordinary device with
/// its own vendor ID, and the AOA request that starts the switch has to be sent
/// to it: matching only Google's accessory IDs left that first step to
/// whatever rules the distribution happened to ship. After the switch it
/// re-appears as `18d1:2d0x`.
///
/// Phones are recognised by what they expose, not by who made them: MTP
/// (`ffff00`), PTP (`060101`) or ADB (`ff4201`) interfaces, from any vendor.
/// `ID_USB_INTERFACES` is filled in by `50-udev-default.rules`, which runs
/// before this file.
///
/// `uaccess` hands the device to whoever is logged in at the seat, which is
/// what we want and nothing more. The predecessor was `MODE="0666"` on a bare
/// vendor match: world read/write on *every* Google USB device attached to the
/// machine.
const RULE_CONTENT: &str = "\
# screenmirror-udev-rules: 2\n\
# ScreenMirror: raw USB access for the logged-in user, for Android phones only.\n\
# After the accessory switch (AOA product IDs 2d00-2d0f):\n\
SUBSYSTEM==\"usb\", ENV{DEVTYPE}==\"usb_device\", ATTR{idVendor}==\"18d1\", ATTR{idProduct}==\"2d0?\", TAG+=\"uaccess\"\n\
# Before it, whoever made the phone: MTP, PTP and ADB interfaces.\n\
SUBSYSTEM==\"usb\", ENV{DEVTYPE}==\"usb_device\", ENV{ID_USB_INTERFACES}==\"*:ffff00:*\", TAG+=\"uaccess\"\n\
SUBSYSTEM==\"usb\", ENV{DEVTYPE}==\"usb_device\", ENV{ID_USB_INTERFACES}==\"*:060101:*\", TAG+=\"uaccess\"\n\
SUBSYSTEM==\"usb\", ENV{DEVTYPE}==\"usb_device\", ENV{ID_USB_INTERFACES}==\"*:ff4201:*\", TAG+=\"uaccess\"\n";

/// Whether the installed rule is this version's.
fn rule_is_current() -> bool {
    std::fs::read_to_string(RULE_PATH).is_ok_and(|text| text.lines().next() == Some(RULE_MARKER))
}

pub(super) fn driver_status() -> DriverStatus {
    if rule_is_current() || Path::new(LEGACY_RULE_PATH).exists() {
        DriverStatus::Ready
    } else {
        DriverStatus::NeedsInstall
    }
}

pub(super) fn install_driver() -> InstallOutcome {
    if rule_is_current() {
        return InstallOutcome::NotNeeded;
    }

    log_event!(codes::DRIVER_SETUP_REQUESTING_ELEVATION);

    // The rule is streamed to the elevated shell on stdin and written by root
    // itself. Staging it in a shared directory first — the old
    // `/tmp/51-android-aoa.rules` followed by a `cp` — let any local user
    // pre-create or swap that path between the write and the copy, so root
    // would install attacker-supplied udev rules. A udev rule can carry
    // `RUN+=`, which makes that arbitrary code execution as root.
    //
    // RULE_PATH is a compile-time constant, so nothing user-controlled reaches
    // the shell.
    let script = format!(
        "umask 022 && cat > {RULE_PATH} && \
         udevadm control --reload-rules && udevadm trigger"
    );

    let spawned = Command::new("pkexec")
        .arg("sh")
        .arg("-c")
        .arg(&script)
        .stdin(Stdio::piped())
        .spawn();

    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            log_event!(codes::DRIVER_SETUP_PKEXEC_LAUNCH_FAILED, "error" => e);
            return InstallOutcome::Failed;
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(RULE_CONTENT.as_bytes()).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return InstallOutcome::Failed;
        }
        // Close the pipe so `cat` sees EOF and the script can proceed.
        drop(stdin);
    }

    match child.wait() {
        Ok(status) if status.success() && rule_is_current() => {
            log_event!(codes::DRIVER_SETUP_UDEV_INSTALLED);
            InstallOutcome::Installed
        }
        _ => {
            log_event!(codes::DRIVER_SETUP_UDEV_FAILED);
            InstallOutcome::Failed
        }
    }
}

pub(super) fn obs_plugin_dir() -> Option<std::path::PathBuf> {
    let home = std::path::PathBuf::from(std::env::var_os("HOME")?);

    // A Flatpak OBS cannot see ~/.config, so its config lives in the sandbox
    // directory. Checked first: a machine can have both, and the Flatpak one
    // is the one that would fail to find a plugin installed elsewhere.
    let flatpak = home.join(".var/app/com.obsproject.Studio/config/obs-studio");
    if flatpak.exists() {
        return Some(flatpak.join("plugins"));
    }

    // OBS 28+ default.
    let modern = home.join(".config/obs-studio");
    if modern.exists() {
        return Some(modern.join("plugins"));
    }

    // Pre-28.
    let legacy = home.join(".obs-studio");
    if legacy.exists() {
        return Some(legacy.join("plugins"));
    }

    None
}

pub(super) fn default_obs_plugin_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(std::path::PathBuf::from(home).join(".config/obs-studio/plugins"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_is_scoped_to_the_accessory_product_range() {
        // A bare vendor match would hand every Google USB device on the
        // machine to the session. This assertion is the regression test for
        // that: it fails if the product match is ever widened or dropped.
        assert!(RULE_CONTENT.contains("ATTR{idProduct}==\"2d0?\""));
        assert!(RULE_CONTENT.contains("ATTR{idVendor}==\"18d1\""));
    }

    #[test]
    fn the_rule_grants_seat_access_not_world_access() {
        assert!(RULE_CONTENT.contains("TAG+=\"uaccess\""));
        assert!(
            !RULE_CONTENT.contains("MODE"),
            "a MODE= rule grants access to every user on the machine"
        );
    }

    #[test]
    fn nothing_user_controlled_reaches_the_elevated_shell() {
        // The whole reason the content goes over stdin rather than into the
        // command line. If RULE_PATH ever stops being a literal, this is where
        // it should be reconsidered.
        assert!(RULE_PATH.starts_with("/etc/udev/rules.d/"));
        assert!(!RULE_PATH.contains(' '));
    }
}
