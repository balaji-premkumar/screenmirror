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
//!   2. If a bundled `wdi-simple.exe` (from the libwdi project) sits next to
//!      this DLL, run it elevated for whatever is still missing: the accessory
//!      IDs, and attached Android phones that do not open yet. Nothing that is
//!      not an Android phone is ever bound. libwdi generates
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
use super::{android_interface, DriverStatus, InstallOutcome};
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

/// How long a status answer is reused. Polling asks every half second and each
/// answer walks the USB bus, which is slow on Windows.
const STATUS_TTL: std::time::Duration = std::time::Duration::from_secs(2);

static STATUS_CACHE: std::sync::Mutex<Option<(i32, std::time::Instant)>> =
    std::sync::Mutex::new(None);

/// 1 when USB access is in place, 0 when an install is needed.
///
/// In place means two things. Every Android phone that is plugged in opens
/// through libusb, because one with no WinUSB binding is invisible to it for
/// anything beyond listing. And the accessory the phone turns into has a driver
/// waiting, because once the phone has switched it is too late to install one.
///
/// Only Android devices are looked at: see [`super::android_interface`].
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
    let mut accessory_opens = false;

    if let Ok(ctx) = rusb::Context::new() {
        if let Ok(devices) = ctx.devices() {
            for device in devices.iter() {
                let Ok(desc) = device.device_descriptor() else {
                    continue;
                };
                let is_accessory = is_accessory(desc.vendor_id(), desc.product_id());
                if !is_accessory && android_interface(&device).is_none() {
                    continue;
                }
                if device.open().is_err() {
                    return 0;
                }
                accessory_opens |= is_accessory;
            }
        }
    }

    if accessory_opens || accessory_packages_installed() {
        1
    } else {
        0
    }
}

fn is_accessory(vid: u16, pid: u16) -> bool {
    vid == GOOGLE_VID && AOA_PID_RANGE.contains(&pid)
}

static DRIVER_STORE_CACHE: std::sync::Mutex<Option<(String, std::time::Instant)>> =
    std::sync::Mutex::new(None);

/// `pnputil /enum-drivers`, lower-cased.
///
/// It walks the whole driver store and takes hundreds of milliseconds.
/// `check_driver_status()` is called from the UI status tick (twice a second),
/// so the answer is cached for a minute; `invalidate_driver_cache()` drops it
/// once an install has changed the store.
fn driver_store() -> String {
    const TTL: std::time::Duration = std::time::Duration::from_secs(60);

    let mut cache = DRIVER_STORE_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some((text, when)) = cache.as_ref() {
        if when.elapsed() < TTL {
            return text.clone();
        }
    }

    let text = Command::new("pnputil")
        .args(["/enum-drivers"])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).to_lowercase())
        .unwrap_or_default();
    *cache = Some((text.clone(), std::time::Instant::now()));
    text
}

/// Whether the driver store holds a WinUSB package for `target`.
///
/// Packages this app installs are named by [`wdi::inf_name`]. Ones libwdi
/// made under any other name (an earlier version of this app, or Zadig) are
/// recognised by the hardware ID libwdi writes into the signer line. The INF
/// fallback covers every accessory ID in one package.
fn package_installed(store: &str, target: &Target) -> bool {
    let hardware_id = match target.interface {
        Some(i) => format!("vid_{:04x}&pid_{:04x}&mi_{i:02x}", target.vid, target.pid),
        None => format!("vid_{:04x}&pid_{:04x}", target.vid, target.pid),
    };
    store.contains(&wdi::inf_name(target))
        || store.contains(&format!("usb\\{hardware_id} (libwdi"))
        || (is_accessory(target.vid, target.pid) && store.contains("mirror_aoa.inf"))
}

/// Whether every accessory ID the phone can switch to has a driver waiting.
fn accessory_packages_installed() -> bool {
    let store = driver_store();
    accessory_targets().all(|t| package_installed(&store, &t))
}

/// Drop the cached answers so a freshly installed package is seen immediately
/// instead of up to a minute later.
fn invalidate_driver_cache() {
    if let Ok(mut cache) = DRIVER_STORE_CACHE.lock() {
        *cache = None;
    }
    if let Ok(mut cache) = STATUS_CACHE.lock() {
        *cache = None;
    }
}

/// Returns 1 on success, 0 when nothing had to be done, -1 on error.
fn install_driver_raw() -> i32 {
    invalidate_driver_cache();
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
    // The bundler puts wdi-simple.exe beside this DLL (`Resources\app\bin`).
    // The process executable is Electrobun's `<app>\bin\bun.exe`, so looking
    // only next to it never found the bundled copy and every install fell
    // through to the unsigned INF, which Windows rejects.
    let mut candidates: Vec<std::path::PathBuf> = backend_dir()
        .map(|dir| dir.join("wdi-simple.exe"))
        .into_iter()
        .collect();
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
    {
        candidates.push(exe_dir.join("bin").join("wdi-simple.exe"));
        candidates.push(exe_dir.join("wdi-simple.exe"));
    }
    candidates.into_iter().find(|c| c.exists())
}

