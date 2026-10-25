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
//! | [`obs_plugin_dir`]      | `~/.config/obs-studio` | `%ProgramData%` | `~/Library` |

#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(target_os = "windows", path = "windows.rs")]
#[cfg_attr(target_os = "macos", path = "macos.rs")]
#[cfg_attr(
    not(any(target_os = "linux", target_os = "windows", target_os = "macos")),
    path = "unsupported.rs"
)]
mod imp;

/// Whether USB access has been granted on this machine.
pub(crate) mod wdi;

/// Where a phone exposes the interface the app talks to before the accessory
/// switch: ADB, or the vendor-specific interface that carries MTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VendorInterface {
    /// The interface number, which is what a Windows driver binds to on a
    /// composite device.
    pub number: u8,
    /// How many interfaces the device has; more than one means composite.
    pub interface_count: usize,
}

/// USB vendor IDs of Android phone makers.
///
/// A vendor-specific interface alone says nothing about what a device is:
/// fingerprint readers, webcams and RGB controllers have them too, and binding
/// WinUSB to one of those breaks it. So a device without an ADB interface is
/// only treated as a phone when its vendor makes phones.
const ANDROID_VENDOR_IDS: &[u16] = &[
    0x0489, // Foxconn (Nokia, Sharp, InFocus)
    0x04DD, // Sharp
    0x04E8, // Samsung
    0x0502, // Acer
    0x05C6, // Qualcomm reference devices
    0x0B05, // Asus
    0x0BB4, // HTC
    0x0E8D, // MediaTek reference devices
    0x0FCE, // Sony Mobile
    0x1004, // LG
    0x109B, // Hisense
    0x12D1, // Huawei, Honor
    0x17EF, // Lenovo
    0x18D1, // Google (Pixel, Nexus, AOSP)
    0x1949, // Amazon
    0x19D2, // ZTE, Nubia
    0x1BBB, // TCL, Alcatel
    0x22B8, // Motorola
    0x22D9, // OPPO, Realme
    0x2717, // Xiaomi, Redmi, POCO
    0x2A45, // Meizu
    0x2A70, // OnePlus
    0x2AE5, // Fairphone
    0x2D95, // vivo, iQOO
    0x2E04, // HMD (Nokia)
];

/// ADB's interface triple: vendor class, subclass 0x42, protocol 1. Nothing
/// but Android uses it.
const ADB_INTERFACE: (u8, u8, u8) = (0xFF, 0x42, 0x01);

/// The interface of `device` the app should use before the accessory switch,
/// if `device` is an Android phone.
///
/// ADB is preferred: it marks the device as Android on its own, and binding it
/// leaves MTP, and with it file access from Explorer, untouched. Without ADB,
/// the first vendor-specific interface is used, but only on a device from an
/// [`ANDROID_VENDOR_IDS`] vendor.
///
/// This is the app's definition of "a phone that has not switched to accessory
/// mode yet". Discovery and the Windows driver step share it so they cannot
/// disagree about which devices are phones, and anything else on the bus is
/// left alone.
pub(crate) fn android_interface<T: rusb::UsbContext>(
    device: &rusb::Device<T>,
) -> Option<VendorInterface> {
    let config = device.active_config_descriptor().ok()?;
    let interface_count = config.interfaces().count();
    let at = |number| VendorInterface {
        number,
        interface_count,
    };

    let mut vendor_specific = None;
    for interface in config.interfaces() {
        for alt in interface.descriptors() {
            let triple = (alt.class_code(), alt.sub_class_code(), alt.protocol_code());
            if triple == ADB_INTERFACE {
                return Some(at(interface.number()));
            }
            if alt.class_code() == 0xFF && vendor_specific.is_none() {
                vendor_specific = Some(interface.number());
            }
        }
    }

    let vid = device.device_descriptor().ok()?.vendor_id();
    if ANDROID_VENDOR_IDS.contains(&vid) {
        vendor_specific.map(at)
    } else {
        None
    }
}

/// Whether a phone with USB debugging switched on is plugged in.
///
/// The app does not need ADB or developer options. With them on, the phone
/// exposes an ADB interface that Android's own ADB drivers (Google's, or the
/// vendor's) also want, and the accessory comes back as 2D01 (accessory +
/// ADB) instead of 2D00, which is where drivers end up mismatched. The
/// interface tells the person to switch it off.
///
/// Walks the bus, so the answer is cached for a couple of seconds: it is
/// polled from the status tick.
pub fn adb_enabled() -> bool {
    const TTL: std::time::Duration = std::time::Duration::from_secs(2);
    static CACHE: std::sync::Mutex<Option<(bool, std::time::Instant)>> =
        std::sync::Mutex::new(None);

    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((value, when)) = *cache {
        if when.elapsed() < TTL {
            return value;
        }
    }
    let value = rusb::devices()
        .map(|devices| devices.iter().any(|d| has_adb(&d)))
        .unwrap_or(false);
    *cache = Some((value, std::time::Instant::now()));
    value
}

