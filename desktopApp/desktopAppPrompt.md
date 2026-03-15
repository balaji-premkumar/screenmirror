# Role: Senior Desktop Systems Architect
# Project: Mirroring Receiver & OBS Pipeline (Windows)
# Tech Stack: Electrobun (UI/Bun Process), Rust (High-Speed Engine), Bun FFI

## Context
A high-performance receiver for 2K 60FPS video. Rust handles the USB bulk data and HW decoding; Electrobun provides the control interface.

## Task 1: Scaffolding & Bun FFI
1. Initialize an Electrobun project.
2. Create a Rust crate `mirror_backend` compiled as a `cdylib`.
3. Use `bun:ffi` in the Electrobun main process to load Rust functions with `#[no_mangle]`.

## Task 2: Adaptive Jitter Buffer (Rust)
Write a Rust receiver that:
1. Reads raw H.265/HEVC packets from USB using `rusb`.
2. **Adaptive Pacing:** Implements a `ConcurrentQueue` buffer that dynamically scales between 10ms (low latency) and 50ms (smoothness) based on packet arrival consistency.
3. Integrates `ffmpeg-next` with **NVDEC/DXVA2** for hardware-accelerated decoding.

## Task 3: Invisible OBS Pipeline (Shared Memory)
Implement a Shared Memory producer in Rust:
1. Create a `MemoryMappedFile` with a custom Header: `[Magic: 4b][Width: 4b][Height: 4b][Timestamp: 8b][Data...]`.
2. Write raw YUV/RGB frames directly to this map for an OBS plugin to consume.

## Task 4: Electrobun Control Panel
Build a TypeScript UI that:
- Sends commands to the Rust core via Bun FFI (Change Resolution, Toggle OBS).
- Displays a "Decoder Status" (e.g., "Nvidia NVDEC Active").
- Minimize-to-tray logic for background streaming.
