# Architecture

How the pieces fit, and where to put a new one.

## The shape of it

A phone encodes its screen and audio and pushes them down a USB cable. A
desktop app reads them off the cable and hands them to whichever sink the user
turned on. There is no network anywhere, and no ADB.

```
┌─────────────────────────── Android phone ───────────────────────────┐
│                                                                     │
│  MediaProjection ──► MediaCodec (HEVC) ─┐                           │
│  AudioPlaybackCapture ──► f32 PCM ──────┤                           │
│                                         ▼                           │
│                                    Rust muxer                       │
│                                         │  frames the packets       │
└─────────────────────────────────────────┼───────────────────────────┘
                                          │ USB (AOA accessory)
┌─────────────────────────────────────────┼───────────────────────────┐
│                                         ▼          Desktop          │
│                                pipeline::receiver                   │
│                                         │                           │
│                                pipeline::demuxer                    │
│                          ┌──────────────┴──────────────┐            │
│                       video                         audio           │
│                          │                             │            │
│                   pipeline::decoder            sinks::push_audio    │
│                          │                             │            │
│                          └──────────────┬──────────────┘            │
│                                         ▼                           │
│                         ┌───────────────┴───────────────┐           │
│                  sinks::player                   sinks::obs_feed    │
│                  (child ffplay)                  (shared memory)    │
└─────────────────────────────────────────────────────────────────────┘
```

## Why AOA

Android Open Accessory lets a phone talk to a USB host without developer mode,
without ADB, and without the user enabling anything beyond tapping *allow* on
the accessory prompt. That is the project's reason to exist — scrcpy is better
at almost everything else, and needs ADB.

The cost is that AOA is a plain byte pipe. There are no message boundaries, so
the framing in `packages/mirror-protocol` exists to put them back.

## Repository layout

```
packages/
  mirror-protocol/   the wire format and the HEVC bitstream helpers
  mirror-i18n/       event codes and locale catalogs
desktopApp/
  mirror_backend/    Rust: receive, decode, route
  obs_plugin/        C: the OBS source that reads the shared memory
  src/               TypeScript: the Electrobun shell and the React UI
mobileApp/
  rust/              Rust: capture muxing and the USB write loop
  lib/               Dart: the Flutter UI
  android/           Kotlin: MediaProjection, MediaCodec, the service
docs/                this file, and the decisions behind it
tools/               release packaging
```

### Two things are shared, deliberately

`packages/mirror-protocol` and `packages/mirror-i18n` are the only code both
sides depend on, and each exists because duplicating it had already caused a
problem:

- **The wire format** was written in the mobile muxer and parsed in the desktop
  demuxer, each with its own copy of the magic bytes and type tags. A mismatch
  between them does not fail a build — it stalls the stream at runtime on a
  user's machine. The Annex-B HEVC walker had four copies, which had already
  drifted on how they treated a trailing start code.
- **Event codes** exist so the backend never sends a finished English sentence
  across the FFI boundary. See [i18n.md](i18n.md).

Nothing else is shared. Two crates is the right number until a third has a
reason.

## Desktop backend

```
mirror_backend/src/
├── ffi/         every extern "C" symbol, and nothing else
├── pipeline/    USB in → demux → queue → decode
├── sinks/       ffplay and OBS. Both opt-in.
├── platform/    one file per OS
└── telemetry/   the event log and the counters
```

**`ffi/`** is the whole surface the interface can see. Each function converts
arguments, calls a normal Rust module, and converts back. Keeping it in one
place means the contract can be read in one sitting, and that adding a feature
does not quietly add an exported symbol somewhere else.

**`pipeline/`** runs three long-lived threads: USB discovery, the USB session,
and the decoder. None is joined on shutdown. Instead `SESSION_GEN` is bumped
and each thread notices at the top of its next loop — a thread blocked in a
syscall when stop was called cannot miss that, which a boolean flag that gets
reset can.

**`sinks/`** is where the stream can go, and the grouping exists to make the
rule visible: there are exactly two sinks, and the app opens no audio device of
its own. Sound reaches the machine through the child `ffplay` process or
through the OBS shared-memory feed, and only after the user asks.

**`platform/`** is how multi-OS support stays bounded. Each OS provides four
functions and `mod.rs` picks one at compile time. Adding a platform is: add the
file, add the `cfg_attr` line, implement four functions, and let the compiler
tell you what you missed.

## Desktop interface

Electrobun runs a Bun process and a WebView. They talk over RPC.

```
src/bun/
  native.ts     dlopen, the symbol table, and C-string ownership
  rpc.ts        the handlers
  index.ts      window creation and startup order
src/
  services/rpc.ts   the typed client the React side calls
  features/         one directory per screen, one file per panel
  i18n/             the provider, the UI catalog, and the event bridge
```

`services/rpc.ts` declares the method contract once as `RpcMethods`, and
`bun/rpc.ts` is typed against the same interface — so a handler whose signature
does not match is a compile error on both sides.

Every `*mut c_char` the backend returns is owned by the caller. `native.ts` is
the only file that knows that; it frees in a `finally` so a parse error
downstream cannot leak an allocation on a twice-a-second poll.

## Mobile app

```
lib/
  main.dart              bootstrap
  src/app/               MaterialApp shell and theme
  src/features/mirror/   the screen, its controller, its widgets
  src/services/          platform channel, native bridge, permissions
  src/models/            log entries, connection phases
  src/l10n/              the wording
```

`MirrorController` holds state and ordering rules; widgets render what it
exposes. The services are injectable, which is what makes the connection
sequence testable without a phone attached.

`MirrorPhase` separates *the cable is connected* from *the user approved
capture*. Those are two different things, and collapsing them into one boolean
is how an earlier version came to display "Streaming to PC" while the consent
dialog was still on screen.

## Threads and ownership, in one table

| Thread | Owns | Ends when |
|---|---|---|
| Desktop discovery | the libusb context for scanning | `SESSION_GEN` changes |
| Desktop session | the claimed interface, the demuxer | `SESSION_GEN` changes, or a fatal read |
| Desktop decoder | the FFmpeg decoder, the frame pool | `SESSION_GEN` changes |
| Desktop log writer | the log file handle | the channel closes |
| Mobile encoder drain | the MediaCodec output buffers | the projection stops |
| Mobile USB writer | the accessory file descriptor | the queue closes |

## Where to add things

| You want to | Put it in |
|---|---|
| Change the frame format | `packages/mirror-protocol`, and bump `PROTOCOL_VERSION` |
| Add a backend log message | `packages/mirror-i18n` (a code and a catalog entry) |
| Support another OS | `desktopApp/mirror_backend/src/platform/<os>.rs` |
| Add a way to play the stream | `desktopApp/mirror_backend/src/sinks/` |
| Add a backend call the UI makes | `ffi/mod.rs`, `bun/native.ts`, `bun/rpc.ts`, `services/rpc.ts` |
| Add a desktop UI panel | `desktopApp/src/features/<feature>/` |
| Add a language | `docs/i18n.md` explains the four files |

## Known gaps

- Latency is measured from *arrival*, not from capture. Honest end-to-end
  latency needs a sender-side timestamp on the wire, which the frame format
  does not carry yet.
- The OBS plugin is built and tested on Linux. The CMake file supports Windows
  and macOS but neither is exercised in CI.
- iOS is not supported and is unlikely to be: it has no equivalent of AOA.
