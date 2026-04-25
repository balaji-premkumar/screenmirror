# ScreenMirror 📱💻

[![CI - Desktop](https://github.com/balaji-premkumar/screenmirror/actions/workflows/desktop.yml/badge.svg)](https://github.com/balaji-premkumar/screenmirror/actions/workflows/desktop.yml)
[![CI - Mobile](https://github.com/balaji-premkumar/screenmirror/actions/workflows/mobile.yml/badge.svg)](https://github.com/balaji-premkumar/screenmirror/actions/workflows/mobile.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**ScreenMirror** is a high-performance, ultra-low-latency screen and audio mirroring solution. It allows you to stream your Android device's screen and system audio to your Desktop (Linux/Windows/macOS) over a standard USB cable.

By leveraging the **Android Open Accessory (AOA) protocol**, ScreenMirror provides a robust, "driver-less" connection that bypasses the limitations and overhead of ADB, providing a near-native experience for gaming and high-fidelity monitoring.

---

## ✨ Features

- **🚀 Ultra-Low Latency**: End-to-end latency optimized via Rust core and FFmpeg hardware acceleration.
- **🔗 USB-AOA Protocol**: Pure USB streaming without requiring ADB, Developer Options (after initial setup), or network dependency.
- **🔊 Audio Mirroring**: Real-time system audio streaming powered by high-performance Oboe (Android) and CPAL (Desktop).
- **🎥 OBS Studio Integration**: Direct-to-SHM feed for OBS, enabling high-quality recording and streaming without capture cards.
- **📈 Live Diagnostics**: Real-time telemetry including FPS, throughput (Mbps), and decoder health.
- **🖥️ Cross-Platform**: Native desktop receiver built with Electrobun (Bun + React) supporting Linux, Windows, and macOS.

---

## 🏗️ Technical Architecture

### Desktop Receiver (`/desktopApp`)
- **Frontend**: React 18 + Tailwind CSS.
- **Runtime**: [Electrobun](https://electrobun.dev/) (High-performance Bun-based native shell).
- **Core (`mirror_backend`)**: 
  - **Rust**: High-concurrency packet processing and demuxing.
  - **FFmpeg**: Hardware-accelerated H.265/HEVC decoding.
  - **SDL2**: Low-latency native preview window.
  - **Shared Memory**: Triple-buffered seqlock for zero-copy OBS integration.

### Mobile Companion (`/mobileApp`)
- **Frontend**: Flutter.
- **Native Core**: 
  - **Rust**: Frame muxing and AOA protocol management.
  - **Oboe**: Low-latency C++ audio capture engine.
  - **MediaProjection**: High-speed screen capture API.

---

## 🛠️ Getting Started

### Prerequisites

| Tool | Requirement |
| :--- | :--- |
| **Rust** | Stable 1.75+ |
| **Bun** | v1.0+ |
| **Flutter** | 3.16+ |
| **FFmpeg** | v6.0+ with development headers (`libavcodec`, `libavutil`, etc.) |
| **SDL2** | Development headers for native preview |

### Installation

#### 1. Clone the repository
```bash
git clone https://github.com/balaji-premkumar/screenmirror.git
cd screenmirror
```

#### 2. Desktop Setup (Receiver)
```bash
cd desktopApp
bun install
bun run build:rust  # Compiles the Rust backend
bun run dev         # Starts the app in dev mode
```

#### 3. Mobile Setup (Companion)
```bash
cd mobileApp
flutter pub get
flutter run --release # Recommended to run in release for performance
```

---

## 📖 Usage

1.  **Launch** the ScreenMirror Desktop application.
2.  **Connect** your Android device via USB.
3.  **Open** the ScreenMirror Companion app on your phone.
4.  **Authorize**: Grant the USB Accessory and Screen Recording permissions when prompted.
5.  **Enjoy**: The stream will automatically initialize. Use the **Dashboard** to monitor performance.

---

## 🤝 Contributing

We welcome contributions! Please see our [ISSUES.md](ISSUES.md) for current bottlenecks and planned features.

1.  Fork the Project.
2.  Create your Feature Branch (`git checkout -b feature/AmazingFeature`).
3.  Commit your Changes (`git commit -m 'feat: add some AmazingFeature'`).
4.  Push to the Branch (`git push origin feature/AmazingFeature`).
5.  Open a Pull Request.

---

## 📄 License

Distributed under the MIT License. See `LICENSE` for more information.

---

## 🌟 Acknowledgments

- [Electrobun](https://github.com/mitch-m/electrobun) for the native runtime.
- [ffmpeg-next](https://github.com/zmwangx/rust-ffmpeg) for the Rust media bindings.
- [Oboe](https://github.com/google/oboe) for high-performance Android audio.
