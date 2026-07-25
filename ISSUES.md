# Project Issue Tracker & Roadmap 🗺️

Tracks performance bottlenecks, stability issues, and planned improvements for the ScreenMirror pipeline.
See [CODE_REVIEW_REPORT.md](CODE_REVIEW_REPORT.md) for the full audit this list is derived from.

## ✅ Resolved (2026-07-25 pipeline overhaul)

**Build breaks**
- Duplicate `MAGIC` constant in `demuxer.rs` and missing `memchr`/`bytes` dependencies.
- `Muxer::frame_packet` referenced by a deleted legacy struct on mobile.

**Throughput / smoothness (the 120 fps stutter)**
- **Partial USB writes** on mobile silently truncated frames under load; now a full write loop with `EAGAIN`/`EINTR` back-off.
- **Blocking 2-slot channel** shared by audio and video stalled both the MediaCodec drain thread and the audio capture thread. Replaced with separate non-blocking queues (drop-oldest video, audio priority on drain).
- **Encoder capped at 60 fps** — `KEY_OPERATING_RATE`, `max-fps-to-encoder` and a `MediaCodecInfo` capability clamp are now set.
- **Global `STATE` mutex** serialized the USB reader against the decoder's 8 MB frame copies; the hot path is now lock-free (`Arc` handles + a dedicated delivery function).
- **Fake hardware decode** (`hevc_vaapi`/`hevc_videotoolbox` are not decoder names) — real `hw_device_ctx` negotiation with a `get_format` callback, GPU frame download, and an automatic software fallback after persistent failures. Software decode now uses frame threading.
- **5 ms sleep-polling** in the decoder replaced with a condvar-blocking queue.
- **Ingress drops discarded keyframes/CSD**; the queue now drops the oldest *droppable* packet and preserves parameter sets and IRAP frames.
- **Keyframe detection assumed 4-byte start codes** — the NAL scanner handles 3- and 4-byte start codes and multi-NAL packets.
- **Blocking file I/O in `log_event`** on streaming threads moved to a dedicated writer thread.
- Frame buffer pool enlarged (3 → 6/8) so the decoder stops allocating 8 MB buffers per frame under load.

**Correctness / stability**
- `stop_mirror()` was never exported to JS and raced its own flag reset; replaced with a monotonic session generation, exported over FFI and wired to process exit.
- `init_mirror()` is idempotent — no more duplicate USB listener threads on re-init.
- Encoder output buffers are always released (zero-size/EOS buffers used to leak until the codec froze).
- Codec config (VPS/SPS/PPS) is cached and replayed on reconnect, plus `KEY_PREPEND_HEADER_TO_SYNC_FRAMES`.
- Accessory fd ownership moved to Rust via `detachFd()` — the double close is gone.
- `KEY_REPEAT_PREVIOUS_FRAME_AFTER` keeps static screens from tripping the desktop's 5 s inactivity timeout.
- Control commands are parsed with `serde_json` on both ends, and the mobile reader accumulates until NUL instead of assuming whole messages per read.
- MethodChannel setup moved to `configureFlutterEngine` (cold-start race).
- Projection start/stop moved off the main thread.
- Preview R/B swap fixed (`ARGB8888`, not `ABGR8888`).
- Triple buffer gained a per-slot seqlock so a lapping writer can no longer tear a reader's copy; header layout is now a fixed 64 bytes asserted on both sides.

**Audio**
- Replaced Oboe **microphone** capture with `AudioPlaybackCapture` (actual system/game audio), f32 mono 48 kHz, with the matching foreground-service type and permissions.
- Desktop now requests a 48 kHz f32 output config instead of silently resampling-by-accident; playback is unmuted by default; overflow keeps the newest packet.

**Platform coverage**
- OBS plugin ported to Windows (named file mappings, MSVC-compatible atomics) with a CMake build for all three platforms.
- Windows WinUSB driver automation (`windows_driver.rs`): libwdi path when bundled, otherwise a generated WinUSB INF installed via `pnputil` — one UAC prompt, run at startup only when the driver is actually missing.
- Linux udev rules use `uaccess` tagging instead of world-writable `MODE="0666"`; OBS audio SHM is `0600`; elevation is a single `pkexec` call.

**Dead code removed**
- `audio_engine.rs` (unused resampler), `video_processing.rs` (fake-SIMD converter), mobile `CircularBuffer`/`USB_BUFFER` (written, never read), `AvPacket`.

---

## 🚩 Open Items

### 1. Missing source timestamps
- **Status**: 🔴 Backlog
- **Problem**: The wire protocol has no sender-side timestamp; the receiver invents one, so real jitter correction and A/V sync are impossible and end-to-end latency can only be inferred.
- **Goal**: Add a 64-bit microsecond capture timestamp to the frame header in `muxer.rs`/`demuxer.rs`.

### 2. Zero-copy decode path
- **Status**: 🔴 Backlog
- **Problem**: Hardware frames are downloaded to system memory and converted to BGRA on the CPU before reaching OBS/preview.
- **Goal**: Keep frames on the GPU (DMA-BUF/D3D11 shared texture) and hand OBS a texture instead of a memcpy.

### 3. Dynamic rotation handling
- **Status**: 🟡 Planned
- **Problem**: Encoder dimensions are chosen once at projection start; rotating the device mid-session leaves the aspect ratio wrong.
- **Goal**: Recreate the VirtualDisplay/encoder on configuration change.

### 4. Protocol CRC/checksum
- **Status**: 🔵 Low
- **Goal**: 32-bit CRC per packet to detect corruption instead of relying on magic-header resync.

### 5. Signed Windows driver package
- **Status**: 🔵 Low
- **Problem**: The generated WinUSB INF is unsigned, so `pnputil` refuses it when driver signature enforcement is on (the libwdi path handles this, but only when `bin\wdi-simple.exe` is bundled).
- **Goal**: Ship a signed catalog, or bundle libwdi by default in the Windows release.

---

## 📊 Performance Targets

| Metric | Target | Notes |
| :--- | :--- | :--- |
| **End-to-End Latency** | < 40 ms | Needs item #1 (source timestamps) to measure honestly |
| **USB Throughput** | 150+ Mbps | AOA over USB 2.0 high-speed caps around 30–35 MB/s |
| **CPU Usage (Idle)** | < 1 % | |
| **Frame Drop Rate** | < 0.1 % | Reported live via `buffer_health` + `frames_dropped` in the dashboard |
