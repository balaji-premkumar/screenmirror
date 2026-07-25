//! Windows USB driver automation.
//!
//! libusb (rusb) on Windows can only talk to a device whose interface is
//! bound to WinUSB (or libusbK). Android phones in AOA accessory mode
//! (VID 18D1, PID 2D00–2D05) do not carry Microsoft OS descriptors, so
//! Windows does not bind WinUSB automatically — we have to install a
//! driver package once.
//!
//! Strategy, in order of preference:
//!   1. If the accessory already opens through libusb → driver is fine.
//!   2. If a bundled `bin\wdi-simple.exe` (from the libwdi project) exists,
//!      run it elevated for every attached Android device. libwdi generates
//!      a self-signed driver package on the fly and installs it — the same
//!      mechanism Zadig uses, fully unattended.
//!   3. Otherwise generate a WinUSB "primitive driver" INF covering the AOA
//!      PIDs and install it elevated via `pnputil /add-driver`. On systems
//!      that enforce driver signature verification this requires the INF to
//!      be signed, so we log clear guidance when it fails.
//!
//! All elevation is done with a single PowerShell `Start-Process -Verb RunAs`
//! so the user sees exactly one UAC prompt.

#![cfg(target_os = "windows")]

use crate::receiver::log_event;
use rusb::UsbContext;
use std::io::Write;
use std::process::Command;

const GOOGLE_VID: u16 = 0x18D1;
const AOA_PID_RANGE: std::ops::RangeInclusive<u16> = 0x2D00..=0x2D05;

/// 1 = an AOA accessory is claimable through WinUSB (or none is attached
/// and a previously installed driver package is present), 0 = action needed.
pub fn check_driver_status() -> i32 {
    let ctx = match rusb::Context::new() {
        Ok(c) => c,
        Err(_) => return 0,
    };

    let mut accessory_attached = false;

    if let Ok(devices) = ctx.devices() {
        for device in devices.iter() {
            let Ok(desc) = device.device_descriptor() else {
                continue;
            };
            if desc.vendor_id() == GOOGLE_VID && AOA_PID_RANGE.contains(&desc.product_id()) {
                accessory_attached = true;
                // Opening succeeds only when WinUSB is bound to the interface.
                if device.open().is_ok() {
                    return 1;
                }
            }
        }
    }

    if accessory_attached {
        // Accessory present but not claimable → driver missing.
        return 0;
    }

    // No accessory attached right now — report OK if our driver package was
    // installed previously so the UI doesn't nag on every launch.
    if driver_package_installed() {
        1
    } else {
        0
    }
}

static DRIVER_PACKAGE_CACHE: std::sync::Mutex<Option<(bool, std::time::Instant)>> =
    std::sync::Mutex::new(None);

/// Whether our INF is registered in the driver store.
///
/// `pnputil /enum-drivers` walks the whole driver store and takes hundreds of
/// milliseconds. `check_driver_status()` is called from the UI status tick
/// (twice a second), and with no accessory attached every one of those calls
/// used to reach this function — so the app spawned a driver-store enumeration
/// twice a second for as long as it was open. Driver installs are rare, so the
/// answer is cached for a minute; `install_driver()` invalidates it on success.
fn driver_package_installed() -> bool {
    const TTL: std::time::Duration = std::time::Duration::from_secs(60);

    let mut cache = DRIVER_PACKAGE_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some((value, when)) = *cache {
        if when.elapsed() < TTL {
            return value;
        }
    }

    // pnputil lists OEM packages by original INF name.
    let value = Command::new("pnputil")
        .args(["/enum-drivers"])
        .output()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .to_lowercase()
                .contains("mirror_aoa.inf")
        })
        .unwrap_or(false);

    *cache = Some((value, std::time::Instant::now()));
    value
}

/// Drop the cached answer so a freshly installed package is seen immediately
/// instead of up to a minute later.
fn invalidate_driver_cache() {
    if let Ok(mut cache) = DRIVER_PACKAGE_CACHE.lock() {
        *cache = None;
    }
}

/// Entry point called at app startup (and from the "Fix USB Permissions"
/// button). Returns 1 on success, 0 when nothing had to be done, -1 on error.
pub fn install_driver() -> i32 {
    if check_driver_status() == 1 {
        log_event(
            "INFO",
            "DRIVER",
            "setup",
            "WinUSB driver already functional — no installation needed.",
        );
        return 0;
    }

    // Preferred path: bundled libwdi installer (handles signing on the fly).
    if let Some(wdi) = find_wdi_simple() {
        return install_with_wdi(&wdi);
    }

    install_with_inf()
}