fn has_adb<T: rusb::UsbContext>(device: &rusb::Device<T>) -> bool {
    if let Ok(desc) = device.device_descriptor() {
        if desc.vendor_id() == 0x18D1 && desc.product_id() == 0x2D01 {
            return true;
        }
    }
    let Ok(config) = device.active_config_descriptor() else {
        return false;
    };
    config.interfaces().any(|interface| {
        interface.descriptors().any(|alt| {
            (alt.class_code(), alt.sub_class_code(), alt.protocol_code()) == ADB_INTERFACE
        })
    })
}

/// Whether `device` is an Android phone not yet in accessory mode.
pub(crate) fn is_android_candidate<T: rusb::UsbContext>(device: &rusb::Device<T>) -> bool {
    android_interface(device).is_some()
}

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

/// [`install_state`] values. The finished states mirror [`InstallOutcome`].
pub const INSTALL_IDLE: i32 = 0;
pub const INSTALL_RUNNING: i32 = 1;
pub const INSTALL_INSTALLED: i32 = 2;
pub const INSTALL_NOT_NEEDED: i32 = 3;
pub const INSTALL_FAILED: i32 = -1;

static INSTALL_STATE: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(INSTALL_IDLE);

/// Runs [`install_driver`] on a background thread and returns at once.
///
/// The install waits on an elevation prompt and then on Windows' driver
/// installer, which together take seconds. Run on the caller's thread, that
/// froze Bun's event loop, so the interface could not show that anything was
/// happening. Poll [`install_state`] for the result instead.
///
/// Returns `false` when an install is already running.
pub fn start_install_driver() -> bool {
    use std::sync::atomic::Ordering;

    if INSTALL_STATE.swap(INSTALL_RUNNING, Ordering::AcqRel) == INSTALL_RUNNING {
        return false;
    }
    std::thread::spawn(|| {
        let state = match install_driver() {
            InstallOutcome::Installed => INSTALL_INSTALLED,
            InstallOutcome::NotNeeded => INSTALL_NOT_NEEDED,
            InstallOutcome::Failed => INSTALL_FAILED,
        };
        INSTALL_STATE.store(state, Ordering::Release);
    });
    true
}

/// Where the install started by [`start_install_driver`] has got to.
#[must_use]
pub fn install_state() -> i32 {
    INSTALL_STATE.load(std::sync::atomic::Ordering::Acquire)
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

/// The plugin's own directory inside an OBS plugin directory.
///
/// OBS Studio's frontend registers `<plugins>/%module%` as a search path on
/// Linux and Windows, and `<plugins>/%module%.plugin` on macOS
/// (`AddExtraModulePaths` in `frontend/widgets/OBSBasic.cpp`).
#[must_use]
pub fn obs_plugin_root(plugin_dir: &std::path::Path) -> std::path::PathBuf {
    if cfg!(target_os = "macos") {
        plugin_dir.join("mirror-source.plugin")
    } else {
        plugin_dir.join("mirror-source")
    }
}

/// Where the plugin binary goes inside [`obs_plugin_root`].
///
/// Linux and Windows load `bin/64bit/<module><ext>`. macOS loads a bundle's
/// `Contents/MacOS/<module>`, and libobs looks for it there with no extension,
/// so the `.so` that [`obs_plugin_filename`] names is installed without one.
#[must_use]
pub fn obs_plugin_binary(plugin_dir: &std::path::Path) -> std::path::PathBuf {
    let root = obs_plugin_root(plugin_dir);
    if cfg!(target_os = "macos") {
        root.join("Contents/MacOS/mirror-source")
    } else {
        root.join("bin/64bit").join(obs_plugin_filename())
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
    fn only_phone_makers_are_on_the_android_vendor_list() {
        // ELAN makes the fingerprint readers that a vendor-specific interface
        // alone used to mistake for phones.
        assert!(!ANDROID_VENDOR_IDS.contains(&0x04F3));
        assert!(ANDROID_VENDOR_IDS.contains(&0x04E8));
        assert!(
            ANDROID_VENDOR_IDS.windows(2).all(|w| w[0] < w[1]),
            "keep the list sorted"
        );
    }

    #[test]
    fn driver_status_never_prompts_and_always_answers() {
        // Cheap enough to call on a status poll, which is what the UI does
        // twice a second.
        let _ = driver_status();
    }
}
