# ScreenMirror 📱💻

[![CI - Desktop](https://github.com/balaji-premkumar/screenmirror/actions/workflows/desktop.yml/badge.svg)](https://github.com/balaji-premkumar/screenmirror/actions/workflows/desktop.yml)
[![CI - Mobile](https://github.com/balaji-premkumar/screenmirror/actions/workflows/mobile.yml/badge.svg)](https://github.com/balaji-premkumar/screenmirror/actions/workflows/mobile.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**ScreenMirror** streams your Android device's screen and system audio to a desktop (Linux/Windows/macOS) over a plain USB cable, with low latency and no network involved.

It uses the **Android Open Accessory (AOA) protocol** rather than ADB. That means **no Developer Options, no USB debugging, and no `adb` authorization prompt** — plug the cable in, tap the accessory dialog on the phone, and the stream starts. This is the main thing that distinguishes it from ADB-based tools.

> **Status: pre-release.** The pipeline works and is covered by tests, but it has not been validated on a wide range of hardware. Expect rough edges, and please [open an issue](https://github.com/balaji-premkumar/screenmirror/issues/new/choose) when you hit one.

---

## ✨ Features

- **🔗 No ADB, no developer mode** — pure USB-AOA. USB permissions are configured on first launch (udev on Linux, WinUSB on Windows).
- **🚀 Hardware-accelerated decode** — FFmpeg with VAAPI/NVDEC (Linux), D3D11VA/DXVA2 (Windows), VideoToolbox (macOS), and a multithreaded software fallback.
- **🔊 System audio** — real-time game/media audio via Android `AudioPlaybackCapture` (48 kHz f32 mono). Captures what the device is *playing*, not the microphone.
- **🎥 OBS Studio integration** — a shared-memory source plugin, so no capture card and no window-capture round trip.
- **▶️ Built-in playback** — a player window with video *and* sound, driven by `ffplay`.
- **📈 Live diagnostics** — FPS, throughput, dropped frames and buffer health in the dashboard.

---

## 🔈 How playback works

The desktop app **never opens an audio device or creates a video window itself.** Audio and video are only sent to a sink you explicitly turn on:

| Sink | How to enable | What it does |
| :--- | :--- | :--- |
| **Player** | *Play Video + Audio* button | Remuxes the incoming HEVC + PCM into Matroska and pipes it to a child `ffplay`, which owns the window and the sound. |
| **OBS** | *Send to OBS* toggle | Writes decoded frames to shared memory and audio to a ring buffer for the OBS source plugin. |

With neither enabled, incoming audio is discarded. Nothing plays until you ask for it.

The OBS toggle only becomes available once the plugin is installed **and** OBS is actually running — otherwise it tells you which of the two is missing rather than feeding a reader that does not exist.

---

## 🏗️ Technical Architecture

```
Android                                    Desktop
────────────────────────────────────────   ──────────────────────────────────────
MediaProjection ─→ MediaCodec (HEVC) ─┐
AudioPlaybackCapture (f32 48kHz) ─────┤
                                      ▼
                              Muxer + USB queues
                                      │  AOA bulk transfer
                                      ▼
                                          Demuxer ─┬─→ ffplay (Matroska remux)
                                                   └─→ decoder ─→ OBS shared memory
```

### Desktop Receiver (`/desktopApp`)
- **Frontend**: React 18 + Tailwind CSS on [Electrobun](https://electrobun.dev/) (Bun-based native shell).
- **Core (`mirror_backend`, Rust)**: packet demuxing, transport queues, FFmpeg HEVC decoding, and the Matroska remuxer that feeds `ffplay`.
- **Shared memory**: triple-buffered with a per-slot seqlock for the OBS plugin; a separate ring buffer carries audio.

### Mobile Companion (`/mobileApp`)
- **Frontend**: Flutter.
- **Native core (Rust)**: frame muxing, non-blocking transport queues, AOA session management.
- **Capture**: `MediaProjection` + `MediaCodec` for HEVC, `AudioPlaybackCapture` for system audio.

### OBS Plugin (`/desktopApp/obs_plugin`)
C source building on Linux, Windows and macOS. Reads the shared-memory segments directly.

---

## 🛠️ Getting Started

### Prerequisites

| Tool | Requirement | Notes |
| :--- | :--- | :--- |
| **Rust** | Stable 1.75+ | |
| **Bun** | v1.0+ | Desktop app runtime |
| **Flutter** | 3.44+ | `pubspec.yaml` requires Dart `^3.10.4`; older Flutter will not resolve |
| **FFmpeg** | **v8.x** with dev headers | `ffmpeg-next` is pinned to `8.1`. On a distro shipping FFmpeg 7, pin it back to `7.1` in `mirror_backend/Cargo.toml` |
| **ffplay** | any recent build | Required for the player. Ships with FFmpeg; the app also looks in `desktopApp/bin/` |
| **clang** | | `ffmpeg-sys-next` generates bindings with bindgen at build time |

SDL2 and ALSA are **not** required. Earlier versions embedded an SDL preview window and played audio through cpal; both were removed when playback moved to `ffplay`.

On Debian/Ubuntu:

```bash
sudo apt install libavcodec-dev libavformat-dev libavutil-dev libswscale-dev \
                 clang libclang-dev libusb-1.0-0-dev libudev-dev ffmpeg
```

### Installation

#### 1. Clone
```bash
git clone https://github.com/balaji-premkumar/screenmirror.git
cd screenmirror
```

#### 2. Desktop (receiver)
```bash
cd desktopApp
bun install
bun run build:rust   # compiles the Rust backend
bun run dev          # starts the app in dev mode
```

#### 3. Mobile (companion)
```bash
cd mobileApp
flutter pub get
flutter run --release   # release is strongly recommended for performance
```

The Android build produces `arm64-v8a`, `armeabi-v7a` and `x86_64`. To build a single ABI while iterating:

```bash
flutter build apk --release -PmirrorAbis=arm64-v8a
```

Release builds are signed with the debug key unless you provide `mobileApp/android/key.properties`:

```properties
storeFile=/absolute/path/to/upload-keystore.jks
storePassword=...
keyAlias=upload
keyPassword=...
```

A debug-signed APK must not be distributed — the debug key is public.

---

## 📖 Usage

1. **Launch** the ScreenMirror desktop app.
2. **Connect** your Android device by USB.
3. **Open** the ScreenMirror companion app on the phone.
4. **Authorize** the USB accessory and screen-recording prompts. Grant the microphone permission too if you want sound — it gates `AudioPlaybackCapture`, which records device output rather than the mic.
5. **Press Start** in the desktop dashboard to begin capture.
6. **Choose a sink**: *Play Video + Audio* for a player window, or *Send to OBS* if OBS is running with the plugin installed.

---

## 🐞 Troubleshooting

| Symptom | Likely cause |
| :--- | :--- |
| Phone never appears | udev rules not installed (Linux) or WinUSB not bound (Windows). Use **Fix USB Permissions** in the dashboard. |
| Player button missing | `ffplay` not found on `PATH` or in `desktopApp/bin/`. |
| *OBS: Not Running* / *Plugin Missing* | Start OBS, or install the plugin from the loader screen. |
| Video but no sound | Microphone permission denied on the phone — `AudioPlaybackCapture` needs it even though it captures device output. |
| Build fails on `limits.h` | `clang` is missing; `libclang` alone is not enough for bindgen. |

Desktop logs are written to `~/.mirror_stream/logs/mirror_rust.log.json`.

---

## 🤝 Contributing

Contributions welcome. [ISSUES.md](ISSUES.md) tracks the roadmap and known bottlenecks; GitHub Issues tracks actionable work.

1. Fork the project.
2. Create your branch (`git checkout -b feat/amazing-feature`).
3. Commit (`git commit -m 'feat: add amazing feature'`).
4. Push and open a Pull Request.

Please run the test suites before opening a PR:

```bash
cd desktopApp/mirror_backend && cargo test --release
cd mobileApp/rust && cargo test
cd mobileApp && flutter analyze
```

---

## 📄 License

Distributed under the MIT License. See [LICENSE](LICENSE).

---

## 🌟 Acknowledgments

- [Electrobun](https://electrobun.dev/) for the native runtime.
- [ffmpeg-next](https://github.com/zmwangx/rust-ffmpeg) for the Rust media bindings.
- [FFmpeg](https://ffmpeg.org/) for decoding, remuxing and `ffplay`.
- [OBS Studio](https://obsproject.com/) for the plugin API.
