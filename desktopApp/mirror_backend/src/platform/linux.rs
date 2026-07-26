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

/// The rule itself.
///
/// `uaccess` hands the device to whoever is logged in at the seat, which is
/// what we want and nothing more. The predecessor was `MODE="0666"` on a bare
/// vendor match: world read/write on *every* Google USB device attached to the
/// machine. The product match `2d0?` covers the AOA range 2D00–2D0F only.
const RULE_CONTENT: &str = "\
# ScreenMirror — Android Open Accessory access for the active session\n\
SUBSYSTEM==\"usb\", ATTR{idVendor}==\"18d1\", ATTR{idProduct}==\"2d0?\", TAG+=\"uaccess\"\n";

pub(super) fn driver_status() -> DriverStatus {
    if Path::new(RULE_PATH).exists() || Path::new(LEGACY_RULE_PATH).exists() {
        DriverStatus::Ready
    } else {
        DriverStatus::NeedsInstall
    }
}

pub(super) fn install_driver() -> InstallOutcome {
    if Path::new(RULE_PATH).exists() {
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
        Ok(status) if status.success() && Path::new(RULE_PATH).exists() => {
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
