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

// No `#![cfg(target_os = "windows")]` here any more: `platform/mod.rs` selects
// this file by `cfg_attr(..., path = ...)`, so it is only ever compiled on
// Windows. Keeping the inner attribute as well made the file compile to
// nothing whenever it *was* included, which is how it went unnoticed that it
// had drifted out of sync with the rest of the crate.

use super::wdi::{self, Target};
use super::{vendor_interface, DriverStatus, InstallOutcome};
use crate::log_event;
use mirror_i18n::codes;
use rusb::UsbContext;
use std::io::Write;
use std::process::Command;

const GOOGLE_VID: u16 = 0x18D1;
const AOA_PID_RANGE: std::ops::RangeInclusive<u16> = 0x2D00..=0x2D05;

/// Whether USB access is already granted.
pub(super) fn driver_status() -> DriverStatus {
    if check_driver_status() == 1 {
        DriverStatus::Ready
    } else {
        DriverStatus::NeedsInstall
    }
}

/// Installs WinUSB, prompting once for elevation.
pub(super) fn install_driver() -> InstallOutcome {
    match install_driver_raw() {
        1 => InstallOutcome::Installed,
        0 => InstallOutcome::NotNeeded,
        _ => InstallOutcome::Failed,
    }
}

/// 1 = an AOA accessory is claimable through WinUSB (or none is attached
/// and a previously installed driver package is present), 0 = action needed.
/// How long a status answer is reused. Polling asks every half second and each
/// answer walks the USB bus, which is slow on Windows.
const STATUS_TTL: std::time::Duration = std::time::Duration::from_secs(2);

static STATUS_CACHE: std::sync::Mutex<Option<(i32, std::time::Instant)>> =
    std::sync::Mutex::new(None);

/// 1 when libusb can open every Android device that is plugged in, or when
/// nothing is plugged in and our driver package is installed; 0 otherwise.
///
/// "Can open" is the real test. A phone with no WinUSB binding is invisible to
/// libusb for any purpose beyond listing it, and that is true of the phone as
/// first plugged in just as much as of the accessory it becomes.
fn check_driver_status() -> i32 {
    if let Ok(cache) = STATUS_CACHE.lock() {
        if let Some((value, when)) = *cache {
            if when.elapsed() < STATUS_TTL {
                return value;
            }
        }
    }
    let value = probe_driver_status();
    if let Ok(mut cache) = STATUS_CACHE.lock() {
        *cache = Some((value, std::time::Instant::now()));
    }
    value
}

fn probe_driver_status() -> i32 {
    let ctx = match rusb::Context::new() {
        Ok(c) => c,
        Err(_) => return 0,
    };

    let mut any_android = false;

    if let Ok(devices) = ctx.devices() {
        for device in devices.iter() {
            let Ok(desc) = device.device_descriptor() else {
                continue;
            };
            let is_accessory =
                desc.vendor_id() == GOOGLE_VID && AOA_PID_RANGE.contains(&desc.product_id());
            if is_accessory || vendor_interface(&device).is_some() {
                any_android = true;
                if device.open().is_err() {
                    return 0;
                }
            }
        }
    }

    if any_android {
        return 1;
    }

    // Nothing to open yet, so fall back to whether we installed anything.
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
            let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
            // Packages made by wdi-simple carry our provider name; the INF
            // fallback is recognised by its file name.
            text.contains(&wdi::PROVIDER.to_lowercase()) || text.contains("mirror_aoa.inf")
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
    if let Ok(mut cache) = STATUS_CACHE.lock() {
        *cache = None;
    }
}

/// Returns 1 on success, 0 when nothing had to be done, -1 on error.
fn install_driver_raw() -> i32 {
    if check_driver_status() == 1 {
        log_event!(codes::DRIVER_SETUP_WINUSB_PRESENT);
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

/// Everything WinUSB should be bound to right now.
///
/// The accessory IDs are always included the first time, because the phone
/// reappears under one of them after the switch and it is too late to install
/// then. Phones that are plugged in are bound as they are, on the interface
/// that carries MTP or ADB.
fn install_targets() -> Vec<Target> {
    let mut targets: Vec<Target> = if driver_package_installed() {
        Vec::new()
    } else {
        AOA_PID_RANGE
            .map(|pid| Target {
                vid: GOOGLE_VID,
                pid,
                interface: None,
                name: wdi::ACCESSORY_NAME,
            })
            .collect()
    };

    if let Ok(ctx) = rusb::Context::new() {
        if let Ok(devices) = ctx.devices() {
            for device in devices.iter() {
                let Ok(desc) = device.device_descriptor() else {
                    continue;
                };
                let is_accessory =
                    desc.vendor_id() == GOOGLE_VID && AOA_PID_RANGE.contains(&desc.product_id());
                if is_accessory {
                    continue;
                }
                if let Some(found) = vendor_interface(&device) {
                    let target = Target {
                        vid: desc.vendor_id(),
                        pid: desc.product_id(),
                        // A single-function device is bound whole; on a
                        // composite one only the interface that carries MTP/ADB,
                        // so the phone's other functions keep their drivers.
                        interface: (found.interface_count > 1).then_some(found.number),
                        name: wdi::PHONE_NAME,
                    };
                    if !targets.contains(&target) {
                        targets.push(target);
                    }
                }
            }
        }
    }
    targets
}

fn install_with_wdi(wdi_exe: &std::path::Path) -> i32 {
    log_event!(codes::DRIVER_SETUP_LIBWDI_INSTALLING);

    let targets = install_targets();
    let dest = std::env::temp_dir().join("mirror_wdi");
    let commands = wdi::install_commands(
        &wdi_exe.display().to_string(),
        &dest.display().to_string(),
        &targets,
    );

    let ran = Command::new("powershell")
        .args(["-NoProfile", "-Command", &wdi::elevated(&commands)])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    // The elevated shell reports whether *it* ran, not whether the driver
    // took, so judge by the same test the dashboard uses. The phone
    // re-enumerates once its driver changes, which takes a moment.
    if ran {
        for _ in 0..16 {
            invalidate_driver_cache();
            if check_driver_status() == 1 {
                log_event!(codes::DRIVER_SETUP_LIBWDI_FINISHED);
                return 1;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }

    log_event!(codes::DRIVER_SETUP_LIBWDI_FAILED);
    -1
}

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
    log_event!(codes::DRIVER_SETUP_PNPUTIL_INSTALLING);

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
                log_event!(codes::DRIVER_SETUP_PNPUTIL_INSTALLED);
                1
            } else {
                log_event!(codes::DRIVER_SETUP_PNPUTIL_UNSIGNED);
                -1
            }
        }
        _ => {
            log_event!(codes::DRIVER_SETUP_PNPUTIL_FAILED);
            -1
        }
    }
}

/// `%APPDATA%\obs-studio` is OBS's config, created on first run. It says OBS
/// is in use; the plugin itself does not go there.
pub(super) fn obs_plugin_dir() -> Option<std::path::PathBuf> {
    let config = std::path::PathBuf::from(std::env::var_os("APPDATA")?).join("obs-studio");
    config.exists().then(default_obs_plugin_dir).flatten()
}

/// OBS 29+ loads third-party plugins from
/// `%ProgramData%\obs-studio\plugins\<name>\bin\64bit`. It never looked in
/// `%APPDATA%`, where this used to install them.
pub(super) fn default_obs_plugin_dir() -> Option<std::path::PathBuf> {
    Some(
        std::path::PathBuf::from(std::env::var_os("ProgramData")?)
            .join("obs-studio")
            .join("plugins"),
    )
}
