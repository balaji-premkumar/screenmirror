//! Building the `wdi-simple` command that installs WinUSB for Android devices.
//!
//! Windows needs a driver bound to a device before libusb can open it. That
//! applies to the phone twice: once as the plain MTP/ADB device it is when
//! plugged in (so the accessory switch can be requested), and again as the
//! accessory it becomes afterwards. libwdi's `wdi-simple` creates and signs a
//! driver package on the spot, which is what makes this a single UAC prompt
//! instead of a manual Zadig session.
//!
//! The script is assembled here, not in `windows.rs`, so it compiles and is
//! tested on every platform: quoting mistakes in an elevated PowerShell command
//! are exactly the kind of bug that is expensive to find on a Windows machine.

#![cfg_attr(not(windows), allow(dead_code))]

/// The manufacturer name written into every package we install. libwdi still
/// lists itself as the provider, so packages are recognised by [`inf_name`].
pub const PROVIDER: &str = "ScreenMirror";

/// One device to bind WinUSB to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub vid: u16,
    pub pid: u16,
    /// The interface to bind on a composite device. `None` binds the whole
    /// device, which is right for a single-function accessory.
    pub interface: Option<u8>,
    pub name: &'static str,
}

pub const ACCESSORY_NAME: &str = "Android Accessory (ScreenMirror)";
pub const PHONE_NAME: &str = "Android Phone (ScreenMirror)";

/// The INF name a package for `t` is installed under, e.g.
/// `mirror_04e8_6860_mi03.inf`.
///
/// libwdi names every package `usb_device.inf` by default, and the driver
/// store keeps that as the package's original name, so without this nothing
/// told our packages apart from anyone else's and each launch installed them
/// all again. Lower case, because the driver store is searched lower-cased.
pub fn inf_name(t: &Target) -> String {
    let interface = t
        .interface
        .map(|i| format!("_mi{i:02x}"))
        .unwrap_or_default();
    format!("mirror_{:04x}_{:04x}{interface}.inf", t.vid, t.pid)
}

/// Quotes `s` as a single-quoted PowerShell string.
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The commands, one `wdi-simple` call per target, as a single PowerShell line.
pub fn install_commands(wdi: &str, dest: &str, targets: &[Target]) -> String {
    let mut out = String::new();
    for t in targets {
        out.push_str(&format!(
            "& {wdi} --vid 0x{:04X} --pid 0x{:04X}{} --type 0 --name {} --manufacturer {} --inf {} --dest {} ; ",
            t.vid,
            t.pid,
            t.interface
                .map(|i| format!(" --iid {i}"))
                .unwrap_or_default(),
            ps_quote(t.name),
            ps_quote(PROVIDER),
            ps_quote(&inf_name(t)),
            ps_quote(dest),
            wdi = ps_quote(wdi),
        ));
    }
    out
}

/// Wraps `commands` so Windows raises one UAC prompt and waits for it to finish.
///
/// The commands travel inside a double-quoted PowerShell string, where `$` and
/// `"` are special, so both are escaped. Single quotes are already doubled by
/// [`ps_quote`].
pub fn elevated(commands: &str) -> String {
    let escaped = commands.replace('"', "`\"").replace('$', "`$");
    format!(
        "Start-Process powershell -Verb RunAs -Wait -WindowStyle Hidden \
         -ArgumentList '-NoProfile','-Command',\"{escaped}\""
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accessory(pid: u16) -> Target {
        Target {
            vid: 0x18D1,
            pid,
            interface: None,
            name: ACCESSORY_NAME,
        }
    }

    #[test]
    fn a_whole_device_target_has_no_interface_flag() {
        let s = install_commands(
            "C:\\app\\wdi-simple.exe",
            "C:\\Temp\\w",
            &[accessory(0x2D01)],
        );
        assert!(s.contains("--vid 0x18D1 --pid 0x2D01 --type 0"));
        assert!(!s.contains("--iid"));
        assert!(s.contains("--manufacturer 'ScreenMirror'"));
    }

    #[test]
    fn a_composite_phone_binds_one_interface() {
        let phone = Target {
            vid: 0x04E8,
            pid: 0x6860,
            interface: Some(0),
            name: PHONE_NAME,
        };
        let s = install_commands("w.exe", "d", &[phone]);
        assert!(s.contains("--vid 0x04E8 --pid 0x6860 --iid 0 --type 0"));
        assert!(s.contains("--inf 'mirror_04e8_6860_mi00.inf'"));
    }

    #[test]
    fn a_whole_device_package_is_named_by_its_ids() {
        assert_eq!(inf_name(&accessory(0x2D01)), "mirror_18d1_2d01.inf");
    }

    #[test]
    fn every_target_gets_its_own_call() {
        let s = install_commands("w.exe", "d", &[accessory(0x2D00), accessory(0x2D01)]);
        assert_eq!(s.matches("& 'w.exe'").count(), 2);
    }

    #[test]
    fn a_quote_in_a_path_cannot_end_the_string() {
        // A user named O'Brien has a profile path with a single quote in it.
        let s = install_commands("C:\\Users\\O'Brien\\wdi.exe", "d", &[accessory(0x2D00)]);
        assert!(s.contains("'C:\\Users\\O''Brien\\wdi.exe'"));
    }

    #[test]
    fn the_elevated_wrapper_survives_dollars_and_double_quotes() {
        let s = elevated("& 'a$b' --x \"y\" ;");
        assert!(s.contains("a`$b"));
        assert!(s.contains("`\"y`\""));
        assert!(s.starts_with("Start-Process powershell -Verb RunAs -Wait"));
    }
}
