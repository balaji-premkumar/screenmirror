# ScreenMirror 📱💻

[![CI - Desktop](https://github.com/balaji-premkumar/screenmirror/actions/workflows/desktop.yml/badge.svg)](https://github.com/balaji-premkumar/screenmirror/actions/workflows/desktop.yml)
[![CI - Mobile](https://github.com/balaji-premkumar/screenmirror/actions/workflows/mobile.yml/badge.svg)](https://github.com/balaji-premkumar/screenmirror/actions/workflows/mobile.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**ScreenMirror** streams your Android device's screen and system audio to a desktop (Linux/Windows/macOS) over a plain USB cable, with low latency and no network involved.

It uses the **Android Open Accessory (AOA) protocol** rather than ADB. That means **no Developer Options, no USB debugging, and no `adb` authorization prompt** — plug the cable in, tap the accessory dialog on the phone, and the stream starts. This is the main thing that distinguishes it from ADB-based tools.

> **Status: pre-release.** The pipeline works and is covered by tests, but it has not been validated on a wide range of hardware. Expect rough edges, and please [open an issue](https://github.com/balaji-premkumar/screenmirror/issues/new/choose) when you hit one.

---

## ✨ Features

- **🔗 No ADB, no developer mode** — pure USB-AOA. USB permissions are configured on first launch (udev on Linux, WinUSB on Windows, nothing on macOS). On Windows, plug the phone in first: the one-time setup binds the driver to your phone's own USB mode as well as to the accessory mode it switches into.
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

### Repository layout

```
packages/
  mirror-protocol/   the wire format and HEVC helpers, shared by both ends
  mirror-i18n/       event codes and locale catalogs, shared by both ends
desktopApp/
  mirror_backend/    Rust: ffi/ pipeline/ sinks/ platform/ telemetry/
  obs_plugin/        C: the OBS source that maps the shared memory
  src/               TypeScript: the Electrobun shell and the React UI
mobileApp/
  rust/              Rust: capture muxing and the USB write loop
  lib/               Dart: app/ features/ services/ models/ l10n/
  android/           Kotlin: MediaProjection, MediaCodec, the service
docs/                architecture notes and decision records
tools/               release packaging
```

The two `packages/` crates are the only code both sides share, and each exists
because duplicating it had already caused a problem — the frame format was
written in one place and parsed in another with no compiler link between them,
and the HEVC bitstream walker had four copies that had already drifted.

**[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)** has the detail, including a
table mapping "I want to add X" to the directory it belongs in.
**[docs/adr/](docs/adr/)** records the decisions that were not obvious.

---

## 📦 Download a build

Every push builds installable packages for all three systems. They are not published anywhere yet: open the workflow run, and the packages are listed under **Artifacts** at the bottom of the page (kept for 14 days).

1. Open the [**Build Desktop App**](https://github.com/balaji-premkumar/screenmirror/actions/workflows/desktop.yml) and [**Build Mobile App**](https://github.com/balaji-premkumar/screenmirror/actions/workflows/mobile.yml) workflows.
2. Pick the latest run for your branch.
3. Download the package for your system. You need to be signed in to GitHub to download artifacts.

| Artifact | For | Notes |
| :--- | :--- | :--- |
| `ScreenMirror-linux-x64` | Linux desktop | Extract the `…-Setup.tar.gz` and run the installer. Needs the FFmpeg runtime libraries and `ffplay` from your package manager; the package is built on Ubuntu, so other distributions may need a different FFmpeg version. |
| `ScreenMirror-windows-x64` | Windows desktop | Bundles FFmpeg and `ffplay`. The first run asks for administrator approval once to install the USB driver; **plug your phone in before you approve it**. |
| `ScreenMirror-macos-arm64` | Apple-silicon Mac | Needs `brew install ffmpeg`. Not signed or notarized yet, so macOS will warn the first time: right-click the app and choose Open. |
| `ScreenMirror-android-debug-signed` | Android phone | Signed with the public debug key, so it is for trying the app only. Allow installs from unknown sources. |

The phone app and the desktop app must come from the **same build**. The wire format changed in this version (frames now carry a capture timestamp), and an older phone app will not pair with a newer desktop app.

Tagged releases build the same packages on the `stable` channel, require a real Android signing key, and attach everything to a GitHub release. Publishing to package registries is not set up yet.

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
bun run build:all    # Rust backend, OBS plugin, native bundle, renderer
bun run dev          # starts the app in dev mode
```

`build:all` needs `libobs-dev` for the OBS plugin step. Without it, use
`bun run build:app` (which is also what `bun start` runs) — the
desktop app runs fine without the plugin, and the *Send to OBS* control simply
stays unavailable.

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

1. **Launch** the ScreenMirror desktop app. The first launch checks USB access, the player and OBS, then opens the dashboard by itself when everything is fine.
2. **Plug in** your Android phone with a USB cable. There is nothing to turn on first: no developer options and no USB debugging.
3. **Connect** when the dashboard shows your phone. If the phone asks which app to open, choose ScreenMirror.
4. **Pick a quality** preset (Light, Balanced, Sharp or Maximum), or open *Fine-tune* for resolution, frame rate, bitrate and audio source.
5. **Press Start mirroring**, then approve the screen-sharing prompt on the phone. Grant the microphone permission too if you want sound: it gates `AudioPlaybackCapture`, which records device output rather than the mic.
6. **Watch**: *Watch on this computer* opens a player window, and *Send to OBS* feeds the OBS source if OBS is running with the plugin installed.

While it runs, the dashboard says in one line whether the stream is smooth, uneven, lagging or dropping frames, with the numbers underneath (frame rate, data rate, extra delay) and a suggestion when it is not smooth.

---

## 🐞 Troubleshooting

| Symptom | Likely cause |
| :--- | :--- |
| Phone never appears | USB access not set up. Use **Allow USB access** in the dashboard: it installs a udev rule on Linux, or binds WinUSB on Windows (plug the phone in first). macOS needs nothing. After updating, the dashboard may ask once more, because the Linux rule is versioned and the Windows driver now also covers the phone's own USB mode. |
| Phone and desktop will not pair | The phone app and desktop app are different versions. Install both from the same build. |
| Video stutters while gaming | Try the **Light** preset, a different cable or USB port, and watch *Extra delay*: if it climbs, the link or the phone cannot keep up. |
| Player button missing | `ffplay` not found on `PATH` or in `desktopApp/bin/`. |
| *OBS: Not Running* / *Plugin Missing* | Start OBS, or install the plugin from the loader screen. |
| Video but no sound | Microphone permission denied on the phone — `AudioPlaybackCapture` needs it even though it captures device output. |
| Build fails on `limits.h` | `clang` is missing; `libclang` alone is not enough for bindgen. |

Desktop logs are written to `~/.mirror_stream/logs/mirror_rust.log.json`.

---

## 🤝 Contributing

Contributions welcome — see **[CONTRIBUTING.md](CONTRIBUTING.md)** for the
setup, the checks, and where things go.

The short version:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
(cd mobileApp/rust && cargo test)
(cd desktopApp && bun run typecheck)
(cd mobileApp && flutter analyze && flutter test)
```

Adding a language does not require touching any of that — see
**[docs/i18n.md](docs/i18n.md)**. It is four JSON/ARB/XML files, and the tests
will tell you if you missed a string.

---

## 📄 License

Distributed under the MIT License. See [LICENSE](LICENSE).

### Third-party components

ScreenMirror itself is MIT, but it builds against FFmpeg, which is **LGPL 2.1+** (or GPL, if your FFmpeg was configured with `--enable-gpl`). Two things follow from that:

- The Rust backend **dynamically links** the system FFmpeg libraries, and `ffplay` is launched as a **separate process**. Neither imposes GPL terms on this project.
- If you redistribute a build that **statically links** FFmpeg, or that bundles a GPL-configured FFmpeg, those terms apply to what you ship. Check before packaging binaries.

Windows builds also bundle `wdi-simple.exe` from [libwdi](https://github.com/pbatard/libwdi) (**LGPL 3**), which installs the WinUSB driver. It runs as a separate process and is built unmodified from the upstream `v1.5.1` source in CI; the corresponding source is that tag.

The OBS plugin links `libobs` (GPL 2.0+), so that plugin binary is subject to GPL terms when distributed. The plugin is a separate artifact from the desktop app.

---

## 🌟 Acknowledgments

- [Electrobun](https://electrobun.dev/) for the native runtime.
- [ffmpeg-next](https://github.com/zmwangx/rust-ffmpeg) for the Rust media bindings.
- [FFmpeg](https://ffmpeg.org/) for decoding, remuxing and `ffplay`.
- [OBS Studio](https://obsproject.com/) for the plugin API.
