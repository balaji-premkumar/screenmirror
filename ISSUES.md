# Streaming Pipeline - Frame Drop & Latency Issues

## Overview

The application experiences frame drops, shuttering/lagging on desktop, and streaming lag when capturing high-FPS mobile games compared to alternatives like Duowan.

---

## Root Cause Analysis

### Mobile Side Issues

#### 1. No Frame Rate Limiting
**Location**: `mobileApp/rust/src/usb_loop.rs:146-204`
**Problem**: Frames are sent as fast as they arrive from MediaCodec, causing USB bandwidth overflow.

#### 2. No Encoder Backpressure
**Location**: `mobileApp/rust/src/api.rs:54-68`, `mobileApp/rust/src/muxer.rs:24-27`
**Problem**: The muxer's `SyncSender` silently drops frames when full. The encoder doesn't know to throttle.

#### 3. Synchronous USB Writes
**Location**: `mobileApp/rust/src/usb_loop.rs:169-176`
**Problem**: `libc::write()` blocks the encoder thread if the USB controller is busy.

#### 4. Audio Blocking Video Pipeline
**Location**: `mobileApp/rust/src/usb_loop.rs:150-164`
**Problem**: Audio and video share the same USB write loop. Audio latency impacts video.

#### 5. [NEW] Allocation Churn & Inefficient Copying
**Location**: `mobileApp/rust/src/muxer.rs`, `mobileApp/rust/src/api.rs`
**Problem**: 
- A new `Vec<u8>` is allocated for every frame/packet, causing high GC pressure.
- `CircularBuffer::push` uses a byte-by-byte loop instead of `copy_from_slice`.

#### 6. [NEW] Brittle Control Command Parsing
**Location**: `mobileApp/rust/src/usb_loop.rs:90-105`
**Problem**: Uses `part.contains("\"command\":\"start\"")` on raw strings. This is prone to false positives or failures if the JSON structure changes slightly.

#### 7. [NEW] Audio Lifecycle Risks
**Location**: `mobileApp/rust/src/audio_capture.rs`
**Problem**: `AudioCapture` lacks an explicit `stop()` or `close()` on the Oboe stream, potentially leaving threads dangling during pipeline restarts.

---

### Desktop Side Issues

#### 8. Decoder Queue Too Small
**Location**: `desktopApp/mirror_backend/src/lib.rs:331`
**Problem**: Bounded(2) is too small; packets drop immediately if the preview thread lags.

#### 9. Decoder Not Throttled / Busy Loop
**Location**: `desktopApp/mirror_backend/src/decoder.rs:186-232`
**Problem**: 
- The decoder thread uses `std::thread::sleep(1ms)` when empty instead of a proper condition variable/event-driven wait, wasting CPU.
- Jitter buffer only triggers at > 10 packets, but queue is size 2.

#### 10. Triple Buffer Memory Fence Contention
**Location**: `desktopApp/mirror_backend/src/shared_mem.rs:60-102`
**Problem**: `fence(Ordering::Release)` overhead accumulates at high FPS (60+).

#### 11. OBS & Preview Race Condition
**Location**: `desktopApp/mirror_backend/src/lib.rs:350-367`
**Problem**: Synchronous writes to both triple buffer AND OBS SHM.

#### 12. [NEW] $O(N^2)$ Demuxer Scanning
**Location**: `desktopApp/mirror_backend/src/demuxer.rs:185`
**Problem**: `find_magic` rescans the entire reassembly buffer for every new chunk, becoming exponentially slower as the buffer fills.

#### 13. [NEW] Artifacting (GOP Unaware Dropping)
**Location**: `desktopApp/mirror_backend/src/decoder.rs:200`
**Problem**: The jitter buffer drops packets indiscriminately. If an HEVC I-frame (Keyframe) is dropped, the video will "smear" until the next one arrives.

#### 14. [NEW] Vsync Rendering Bottleneck
**Location**: `desktopApp/mirror_backend/src/renderer.rs:65`
**Problem**: SDL2 is initialized with `present_vsync()`. This locks rendering to the monitor's refresh rate (e.g., 60Hz), forcing frame drops if the source is 90/120 FPS.

#### 15. [NEW] Unsafe Triple Buffer Implementation
**Location**: `desktopApp/mirror_backend/src/shared_mem.rs`
**Problem**: Lacks proper state tracking. A fast writer can overwrite a slot while a slow reader (OBS) is still copying from it, causing tearing/crashes.

#### 16. [NEW] Global Mutex Contention (STATE)
**Location**: `desktopApp/mirror_backend/src/lib.rs`
**Problem**: The global `STATE` mutex is held during heavy operations (frame writing, logging). This can block the USB receiver thread, causing backlog.

---

### Protocol & Architectural Issues

#### 17. No Frame Timestamps
**Location**: `mobileApp/rust/src/muxer.rs:35-45`, `desktopApp/mirror_backend/src/demuxer.rs:60-181`
**Problem**: Receiver uses its own counter. Real timing info is lost, making proper jitter correction or A/V sync impossible.

#### 18. No CRC/Checksum
**Problem**: Silent data corruption during USB transfer can cause decoder crashes or visual glitches.

#### 19. [NEW] Missing Graceful Shutdown (FFI)
**Location**: `desktopApp/mirror_backend/src/lib.rs`
**Problem**: No `stop_mirror()` function. Background threads (USB listener, decoder) continue running until the process dies, making clean app restarts difficult.

---

## Priority Fixes

### High Priority (Critical for Performance)
1. **Optimize Demuxer Magic Search**: Use `memchr` or a sliding window to avoid $O(N^2)$ rescanning.
2. **Increase Decoder Queue & Remove Busy Loop**: Move to `bounded(20)` and use blocking pops.
3. **Address Mutex Contention**: Split the global `STATE` into smaller, task-specific locks.
4. **Fix Vsync Bottleneck**: Make Vsync optional or adaptive to support high-FPS gaming.

### High Priority (Stability & Quality)
5. **Implement GOP-Aware Dropping**: Ensure the jitter buffer only drops up to the next I-frame.
6. **Add `stop_mirror()` FFI**: Ensure all threads and USB handles are cleaned up.
7. **Fix Triple Buffer Safety**: Implement a proper 3-slot acquisition protocol (Dirty/Clean/Busy).

### Medium Priority
8. **Add timestamps to protocol**: Include frame send time in packet header.
9. **Remove Allocation Churn**: Use a pre-allocated buffer pool for frames on both Mobile and Desktop.
10. **Add CRC checksum**: Detect corrupt frames before decoding.

---

## Diagnostic Metrics to Monitor

### Mobile
- `throughput_mbps` - Should be 80-90% of USB bandwidth capacity.
- `encoding_latency_ms` - Target < 50ms for 60fps.
- `alloc_pressure` - (New) Monitor frequency of Vec allocations per second.

### Desktop
- `TOTAL_DROPPED_FRAMES` - Total since session start.
- `decoder_latency_ms` - Time spent in FFmpeg decode + scaling.
- `shm_write_latency_ms` - (New) Time spent copying to OBS/Triple Buffer.
- `queue_backlog` - Current packets waiting in `PREVIEW_QUEUE`.