fn find_wdi_simple() -> Option<std::path::PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    for candidate in [
        exe_dir.join("bin").join("wdi-simple.exe"),
        exe_dir.join("wdi-simple.exe"),
    ] {
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn install_with_wdi(wdi: &std::path::Path) -> i32 {
    log_event(
        "INFO",
        "DRIVER",
        "setup",
        "Installing WinUSB via bundled libwdi (single UAC prompt)...",
    );

    // Install for the whole AOA PID range plus any currently attached
    // Android vendor device, so the phone works both before and after it
    // switches to accessory mode.
    let mut targets: Vec<(u16, u16)> = AOA_PID_RANGE.map(|pid| (GOOGLE_VID, pid)).collect();
    if let Ok(ctx) = rusb::Context::new() {
        if let Ok(devices) = ctx.devices() {
            for device in devices.iter() {
                if let Ok(desc) = device.device_descriptor() {
                    let pair = (desc.vendor_id(), desc.product_id());
                    if desc.vendor_id() == GOOGLE_VID && !targets.contains(&pair) {
                        targets.push(pair);
                    }
                }
            }
        }
    }

    // Chain all wdi-simple invocations into one elevated PowerShell command.
    let mut inner = String::new();
    for (vid, pid) in &targets {
        inner.push_str(&format!(
            "& '{}' --vid 0x{vid:04X} --pid 0x{pid:04X} --type 0 --name 'Android Accessory (ScreenMirror)' --dest '{}\\mirror_wdi' ; ",
            wdi.display(),
            std::env::temp_dir().display(),
        ));
    }

    let status = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "Start-Process powershell -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile','-Command',\"{}\"",
                inner.replace('"', "`\"")
            ),
        ])
        .status();

    match status {
        Ok(s) if s.success() => {
            invalidate_driver_cache();
            log_event("SUCCESS", "DRIVER", "setup", "libwdi driver installation finished.");
            1
        }
        _ => {
            log_event(
                "ERROR",
                "DRIVER",
                "setup",
                "libwdi installation failed or was cancelled at the UAC prompt.",
            );
            -1
        }
    }
}

/// WinUSB primitive-driver INF covering the AOA accessory PIDs (bare device
/// and MI_00 composite interface variants).
const INF_TEMPLATE: &str = r#"; mirror_aoa.inf — WinUSB binding for Android Open Accessory devices
; Installed by ScreenMirror. Uses the inbox winusb.sys — no third-party code.

[Version]
Signature   = "$Windows NT$"
Class       = USBDevice
ClassGuid   = {88BAE032-5A81-49f6-BB50-A46C9BE72FA4}
Provider    = %ProviderName%
DriverVer   = 01/01/2025,1.0.0.0
CatalogFile = mirror_aoa.cat

[Manufacturer]
%ProviderName% = Devices,NTamd64

[Devices.NTamd64]
%AccessoryName% = USB_Install, USB\VID_18D1&PID_2D00
%AccessoryName% = USB_Install, USB\VID_18D1&PID_2D01
%AccessoryName% = USB_Install, USB\VID_18D1&PID_2D01&MI_00
%AccessoryName% = USB_Install, USB\VID_18D1&PID_2D02
%AccessoryName% = USB_Install, USB\VID_18D1&PID_2D03
%AccessoryName% = USB_Install, USB\VID_18D1&PID_2D04
%AccessoryName% = USB_Install, USB\VID_18D1&PID_2D04&MI_00
%AccessoryName% = USB_Install, USB\VID_18D1&PID_2D05
%AccessoryName% = USB_Install, USB\VID_18D1&PID_2D05&MI_00

[USB_Install]
Include = winusb.inf
Needs   = WINUSB.NT

[USB_Install.Services]
Include = winusb.inf
Needs   = WINUSB.NT.Services

[USB_Install.HW]
AddReg = Dev_AddReg

[Dev_AddReg]
HKR,,DeviceInterfaceGUIDs,0x10000,"{F72FE0D4-CBCB-407D-8814-9ED673D0DD6B}"

[Strings]
ProviderName  = "ScreenMirror"
AccessoryName = "Android Accessory Interface (WinUSB)"
"#;

fn install_with_inf() -> i32 {
    log_event(
        "INFO",
        "DRIVER",
        "setup",
        "Installing WinUSB INF for AOA devices via pnputil (single UAC prompt)...",
    );

    let inf_dir = std::env::temp_dir().join("mirror_driver");
    if std::fs::create_dir_all(&inf_dir).is_err() {
        return -1;
    }
    let inf_path = inf_dir.join("mirror_aoa.inf");

    match std::fs::File::create(&inf_path) {
        Ok(mut f) => {
            if f.write_all(INF_TEMPLATE.as_bytes()).is_err() {
                return -1;
            }
        }
        Err(_) => return -1,
    }

    let status = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "Start-Process pnputil -Verb RunAs -Wait -ArgumentList '/add-driver','{}','/install'",
                inf_path.display()
            ),
        ])
        .status();

    match status {
        Ok(s) if s.success() => {
            invalidate_driver_cache();
            if driver_package_installed() {
                log_event(
                    "SUCCESS",
                    "DRIVER",
                    "setup",
                    "WinUSB INF installed. Replug the USB cable to activate it.",
                );
                1
            } else {
                log_event(
                    "ERROR",
                    "DRIVER",
                    "setup",
                    "pnputil did not register the package. Unsigned INF packages are \
                     rejected when driver signature enforcement is active — bundle \
                     bin\\wdi-simple.exe (libwdi) for fully automatic installation, \
                     or install the driver once with Zadig.",
                );
                -1
            }
        }
        _ => {
            log_event(
                "ERROR",
                "DRIVER",
                "setup",
                "Driver installation was cancelled or pnputil failed.",
            );
            -1
        }
    }
}
