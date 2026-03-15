# Role: Senior Mobile Systems Engineer
# Project: Open-Source Mirroring Companion (Android)
# Tech Stack: Flutter (UI), Rust (AOA/Command Logic), Java (MediaCodec/Capture)

## Context
Building a high-performance 2K 60FPS mirroring sender. We use the `flutter_rust_bridge` (FRB) to link the UI to a Rust core, while Java handles the native MediaProjection surface.

## Task 1: Project Scaffolding
Please generate/refine the project structure:
1. Configure `Cargo.toml` with `aoa`, `rusb`, and `anyhow`.
2. Set up a Foreground Service in Java with `mediaProjection` type.
3. Establish the FRB bridge to pass configuration (Bitrate, FPS) from Dart to Rust.

## Task 2: Robust AOA Handshake (Rust Core)
Implement a Rust module `aoa_engine` that:
1. Performs the AOA handshake (Requests 51-53).
2. **Error Recovery:** Implements a retry loop that attempts to re-establish the AOA session if the USB connection is interrupted.
3. Listens for an initial 'Config Packet' from the PC to set encoder parameters.

## Task 3: Adaptive Buffer & Zero-Copy (Java/C++ Integration)
Refine the encoding pipeline:
1. Use `MediaCodec` with Surface input for zero-copy capturing.
2. **The Fix:** Implement a non-blocking `CircularBuffer` for the USB output. 
3. **Policy:** If the USB `OutputStream` blocks (PC is slow), discard the current frame immediately to maintain real-time sync for games like PUBG.

## Task 4: Flutter Dashboard
Create a UI that allows:
- Real-time Bitrate/FPS selection.
- A "Gaming Mode" toggle that tells Rust to prioritize frame-dropping over quality.
