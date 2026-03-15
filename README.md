# ScreenMirror

**ScreenMirror** is a high-performance, low-latency screen and audio mirroring solution designed to stream your Android device's screen to your Desktop (Linux/Windows) over USB. Unlike traditional solutions that rely on ADB or network connections, ScreenMirror utilizes the **Android Open Accessory (AOA) protocol**, providing a robust, driver-less (on Linux), and extremely low-latency connection.

## 🚀 Key Features

- **USB-AOA Protocol**: High-reliability streaming over USB without requiring ADB or developer mode.
- **Ultra-Low Latency**: Optimized pipeline using Rust for core logic and FFmpeg for hardware-accelerated decoding.
- **High-Performance Rendering**: Native GPU rendering on the desktop using **WGPU**.
- **Audio Mirroring**: Real-time audio streaming from Android to Desktop (powered by Oboe).
- **Cross-Platform Receiver**: Desktop application built with **Electrobun** (Bun + React), supporting Linux and Windows.
- **Real-time Diagnostics**: The mobile companion app provides live metrics for FPS, throughput, and encoding latency.
- **Automated Permissions**: Simple setup scripts for Linux udev rules and Windows driver management.

---

## 🏗️ Technical Architecture

### Desktop Receiver (`desktopApp`)
- **Frontend**: React 18, Tailwind CSS, Vite.
- **Runtime**: [Electrobun](https://electrobun.dev/) (A lightweight, high-performance alternative to Electron using Bun).
- **Core Engine (`mirror_backend`)**: 
  - Written in **Rust**.
  - **FFmpeg**: Hardware-accelerated video decoding (H.264/H.265).
  - **WGPU**: Cross-platform, native GPU rendering.
  - **rusb**: High-level USB communication.

### Mobile Companion (`mobileApp`)
- **Frontend**: Flutter.
- **Native Core**: 
  - Written in **Rust** via `flutter_rust_bridge`.
  - **Oboe**: High-performance Android audio capture.
  - **MediaProjection**: Native Android screen capture API.
  - **JNI Bridge**: Seamless integration between Kotlin and Rust.

---

## 🛠️ Getting Started

### 1. Prerequisites

- **Rust**: [Install Rust](https://www.rust-lang.org/tools/install)
- **Bun**: [Install Bun](https://bun.sh/)
- **Flutter**: [Install Flutter](https://docs.flutter.dev/get-started/install)
- **FFmpeg Libraries**: Ensure FFmpeg development headers are installed on your system.

### 2. Desktop Setup (Receiver)

```bash
cd desktopApp

# Install dependencies
bun install

# Build the Rust backend
bun run build:rust

# Start the application in development mode with HMR
bun run dev:hmr
```

#### Linux USB Permissions
If you are on Linux, run the included script to grant your user permission to access USB devices in Accessory mode:
```bash
sudo ./setup_udev.sh
```

### 3. Mobile Setup (Companion)

```bash
cd mobileApp

# Get Flutter dependencies
flutter pub get

# (Optional) Generate Rust bindings if you made changes
flutter_rust_bridge_codegen generate

# Build and run on your Android device
flutter run
```

---

## 📖 Usage

1.  **Open the Desktop App**: Launch the ScreenMirror receiver on your PC.
2.  **Connect Device**: Plug your Android device into your PC via a high-quality USB cable.
3.  **Launch Companion**: Open the **Mirror Companion** app on your Android device.
4.  **Authorize**: Accept the USB connection prompt on your phone.
5.  **Start Mirroring**: The stream should start automatically. You can monitor performance metrics (FPS, Latency, Mbps) directly on the phone's dashboard.

---

## 🤝 Contributing

Contributions are welcome! Whether it's bug fixes, new features, or documentation improvements, please feel free to open a Pull Request.

1.  Fork the Project
2.  Create your Feature Branch (`git checkout -b feature/AmazingFeature`)
3.  Commit your Changes (`git commit -m 'Add some AmazingFeature'`)
4.  Push to the Branch (`git push origin feature/AmazingFeature`)
5.  Open a Pull Request

---

## 📄 License

Distributed under the MIT License. See `LICENSE` for more information. (Note: Ensure you add a LICENSE file if this is a public repo).

---

## 🌟 Acknowledgments

- [Electrobun](https://github.com/mitch-m/electrobun) for the lightweight desktop runtime.
- [flutter_rust_bridge](https://github.com/fzyzcjy/flutter_rust_bridge) for the seamless Flutter/Rust integration.
- [FFmpeg](https://ffmpeg.org/) and [WGPU](https://wgpu.rs/) for the power-efficient media pipeline.