/// The directory `mirror_backend.dll` was loaded from.
fn backend_dir() -> Option<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleExW(flags: u32, address: *const u16, module: *mut *mut libc::c_void) -> i32;
        fn GetModuleFileNameW(module: *mut libc::c_void, buf: *mut u16, size: u32) -> u32;
    }
    const FROM_ADDRESS: u32 = 0x4;
    const UNCHANGED_REFCOUNT: u32 = 0x2;

    let mut module = std::ptr::null_mut();
    // SAFETY: the address is a function in this module and `module` is a
    // valid out-pointer; with UNCHANGED_REFCOUNT no handle needs releasing.
    let found = unsafe {
        GetModuleHandleExW(
            FROM_ADDRESS | UNCHANGED_REFCOUNT,
            backend_dir as *const u16,
            &mut module,
        )
    };
    if found == 0 {
        return None;
    }

    let mut buf = vec![0u16; 32_768];
    // SAFETY: `buf` is writable for the length passed.
    let len = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if len == 0 || len >= buf.len() {
        return None;
    }
    let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(&buf[..len]));
    path.parent().map(std::path::Path::to_path_buf)
}

/// The accessory IDs the phone can come back as after the switch.
///
/// The handshake never asks for audio, so the phone always reappears as
/// 2D00 (accessory) or 2D01 (accessory + ADB). The audio IDs 2D02–2D05 are
/// still recognised when present but no package is installed for them.
fn accessory_targets() -> impl Iterator<Item = Target> {
    [0x2D00, 0x2D01].into_iter().map(|pid| Target {
        vid: GOOGLE_VID,
        pid,
        interface: None,
        name: wdi::ACCESSORY_NAME,
    })
}

/// What WinUSB still has to be bound to, and nothing else.
///
/// Accessory IDs whose package is already in the driver store are skipped,
/// and so are phones that already open, whatever driver made that possible.
/// Devices that are not Android phones are never considered, so nothing else
/// on the bus has its driver replaced. Each package costs several seconds to
/// generate, sign and install, so the list is usually empty or one long.
fn install_targets() -> Vec<Target> {
    let store = driver_store();
    let mut targets: Vec<Target> = accessory_targets()
        .filter(|t| !package_installed(&store, t))
        .collect();

    if let Ok(ctx) = rusb::Context::new() {
        if let Ok(devices) = ctx.devices() {
            for device in devices.iter() {
                let Ok(desc) = device.device_descriptor() else {
                    continue;
                };
                if is_accessory(desc.vendor_id(), desc.product_id()) {
                    continue;
                }
                let Some(found) = android_interface(&device) else {
                    continue;
                };
                if device.open().is_ok() {
                    continue;
                }
                let target = Target {
                    vid: desc.vendor_id(),
                    pid: desc.product_id(),
                    // A single-function device is bound whole; on a composite
                    // one only the ADB (or MTP) interface, so the phone's other
                    // functions keep their drivers.
                    interface: (found.interface_count > 1).then_some(found.number),
                    name: wdi::PHONE_NAME,
                };
                if !targets.contains(&target) {
                    targets.push(target);
                }
            }
        }
    }
    targets
}

fn install_with_wdi(wdi_exe: &std::path::Path) -> i32 {
    let targets = install_targets();
    if targets.is_empty() {
        // Every package is in place; a phone that still will not open needs
        // replugging, not another install.
        log_event!(codes::DRIVER_SETUP_WINUSB_PRESENT);
        return 0;
    }
    log_event!(codes::DRIVER_SETUP_LIBWDI_INSTALLING);

    let dest = std::env::temp_dir().join("mirror_wdi");
    let commands = wdi::install_commands(
        &wdi_exe.display().to_string(),
        &dest.display().to_string(),
        &targets,
    );

    // Declining the UAC prompt makes Start-Process throw, which fails here.
    let ran = Command::new("powershell")
        .args(["-NoProfile", "-Command", &wdi::elevated(&commands)])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    // The elevated shell reports whether *it* ran, not whether each package
    // took, so read the driver store. Waiting for the phone to re-enumerate
    // and open made a successful install look like a failure whenever that
    // took longer than the wait.
    invalidate_driver_cache();
    let store = driver_store();
    if ran && targets.iter().all(|t| package_installed(&store, t)) {
        log_event!(codes::DRIVER_SETUP_LIBWDI_FINISHED);
        return 1;
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
            if accessory_packages_installed() {
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

/// `%APPDATA%\obs-studio` is where OBS keeps its settings, so it existing means
/// OBS has been run here. Plugins still go to [`default_obs_plugin_dir`].
pub(super) fn obs_plugin_dir() -> Option<std::path::PathBuf> {
    let config = std::path::PathBuf::from(std::env::var_os("APPDATA")?).join("obs-studio");
    config.exists().then(default_obs_plugin_dir).flatten()
}

/// OBS on Windows loads third-party plugins from
/// `%ProgramData%\obs-studio\plugins` only; it never reads `%APPDATA%` for
/// them, so a plugin installed there is silently ignored.
pub(super) fn default_obs_plugin_dir() -> Option<std::path::PathBuf> {
    Some(
        std::path::PathBuf::from(std::env::var_os("ProgramData")?)
            .join("obs-studio")
            .join("plugins"),
    )
}
