# 🔍 ScreenMirror — Comprehensive Issues Report

> **Generated**: 2026-03-22  
> **Scope**: Full codebase audit across `desktopApp` (Rust + TypeScript + React) and `mobileApp` (Flutter + Rust + Kotlin)

---

## Table of Contents

1. [🔴 Critical — Will Cause Crashes or Data Loss](#-critical--will-cause-crashes-or-data-loss)
2. [🟠 High — Functional Bugs & Reliability Issues](#-high--functional-bugs--reliability-issues)
3. [🟡 Medium — Performance & Robustness Gaps](#-medium--performance--robustness-gaps)
4. [🔵 Low — Code Quality & Maintainability](#-low--code-quality--maintainability)
5. [🟣 Architecture & Design Debt](#-architecture--design-debt)
6. [⚪ Missing Features / Incomplete Implementation](#-missing-features--incomplete-implementation)

---

## 🔴 Critical — Will Cause Crashes or Data Loss

### C-01: `unsafe static mut STATE` — Undefined Behavior (UB)

| Detail | Value |
|--------|-------|
| **File** | [lib.rs](file:///home/balaji/personalProjects/streamApp/desktopApp/mirror_backend/src/lib.rs#L30) |
| **Lines** | 30, 223, 229, 260, 274, 279 |
| **Severity** | 🔴 Critical |

```rust
static mut STATE: Option<MirrorState> = None;
```

This `static mut` is accessed from **multiple threads** (the main thread, the decoder thread, and the USB streaming thread) without any synchronization. This is **textbook undefined behavior** in Rust. Concurrent reads and writes to `static mut` can cause data races, memory corruption, and segfaults.

**Fix**: Replace with `static STATE: Lazy<Mutex<Option<MirrorState>>>` or `OnceLock<MirrorState>`.

---

### C-02: Memory Leak — `CString::into_raw()` without guaranteed `free_string()`

| Detail | Value |
|--------|-------|
| **File** | [lib.rs](file:///home/balaji/personalProjects/streamApp/desktopApp/mirror_backend/src/lib.rs#L100-L133) |
| **Lines** | 100–133 |
| **Severity** | 🔴 Critical |

Functions `get_devices()`, `get_structured_logs()`, `get_new_logs()`, and `get_metrics()` all call `CString::into_raw()` which transfers ownership to the caller. If the Bun/FFI side ever skips calling `free_string()` (e.g., on exception, early return, or RPC timeout), **memory is permanently leaked**. In a polling loop running every 500ms, this compounds rapidly.

Looking at [index.ts](file:///home/balaji/personalProjects/streamApp/desktopApp/src/bun/index.ts#L146-L151), the `readCString()` helper does call `free_string()`, but **only after constructing a new `CString` object**. If the `CString` constructor or `toString()` throws, the pointer is leaked.

**Fix**: Wrap `readCString()` in a try/finally, or use Rust's `Box<str>` with a proper FFI contract.

---

### C-03: Double `getInitialAccessory` Call — Duplicate USB Pipelines

| Detail | Value |
|--------|-------|
| **File** | [main.dart](file:///home/balaji/personalProjects/streamApp/mobileApp/lib/main.dart#L82-L98) |
| **Lines** | 82–98 (in `initState`) and 136–161 (in `_setupUsb`) |
| **Severity** | 🔴 Critical |

Both `_checkInitialAccessory()` (called from `initState`) **and** `_setupUsb()` call `getInitialAccessory`. If the Android side has a pending FD, **both paths will receive it** and call `_onConnected(fd)` **twice**, spawning duplicate USB read/write threads against the same file descriptor. This causes data corruption and race conditions on the FD.

**Fix**: Remove the duplicate `getInitialAccessory` call from `_setupUsb()` or add a guard (`_connState != 'idle'`) before calling `_onConnected()`.

---

### C-04: Unclosed File Descriptor on Error Path (Mobile Rust)

| Detail | Value |
|--------|-------|
| **File** | [usb_loop.rs](file:///home/balaji/personalProjects/streamApp/mobileApp/rust/src/usb_loop.rs#L73-L100) |
| **Lines** | 73–100 (read thread) |
| **Severity** | 🔴 Critical |

The USB read thread (first `std::thread::spawn`) uses the same `fd` as the write thread but **never closes it**. If the read thread exits first (e.g., on error), the `fd` is left dangling. The write thread later calls `libc::close(fd)` on line 170, but if the read thread breaks first, it might leave the kernel-side state inconsistent.

More critically, if `start_usb_loop(fd)` is called again before the write thread finishes cleanup, there's a **use-after-close** on the old FD.

**Fix**: Centralize FD lifecycle management. Use `Arc<AtomicI32>` for the FD and close it exactly once.

---

## 🟠 High — Functional Bugs & Reliability Issues

### H-01: USB Inactivity Timeout Uses Wrong Unit

| Detail | Value |
|--------|-------|
| **File** | [receiver.rs](file:///home/balaji/personalProjects/streamApp/desktopApp/mirror_backend/src/receiver.rs#L244-L261) |
| **Lines** | 244–261 |
| **Severity** | 🟠 High |

```rust
Err(rusb::Error::Timeout) => {
    idle_seconds += 1;
    if idle_seconds >= 5 { ... break; }
}
```

The `read_bulk` timeout is set to **1000ms**, so each timeout increments `idle_seconds`. However, timeouts can also occur due to normal USB bus contention. `idle_seconds` is reset only on `Ok(len) if len > 0`, meaning even a single zero-byte read (`Ok(0)`) won't reset the counter, and 5 consecutive 1-second timeouts will false-positive disconnect.

**Fix**: Use `Instant::now()` to track actual wall-clock idle time instead of a counter.

---

### H-02: `StreamingActiveGuard` Lock Can Be Poisoned

| Detail | Value |
|--------|-------|
| **File** | [receiver.rs](file:///home/balaji/personalProjects/streamApp/desktopApp/mirror_backend/src/receiver.rs#L140-L150) |
| **Lines** | 140–150 |
| **Severity** | 🟠 High |

The `StreamingActiveGuard::drop()` implementation tries to lock `STREAMING_ACTIVE` twice:
```rust
if let Ok(mut active) = STREAMING_ACTIVE.lock() {
    *active = false;
} else if let Err(e) = STREAMING_ACTIVE.lock() {
    *e.into_inner() = false;
}
```

The `else if` branch calls `.lock()` a **second time** — if the first lock attempt failed because the mutex is poisoned, the second call will also fail with `.lock()` returning an `Err`. The `into_inner()` call on the second error is correct but redundant and confusing.

**Fix**: Use a single lock call with `unwrap_or_else(|e| e.into_inner())`.

---

### H-03: `FORCE_DISCONNECT` is Never Reset After Manual Handshake

| Detail | Value |
|--------|-------|
| **File** | [lib.rs](file:///home/balaji/personalProjects/streamApp/desktopApp/mirror_backend/src/lib.rs#L58-L72) |
| **Severity** | 🟠 High |

`force_disconnect()` sets `FORCE_DISCONNECT = true` **and** disables `AUTO_RECONNECT_ENABLED`. When the user later clicks "Initiate" on a device, `trigger_manual_handshake()` re-enables auto-reconnect but **doesn't clear `FORCE_DISCONNECT`**. The streaming loop checks `FORCE_DISCONNECT` on each iteration and immediately breaks if it's still `true`.

This means: **Disconnect → Connect again → stream immediately terminates**.

The flag is only cleared inside the streaming loop after it triggers the break (line 237), but the new streaming loop checks the flag **before** the old value is cleared.

**Fix**: Clear `FORCE_DISCONNECT` in `trigger_manual_handshake()` and in `toggleAutoReconnect()`.

---

### H-04: Audio Sent as Raw PCM Float But Receiver Expects Encoded AAC

| Detail | Value |
|--------|-------|
| **File** | [audio_capture.rs](file:///home/balaji/personalProjects/streamApp/mobileApp/rust/src/audio_capture.rs#L29-L36) & [muxer.rs](file:///home/balaji/personalProjects/streamApp/mobileApp/rust/src/muxer.rs#L29-L33) |
| **Severity** | 🟠 High |

The `AudioCapture::on_audio_ready()` callback converts f32 PCM samples to raw bytes and sends them directly to `Muxer::push_audio()`. The Muxer's comments say "Push raw AAC encoded audio data", but the data is actually **uncompressed PCM**. There's no audio encoder in the pipeline.

On the desktop side, the `Demuxer` strips audio frames correctly, but they're **never processed** — line 249 of `receiver.rs` only handles `FrameType::Video`:
```rust
if matches!(frame.frame_type, crate::demuxer::FrameType::Video) {
    push_packet(frame.data.as_ptr(), frame.data.len());
}
```

**Result**: Audio frames waste USB bandwidth but are silently discarded. The README claims "Audio Mirroring" but it's non-functional.

**Fix**: Either add an audio encoder (AAC/Opus) on the mobile side and a decoder on desktop, or remove the audio pipeline entirely and update docs.

---

### H-05: `unwrap()` Calls on Mutex Locks Will Panic on Poisoning

| Detail | Value |
|--------|-------|
| **Files** | Multiple |
| **Severity** | 🟠 High |

Several critical paths use `.unwrap()` on mutex locks:

- [lib.rs:101](file:///home/balaji/personalProjects/streamApp/desktopApp/mirror_backend/src/lib.rs#L101): `DISCOVERED_DEVICES.lock().unwrap()`
- [lib.rs:109](file:///home/balaji/personalProjects/streamApp/desktopApp/mirror_backend/src/lib.rs#L109): `LOG_BUFFER.lock().unwrap()`
- [lib.rs:238](file:///home/balaji/personalProjects/streamApp/desktopApp/mirror_backend/src/lib.rs#L238): `metrics::METRICS.lock().unwrap()`
- [api.rs:90](file:///home/balaji/personalProjects/streamApp/mobileApp/rust/src/api.rs#L90): `METRICS.lock().unwrap()`
- [usb_loop.rs:60](file:///home/balaji/personalProjects/streamApp/mobileApp/rust/src/usb_loop.rs#L60): `USB_ACTIVE.lock().unwrap()`

If any thread panics while holding one of these locks, the mutex becomes **poisoned** and all subsequent `.unwrap()` calls will **cascade-panic** every thread that touches it, crashing the entire application.

**Fix**: Use `.lock().unwrap_or_else(|e| e.into_inner())` or the pattern already used in some places.

---

### H-06: `#[repr(C, packed)]` FrameHeader — Unaligned Access UB

| Detail | Value |
|--------|-------|
| **File** | [lib.rs](file:///home/balaji/personalProjects/streamApp/desktopApp/mirror_backend/src/lib.rs#L15-L21) |
| **Severity** | 🟠 High |

```rust
#[repr(C, packed)]
pub struct FrameHeader {
    pub magic: [u8; 4],
    pub width: u32,
    pub height: u32,
    pub timestamp: u64,
}
```

Taking a reference to fields in a `packed` struct is UB because the fields may be unaligned. Line 233 creates a reference via `&header`, which may produce an unaligned pointer. On x86 this usually works but **Miri and strict UB checking will flag this**, and on ARM targets (like Android NDK) it can crash.

**Fix**: Use `#[repr(C)]` without `packed`, or use byte-level writes.

---

## 🟡 Medium — Performance & Robustness Gaps

### M-01: Unbounded `ConcurrentQueue` Can Exhaust Memory

| Detail | Value |
|--------|-------|
| **File** | [lib.rs](file:///home/balaji/personalProjects/streamApp/desktopApp/mirror_backend/src/lib.rs#L220) |
| **Severity** | 🟡 Medium |

```rust
let queue = Arc::new(ConcurrentQueue::unbounded());
```

If the decoder thread falls behind the USB receiver (e.g., CPU spike, decoder stall), the unbounded queue will fill up indefinitely, consuming all available RAM.

**Fix**: Use `ConcurrentQueue::bounded(max_frames)` and drop the oldest frames when full.

---

### M-02: Decoder Thread Spin-Waits With 1ms Sleep

| Detail | Value |
|--------|-------|
| **File** | [decoder.rs](file:///home/balaji/personalProjects/streamApp/desktopApp/mirror_backend/src/decoder.rs#L76-L87) |
| **Severity** | 🟡 Medium |

```rust
loop {
    if let Ok(packet_data) = queue.pop() {
        // ...
    } else {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
```

When the queue is empty, the thread sleeps for 1ms then retries. This wastes CPU in busy-wait fashion at ~1000 wakeups/second. On battery-powered machines this drains resources.

**Fix**: Use a `Condvar` or channel-based approach that parks the thread until data is available.

---

### M-03: `buffer_health` Hardcoded to 0.85

| Detail | Value |
|--------|-------|
| **File** | [metrics.rs](file:///home/balaji/personalProjects/streamApp/desktopApp/mirror_backend/src/metrics.rs#L79) |
| **Severity** | 🟡 Medium |

```rust
buffer_health: 0.85, // Mock for now, will link to Jitter buffer later
```

The `buffer_health` metric is reported to the UI but is always `0.85`. This misleads the user into thinking the pipeline is healthy when it may not be.

**Fix**: Compute actual health from `queue.len() / max_capacity`, or remove the metric until implemented.

---

### M-04: No HW Acceleration Actually Configured in Decoder

| Detail | Value |
|--------|-------|
| **File** | [decoder.rs](file:///home/balaji/personalProjects/streamApp/desktopApp/mirror_backend/src/decoder.rs#L17-L37) |
| **Severity** | 🟡 Medium |

The decoder logs hardware acceleration hints per platform but **never actually configures** FFmpeg's `hw_device_ctx` or sets a hardware pixel format. The decoder falls back to CPU-only software decoding.

```rust
#[cfg(target_os = "linux")]
log_event("INFO", "DECODER", "init", "Targeting Linux NVDEC/VAAPI acceleration");
// ... but no code to enable it
```

This is a performance bottleneck for high-res/high-fps streams.

**Fix**: Use `ffmpeg_next`'s hardware device context APIs to request VAAPI/NVDEC/DXVA2.

---

### M-05: `CircularBuffer` Full → Data Silently Dropped

| Detail | Value |
|--------|-------|
| **File** | [api.rs](file:///home/balaji/personalProjects/streamApp/mobileApp/rust/src/api.rs#L41-L48) |
| **Severity** | 🟡 Medium |

```rust
pub fn push(&mut self, packet: &[u8]) -> bool {
    if packet_len > available { return false; }
    // ...
}
```

When the buffer is full, `push()` silently returns `false` and the frame is **lost** without any logging or metric update. The caller `push_to_usb()` also returns `false` silently (no retry, no backpressure).

**Fix**: Log dropped frames and increment a drop counter in `METRICS`.

---

### M-06: USB Discovery Loop Polls Every 2 Seconds — Resource Waste

| Detail | Value |
|--------|-------|
| **File** | [receiver.rs](file:///home/balaji/personalProjects/streamApp/desktopApp/mirror_backend/src/receiver.rs#L359-L427) |
| **Severity** | 🟡 Medium |

The USB listener thread polls `context.devices()` every 2 seconds in a tight loop. On Linux, libusb supports **hotplug callbacks** via `rusb::HotplugBuilder`, which would be far more efficient and responsive.

**Fix**: Use rusb hotplug API on supported platforms, falling back to polling on others.

---

### M-07: Encoding Latency Not Measured (Mobile)

| Detail | Value |
|--------|-------|
| **File** | [usb_loop.rs](file:///home/balaji/personalProjects/streamApp/mobileApp/rust/src/usb_loop.rs#L121-L153) |
| **Severity** | 🟡 Medium |

The `MobileMetrics.encoding_latency_ms` field is defined but **never set**. The metrics timer only updates `throughput_mbps` and `fps_actual`. The mobile UI shows "LATENCY: 0ms" permanently.

**Fix**: Measure time between frame push and USB write completion.

---

## 🔵 Low — Code Quality & Maintainability

### L-01: `type` used as Variable Name in TSX (Reserved in Strict Mode)

| Detail | Value |
|--------|-------|
| **File** | [App.tsx](file:///home/balaji/personalProjects/streamApp/desktopApp/src/mainview/App.tsx#L178-L179) |
| **Severity** | 🔵 Low |

```tsx
const type = parts[0];  // Line 178
// ...
const [type, name, id] = dev.split('|');  // Line 378
```

`type` is a reserved keyword in TypeScript. While it works as a variable name in runtime JavaScript, it can cause confusion and linter warnings.

**Fix**: Rename to `deviceType` or `devType`.

---

### L-02: No Error Handling for `readCString` Parsing Failures

| Detail | Value |
|--------|-------|
| **File** | [index.ts](file:///home/balaji/personalProjects/streamApp/desktopApp/src/bun/index.ts#L94-L113) |
| **Severity** | 🔵 Low |

```typescript
const devices = readCString(lib.symbols.get_devices()).split(',').filter(s => s.length > 0);
```

If `get_devices()` returns a null pointer (possible if `DISCOVERED_DEVICES` lock is poisoned), `readCString` returns `""`, which is handled. But `split(',')` on an empty string returns `[""]`, not `[]`. The `.filter(s => s.length > 0)` saves this case, but the intention is fragile.

**Fix**: Add explicit null-pointer checks and consider returning JSON arrays instead of comma-separated strings from Rust.

---

### L-03: Missing `_configPollTimer` Disposal in `dispose()`

| Detail | Value |
|--------|-------|
| **File** | [main.dart](file:///home/balaji/personalProjects/streamApp/mobileApp/lib/main.dart#L109-L117) |
| **Severity** | 🔵 Low |

```dart
@override
void dispose() {
    _metricsTimer?.cancel();
    _uptimeTimer?.cancel();
    _pulse.dispose();
    _glow.dispose();
    _logScroll.dispose();
    super.dispose();
}
```

`_configPollTimer` is not cancelled in `dispose()`. If the widget is disposed while connected, the timer continues to fire, calling `setState()` on a disposed widget (which triggers a Flutter framework error).

**Fix**: Add `_configPollTimer?.cancel();` to `dispose()`.

---

### L-04: Unused `FRAME_COUNTER` Import/Dead Code

| Detail | Value |
|--------|-------|
| **File** | [decoder.rs](file:///home/balaji/personalProjects/streamApp/desktopApp/mirror_backend/src/decoder.rs#L10) |
| **Severity** | 🔵 Low |

`FRAME_COUNTER` is used as the timestamp for each frame, but it's just an incrementing integer — it has no relation to actual wall-clock time. This means PTS values sent to FFmpeg are `0, 1, 2, 3...` which are not valid timestamps. FFmpeg may misinterpret frame timing.

**Fix**: Use actual microsecond timestamps or at least multiply by frame duration.

---

### L-05: `greet()` and `startAoa()` — Dead API Functions

| Detail | Value |
|--------|-------|
| **File** | [api.rs](file:///home/balaji/personalProjects/streamApp/mobileApp/rust/src/api.rs#L104-L111) |
| **Severity** | 🔵 Low |

```rust
pub fn start_aoa() -> Result<String> {
    Ok("AOA mode managed by Android framework".to_string())
}
pub fn greet(name: String) -> String { format!("Hello, {name}!") }
```

Both are unused legacy functions that add dead code to the binary and clutter the API surface.

**Fix**: Remove or gate behind a `#[cfg(test)]`.

---

### L-06: Inconsistent `CString::new().unwrap()` — Can Panic on Null Bytes

| Detail | Value |
|--------|-------|
| **File** | [lib.rs](file:///home/balaji/personalProjects/streamApp/desktopApp/mirror_backend/src/lib.rs#L103-L104) |
| **Severity** | 🔵 Low |

```rust
let c_str = std::ffi::CString::new(combined).unwrap();
```

If the device list string contains a null byte (possible from corrupt USB descriptors), `CString::new()` will **panic**, crashing the whole application.

**Fix**: Replace null bytes before constructing `CString`, or use `.unwrap_or_else()`.

---

## 🟣 Architecture & Design Debt

### A-01: No Graceful Shutdown Protocol

| Detail | |
|--------|---|
| **Scope** | Desktop Backend |
| **Impact** | 🟣 Architectural |

There is no clean shutdown path. Threads (`decoder`, `USB listener`, `streaming loop`) are spawned with `std::thread::spawn` and have no join handles stored. When the application exits:
- The decoder thread runs forever (infinite `loop`)
- The USB listener thread runs forever (infinite `loop`)
- There's no `Drop` implementation on `MirrorState` to clean up shared memory

**Fix**: Store `JoinHandle`s, use atomic flags for shutdown, and implement `Drop` for `MirrorState`.

---

### A-02: Shared Memory Segment Never Cleaned Up

| Detail | |
|--------|---|
| **Scope** | Desktop — `init_mirror()` |
| **Impact** | 🟣 Architectural |

`ShmemConf::new().os_id("obs_mirror_buffer").create()` creates a system-wide shared memory segment with a fixed name. If the app crashes, the segment persists. On the next launch, it falls through to `.open()`, which may have a **different size** if the resolution changed. There's no validation that the opened segment matches the expected dimensions.

**Fix**: Delete stale segments on init, and validate sizes when opening existing ones.

---

### A-03: Rust Backend is a `cdylib` Called via FFI — No Type Safety

| Detail | |
|--------|---|
| **Scope** | Desktop — Bun ↔ Rust boundary |
| **Impact** | 🟣 Architectural |

The entire desktop backend is exposed as C-style `extern "C"` functions called via Bun's `dlopen`/FFI. This means:
- No type checking at the boundary
- Manual memory management (`CString::into_raw` / `free_string`)
- Pointer types are all `FFIType.ptr` with no safety
- Incorrect call signatures will silently corrupt memory

**Recommendation**: Consider using napi-rs for type-safe Node/Bun bindings, or generate bindings via cbindgen with validation.

---

### A-04: No Protocol Versioning Between Mobile and Desktop

| Detail | |
|--------|---|
| **Scope** | Cross-component |
| **Impact** | 🟣 Architectural |

The Muxer wire format (`[magic][type][length][data]`) has no version field. If the format changes (e.g., adding a timestamp field, changing to a different magic), old desktop clients will silently fail to parse streams from new mobile apps and vice versa.

**Fix**: Add a version negotiation handshake or a version byte in the header.

---

### A-05: Monolithic `main.dart` — 535 Lines, Mixed Concerns

| Detail | |
|--------|---|
| **Scope** | Mobile App |
| **Impact** | 🟣 Architectural |

All UI, state management, USB handling, Rust bridging, metrics polling, and config management are in a single `main.dart` file. This makes the code hard to test, maintain, and extend.

**Fix**: Extract into:
- `usb_controller.dart` — USB lifecycle & MethodChannel
- `streaming_state.dart` — State management (consider Riverpod/Bloc)
- `metrics_service.dart` — Metrics polling
- `widgets/` — UI components

---

## ⚪ Missing Features / Incomplete Implementation

### F-01: No Audio Playback on Desktop

The README claims "Audio Mirroring" as a key feature, but:
1. Mobile sends raw PCM (not encoded audio)
2. Desktop demuxer extracts audio frames but **discards them**
3. There is no audio decoder or playback engine on the desktop side

---

### F-02: No Windows Driver Installation Logic

| Detail | Value |
|--------|-------|
| **File** | [lib.rs](file:///home/balaji/personalProjects/streamApp/desktopApp/mirror_backend/src/lib.rs#L158-L169) |

```rust
pub extern "C" fn install_windows_driver() -> i32 {
    #[cfg(target_os = "windows")]
    {
        receiver::log_event("INFO", "DRIVER", "setup", "Windows Driver Installation initiated...");
        0  // No actual driver installation
    }
}
```

The Windows driver installation is a stub that just logs a message and returns success. Windows users will have no working USB driver setup.

---

### F-03: OBS Feed Toggle is a No-Op

| Detail | Value |
|--------|-------|
| **File** | [index.ts](file:///home/balaji/personalProjects/streamApp/desktopApp/src/bun/index.ts#L82-L87) |

```typescript
toggleObsFeed: (data: { enabled: boolean }) => {
    console.log(`Enterprise RPC: OBS Feed toggled to ${data.enabled}`);
    // In a production app, we'd pass this flag to the Rust backend
    return { success: true };
},
```

The UI has a "Direct to OBS" button, but toggling it does nothing on the backend side.

---

### F-04: WGPU Renderer Removed — Only `ffplay` Fallback

The README highlights **WGPU** for "Cross-platform, native GPU rendering", but [renderer.rs](file:///home/balaji/personalProjects/streamApp/desktopApp/mirror_backend/src/renderer.rs) only contains an `ffplay` pipe-based renderer. The WGPU renderer has been removed or was never implemented.

---

### F-05: No Error Recovery / Auto-Reconnect on Mobile

The mobile app's `_onDisconnected()` transitions to `idle` state but provides **no automatic reconnection** mechanism. If the USB cable is briefly disconnected and reconnected, the user must relaunch the app or wait for the Android system to re-trigger the accessory intent.

---

### F-06: `setup_udev.sh` Only Handles Google Vendor ID

| Detail | Value |
|--------|-------|
| **File** | [setup_udev.sh](file:///home/balaji/personalProjects/streamApp/setup_udev.sh) |

```bash
echo 'SUBSYSTEM=="usb", ATTR{idVendor}=="18d1", MODE="0666", GROUP="plugdev"' | sudo tee ...
```

Only the Google vendor ID (`18d1`) is whitelisted, but `setup_linux_permissions()` in Rust also adds Samsung (`04e8`) and another vendor (`2d95`). The shell script and Rust code are **out of sync**.

---

### F-07: No LICENSE File

The README mentions "Distributed under the MIT License. See `LICENSE` for more information" but there is no `LICENSE` file in the repository.

---

## Summary

| Category | Count |
|----------|-------|
| 🔴 Critical | 4 |
| 🟠 High | 6 |
| 🟡 Medium | 7 |
| 🔵 Low | 6 |
| 🟣 Architecture | 5 |
| ⚪ Missing/Incomplete | 7 |
| **Total** | **35** |

### Recommended Priority Order

> [!IMPORTANT]
> Address these first to prevent crashes and data loss:
> 1. **C-01**: Replace `static mut STATE` with thread-safe wrapper  
> 2. **C-03**: Fix duplicate `getInitialAccessory` call  
> 3. **C-04**: Fix FD lifecycle in mobile USB loop  
> 4. **H-03**: Fix `FORCE_DISCONNECT` not clearing on reconnect  
> 5. **H-05**: Replace all `.unwrap()` on mutex locks  
> 6. **C-02**: Make FFI string memory management exception-safe  

> [!TIP]
> For the next iteration, consider:
> - Implementing actual hardware-accelerated decoding (**M-04**)
> - Either completing or removing the audio pipeline (**H-04**, **F-01**)
> - Breaking up `main.dart` into proper architecture (**A-05**)
