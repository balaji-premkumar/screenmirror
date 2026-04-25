# Project Issue Tracker & Roadmap 🗺️

This document tracks identified performance bottlenecks, stability issues, and planned architectural improvements for the ScreenMirror pipeline.

## ✅ Resolved Bottlenecks (Recent Fixes)

The following critical issues have been addressed in the latest release:

- **[PERF] O(N²) Demuxer Scanning**: Optimized the magic sequence search from $O(N^2)$ to a fast `memchr`-based scan.
- **[PERF] Decoder Busy Loop**: Replaced the 1ms sleep loop with an event-driven wait/balanced polling strategy.
- **[PERF] Mobile Allocation Churn**: Introduced a `BUFFER_POOL` in the mobile muxer to reuse memory for frames.
- **[STABILITY] GOP-Aware Dropping**: Updated the jitter buffer to prevent visual "smearing" by only dropping up to the next HEVC Keyframe.
- **[STABILITY] Graceful Shutdown**: Implemented `stop_mirror()` FFI and global termination signals for clean resource release.
- **[UX] Vsync Bottleneck**: Disabled mandatory Vsync in the SDL2 renderer to support high-FPS (90/120Hz) mobile gaming.

---

## 🚩 High Priority (Next Steps)

### 1. Unsafe Triple Buffer Implementation
- **Status**: 🟠 In Progress
- **Problem**: The current `shared_mem.rs` lacks proper state tracking (Dirty/Clean/Busy). A fast writer can overwrite a slot while a slow reader (like OBS) is still copying from it.
- **Goal**: Implement a robust 3-slot acquisition protocol to ensure zero-copy safety.

### 2. Global Mutex Contention (STATE)
- **Status**: 🔴 Backlog
- **Problem**: A single global `STATE` mutex in `lib.rs` is held during heavy operations. This can block the USB receiver thread, causing packet backlog.
- **Goal**: Refactor the state into smaller, task-specific atomics or granular locks.

### 3. Missing Source Timestamps
- **Status**: 🔴 Backlog
- **Problem**: The protocol lacks sender-side timestamps. The receiver generates its own, making real jitter correction or A/V sync impossible.
- **Goal**: Add a 64-bit microsecond timestamp to the packet header in `muxer.rs`.

---

## 🟡 Medium Priority

### 4. Brittle Control Command Parsing
- **Status**: 🟡 Planned
- **Location**: `mobileApp/rust/src/usb_loop.rs`
- **Problem**: Control commands are detected via `contains()` on raw strings instead of proper JSON deserialization.
- **Goal**: Implement proper `serde_json` parsing for all AOA control messages.

### 5. Audio Lifecycle Risks
- **Status**: 🟡 Planned
- **Problem**: `AudioCapture` lacks an explicit `stop()` on the Oboe stream, potentially leaving threads dangling during pipeline restarts.
- **Goal**: Implement `Drop` for `AudioCapture` to ensure clean Oboe stream termination.

---

## 🔵 Low Priority / Feature Requests

### 6. Protocol CRC/Checksum
- **Goal**: Add a 32-bit CRC to each packet to detect and discard data corrupted during USB transmission.

### 7. Windows Driver Automation
- **Goal**: Implement the `install_windows_driver` FFI to automate WinUSB setup via Zadig-like backend logic.

---

## 📊 Performance Targets

| Metric | Target | Current |
| :--- | :--- | :--- |
| **End-to-End Latency** | < 40ms | ~60-80ms |
| **USB Throughput** | 150+ Mbps | ~90 Mbps |
| **CPU Usage (Idle)** | < 1% | ~3-5% |
| **Frame Drop Rate** | < 0.1% | ~2% (on 120fps source) |
