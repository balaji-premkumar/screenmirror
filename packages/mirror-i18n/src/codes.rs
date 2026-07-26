//! Every event code the backend can emit.
//!
//! A code is a stable identifier. Its wording lives in the locale catalogs and
//! may change freely; the code may not, because a translator's file and a
//! user's saved log both key off it.
//!
//! The `codes!` macro below declares each constant *and* collects it into
//! [`ALL`], which the catalog completeness test walks. That is the whole point
//! of the macro: without a registry, a code added here but never translated
//! would be found by a user, not by CI.

macro_rules! codes {
    ($( $(#[$meta:meta])* $name:ident = $value:literal; )*) => {
        $( $(#[$meta])* pub const $name: &str = $value; )*

        /// Every code declared in this module, for the catalog coverage test.
        pub const ALL: &[&str] = &[ $($name),* ];
    };
}

codes! {
    // ── AOA handshake ───────────────────────────────────────
    /// Asking the phone which AOA protocol version it speaks.
    AOA_HANDSHAKE_REQUESTING_VERSION = "aoa.handshake.requesting_version";
    /// One handshake attempt failed; more will follow. Params: `attempt`, `error`.
    AOA_HANDSHAKE_ATTEMPT_FAILED = "aoa.handshake.attempt_failed";
    /// The device reported AOA version 0, meaning it will not enter accessory mode.
    AOA_HANDSHAKE_REFUSED = "aoa.handshake.refused";
    /// An identification string was accepted. Params: `index`, `value`.
    AOA_HANDSHAKE_STRING_SET = "aoa.handshake.string_set";
    /// An identification string was rejected. Params: `index`, `error`.
    AOA_HANDSHAKE_STRING_FAILED = "aoa.handshake.string_failed";
    /// Asking the device to re-enumerate as an accessory.
    AOA_HANDSHAKE_SWITCHING = "aoa.handshake.switching";

    // ── USB session ─────────────────────────────────────────
    /// A previous session's guard was dropped and the link state reset.
    USB_STREAMING_SESSION_RESET = "usb.streaming.session_reset";
    /// The session thread has started.
    USB_STREAMING_THREAD_STARTED = "usb.streaming.thread_started";
    /// The accessory could not be opened. Params: `error`.
    USB_STREAMING_OPEN_FAILED = "usb.streaming.open_failed";
    /// Interface 0 could not be claimed, usually because another process holds it. Params: `error`.
    USB_STREAMING_CLAIM_FAILED = "usb.streaming.claim_failed";
    /// The link is up and interface 0 is claimed.
    USB_STREAMING_LINK_ESTABLISHED = "usb.streaming.link_established";
    /// No OUT endpoint was advertised, so the protocol default is assumed.
    USB_STREAMING_DEFAULT_ENDPOINT = "usb.streaming.default_endpoint";
    /// The session thread saw the shutdown signal.
    USB_STREAMING_THREAD_STOPPING = "usb.streaming.thread_stopping";
    /// The user asked to disconnect.
    USB_STREAMING_USER_DISCONNECT = "usb.streaming.user_disconnect";
    /// Writing a configuration message to the phone. Params: `bytes`.
    USB_STREAMING_CONFIG_SENDING = "usb.streaming.config_sending";
    /// The configuration message was written. Params: `bytes`.
    USB_STREAMING_CONFIG_SENT = "usb.streaming.config_sent";
    /// The configuration message could not be written. Params: `error`.
    USB_STREAMING_CONFIG_FAILED = "usb.streaming.config_failed";
    /// Nothing arrived for long enough that the phone is assumed gone.
    USB_STREAMING_INACTIVITY_TIMEOUT = "usb.streaming.inactivity_timeout";
    /// A read failed in a way the session cannot recover from. Params: `error`.
    USB_STREAMING_READ_FAILED = "usb.streaming.read_failed";
    /// The session thread has finished.
    USB_STREAMING_THREAD_ENDED = "usb.streaming.thread_ended";

    // ── Manual handshake driven from the UI ─────────────────
    /// A handshake was requested for a specific device. Params: `vid`, `pid`.
    FFI_HANDSHAKE_REQUESTED = "ffi.handshake.requested";
    /// The USB context could not be created. Params: `error`.
    FFI_HANDSHAKE_CONTEXT_FAILED = "ffi.handshake.context_failed";
    /// The device is already in accessory mode, so no handshake is needed.
    FFI_HANDSHAKE_ALREADY_ACCESSORY = "ffi.handshake.already_accessory";
    /// The handshake did not complete. Params: `error`.
    FFI_HANDSHAKE_FAILED = "ffi.handshake.failed";
    /// The device accepted the handshake and is switching mode.
    FFI_HANDSHAKE_SWITCHING = "ffi.handshake.switching";
    /// The device could not be opened for the handshake. Params: `error`.
    FFI_HANDSHAKE_OPEN_FAILED = "ffi.handshake.open_failed";

    // ── Re-enumeration after the mode switch ────────────────
    /// The device came back on the bus as an accessory.
    REENUM_ACCESSORY_FOUND = "reenum.accessory_found";
    /// The device never came back within the timeout.
    REENUM_TIMEOUT = "reenum.timeout";

    // ── Demuxer ─────────────────────────────────────────────
    /// Bytes ahead of the frame magic were dropped to resynchronise. Params: `bytes`.
    DEMUX_RESYNC_DISCARDED = "demux.resync.discarded";
    /// A frame carried a type tag this build does not define. Params: `type`.
    DEMUX_FRAME_UNKNOWN_TYPE = "demux.frame.unknown_type";
    /// A frame's length field exceeded the protocol maximum. Params: `bytes`.
    DEMUX_FRAME_OVERSIZED = "demux.frame.oversized";
    /// The first frame of a session was reassembled. Params: `kind`, `bytes`.
    DEMUX_FRAME_FIRST = "demux.frame.first";
    /// Periodic progress. Params: `frames`, `video`, `audio`, `discarded`.
    DEMUX_PROGRESS = "demux.progress";
    /// The reassembly buffer was cleared without ever finding a valid frame. Params: `bytes`.
    DEMUX_BUFFER_OVERFLOW = "demux.buffer.overflow";

    // ── Decoder ─────────────────────────────────────────────
    /// A hardware decode device was opened. Params: `device`.
    DECODER_INIT_HARDWARE = "decoder.init.hardware";
    /// No hardware device was available, so software decoding is in use.
    DECODER_INIT_SOFTWARE = "decoder.init.software";
    /// The decode thread has started.
    DECODER_THREAD_STARTED = "decoder.thread_started";
    /// The decoder could not be constructed at all. Params: `error`.
    DECODER_INIT_FAILED = "decoder.init.failed";
    /// The decode thread saw the shutdown signal.
    DECODER_THREAD_STOPPING = "decoder.thread_stopping";
    /// A single packet failed to decode. Params: `error`.
    DECODER_DECODE_FAILED = "decoder.decode_failed";
    /// Hardware decoding failed repeatedly, so the decoder is being rebuilt in software.
    DECODER_HARDWARE_FALLBACK = "decoder.hardware_fallback";

    // ── Player (child ffplay process) ───────────────────────
    /// Playback was asked for and is waiting on the first keyframe.
    PLAYER_WAITING_FOR_KEYFRAME = "player.waiting_for_keyframe";
    /// No keyframe arrived in time, so playback was abandoned.
    PLAYER_KEYFRAME_TIMEOUT = "player.keyframe_timeout";
    /// ffplay could not be started. Params: `error`.
    PLAYER_SPAWN_FAILED = "player.spawn_failed";
    /// ffplay started but its stdin pipe was not available to write to.
    PLAYER_STDIN_UNAVAILABLE = "player.stdin_unavailable";
    /// The Matroska muxer feeding ffplay could not be set up. Params: `error`.
    PLAYER_MUXER_FAILED = "player.muxer_failed";
    /// Playback is running. Params: `width`, `height`.
    PLAYER_PLAYING = "player.playing";
    /// The user closed the player window.
    PLAYER_WINDOW_CLOSED = "player.window_closed";
    /// The ffplay process exited.
    PLAYER_EXITED = "player.exited";
    /// Writing to the muxer failed mid-session. Params: `error`.
    PLAYER_MUX_ERROR = "player.mux_error";
    /// The playback session has finished and been cleaned up.
    PLAYER_SESSION_ENDED = "player.session_ended";

    // ── OBS feed ────────────────────────────────────────────
    /// The OBS feed was switched on or off. Params: `state`.
    OBS_FEED_TOGGLED = "obs.feed.toggled";
    /// The audio shared-memory segment could not be opened.
    OBS_SHMEM_AUDIO_OPEN_FAILED = "obs.shmem.audio_open_failed";
    /// The audio shared-memory segment is ready.
    OBS_SHMEM_AUDIO_READY = "obs.shmem.audio_ready";
    /// The audio shared-memory segment could not be mapped.
    OBS_SHMEM_AUDIO_MAP_FAILED = "obs.shmem.audio_map_failed";
    /// This platform has no shared-memory backend for the OBS feed.
    OBS_SHMEM_UNSUPPORTED = "obs.shmem.unsupported";
    /// The audio shared-memory segment was released.
    OBS_SHMEM_AUDIO_RELEASED = "obs.shmem.audio_released";
    /// Plugin installation has begun. Params: `version`.
    OBS_INSTALL_STARTED = "obs.install.started";
    /// The OBS plugin directory could not be located on this system.
    OBS_INSTALL_DIR_NOT_FOUND = "obs.install.dir_not_found";
    /// No prebuilt plugin was bundled, so one is being compiled locally.
    OBS_INSTALL_COMPILING = "obs.install.compiling";
    /// The local compile failed, most often because the OBS headers are missing.
    OBS_INSTALL_COMPILE_FAILED = "obs.install.compile_failed";
    /// Neither a bundled nor a locally built plugin binary exists.
    OBS_INSTALL_BINARY_MISSING = "obs.install.binary_missing";
    /// The plugin install directory could not be created.
    OBS_INSTALL_MKDIR_FAILED = "obs.install.mkdir_failed";
    /// Copying the plugin into place. Params: `from`, `to`.
    OBS_INSTALL_COPYING = "obs.install.copying";
    /// The plugin binary could not be copied. Params: `error`.
    OBS_INSTALL_COPY_FAILED = "obs.install.copy_failed";
    /// The version marker beside the plugin could not be written. Params: `path`, `error`.
    OBS_INSTALL_VERSION_WRITE_FAILED = "obs.install.version_write_failed";
    /// The plugin is installed. Params: `version`, `path`.
    OBS_INSTALL_COMPLETE = "obs.install.complete";

    // ── Driver and permission setup ─────────────────────────
    /// Asking the user to authorise the udev rule installation.
    DRIVER_SETUP_REQUESTING_ELEVATION = "driver.setup.requesting_elevation";
    /// pkexec itself could not be started. Params: `error`.
    DRIVER_SETUP_PKEXEC_LAUNCH_FAILED = "driver.setup.pkexec_launch_failed";
    /// The udev rule was written.
    DRIVER_SETUP_UDEV_INSTALLED = "driver.setup.udev_installed";
    /// The udev rule was not written, usually because the prompt was dismissed.
    DRIVER_SETUP_UDEV_FAILED = "driver.setup.udev_failed";
    /// WinUSB is already bound to the accessory interface.
    DRIVER_SETUP_WINUSB_PRESENT = "driver.setup.winusb_present";
    /// Installing WinUSB through the bundled libwdi helper.
    DRIVER_SETUP_LIBWDI_INSTALLING = "driver.setup.libwdi_installing";
    /// The libwdi helper finished.
    DRIVER_SETUP_LIBWDI_FINISHED = "driver.setup.libwdi_finished";
    /// The libwdi helper failed or its prompt was dismissed.
    DRIVER_SETUP_LIBWDI_FAILED = "driver.setup.libwdi_failed";
    /// Installing the WinUSB INF through pnputil.
    DRIVER_SETUP_PNPUTIL_INSTALLING = "driver.setup.pnputil_installing";
    /// pnputil registered the INF package.
    DRIVER_SETUP_PNPUTIL_INSTALLED = "driver.setup.pnputil_installed";
    /// pnputil rejected the package because it is unsigned.
    DRIVER_SETUP_PNPUTIL_UNSIGNED = "driver.setup.pnputil_unsigned";
    /// pnputil failed for some other reason, or its prompt was dismissed.
    DRIVER_SETUP_PNPUTIL_FAILED = "driver.setup.pnputil_failed";

    // ── Engine lifecycle ────────────────────────────────────
    /// Shared memory could not be initialised at startup. Params: `error`.
    SYSTEM_INIT_SHARED_MEMORY_FAILED = "system.init.shared_memory_failed";
    /// The session was stopped and every resource released.
    SYSTEM_SHUTDOWN_COMPLETE = "system.shutdown.complete";
    /// The USB context could not be created, so discovery cannot run. Params: `error`.
    SYSTEM_DISCOVERY_CONTEXT_FAILED = "system.discovery.context_failed";
    /// Background device scanning has started.
    SYSTEM_DISCOVERY_STARTED = "system.discovery.started";
    /// Background device scanning saw the shutdown signal.
    SYSTEM_DISCOVERY_STOPPING = "system.discovery.stopping";
    /// One bus poll failed; scanning continues. Params: `error`.
    SYSTEM_DISCOVERY_POLL_FAILED = "system.discovery.poll_failed";
}
