# Contributing

## Getting a build

You need Rust (stable), Bun, Flutter, and the Android NDK. On Linux you also
need FFmpeg development headers and, if you want the OBS plugin, `libobs-dev`.

```bash
# Ubuntu / Debian
sudo apt install build-essential clang pkg-config \
    libavcodec-dev libavformat-dev libswscale-dev libavutil-dev \
    libwebkit2gtk-4.1-dev libobs-dev

# macOS
brew install ffmpeg pkg-config obs
```

Then:

```bash
cargo test --workspace        # the Rust crates
cd desktopApp && bun install && bun run build:all
cd mobileApp && flutter pub get && flutter build apk --debug
```

`-PmirrorAbis=arm64-v8a` restricts the Android build to one architecture, which
is roughly three times faster and enough for a device you actually own.

## Before opening a pull request

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
(cd mobileApp/rust && cargo test)

(cd desktopApp && bun run typecheck)
(cd mobileApp && flutter analyze && flutter test)
```

All of it is expected to be clean. If something is failing on `master`, say so
in the pull request rather than working around it.

## Where things go

[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) has a table at the end mapping
"you want to do X" to the directory it belongs in. The short version:

- Changing the frame format means `packages/mirror-protocol`, and bumping
  `PROTOCOL_VERSION`. Both ends depend on that crate for a reason — please do
  not add a second copy of the framing to either side.
- Adding a backend log message means an event code and a catalog entry in
  `packages/mirror-i18n`. Do not `format!` English at the call site; the
  interface has to be able to translate it. See [docs/i18n.md](docs/i18n.md).
- Supporting another OS means one file in
  `desktopApp/mirror_backend/src/platform/`, not `#[cfg]` blocks spread through
  the code that calls it.

## Style

**Rust** is `rustfmt` with the settings in `rustfmt.toml`, and `clippy` clean.

**TypeScript** is strict mode with no `any` at module boundaries. If you find
yourself reaching for `@ts-ignore`, the type is probably wrong somewhere it can
be fixed.

**Dart** follows `flutter_lints`.

**Comments** should say why, not what. The code already says what it does; a
comment earns its place by recording something the next reader could not
reconstruct — a constraint, a failure it prevents, an option that was tried and
did not work.

## Tests

Test the thing that broke. The existing tests are a reasonable guide to what
that means here: a frame split across USB reads one byte at a time, a corrupt
header followed by a valid frame, a cable pulled without a detach broadcast, a
capture request that is never approved.

Hardware-dependent paths cannot be unit-tested, and pretending otherwise with
heavy mocking is worse than leaving them uncovered. Say what you tested by hand
in the pull request instead.

## Commits

Conventional commits — `feat:`, `fix:`, `refactor:`, `docs:`, `chore:` — with a
scope where it helps (`fix(mobile):`).

Explain why in the body. A message that says what the diff already shows is a
message nobody will need to read again; one that says which failure it prevents
is one someone will thank you for in a year.

## Reporting a bug

Use the issue template. Phone model, Android version and host OS are asked for
because most of what goes wrong here is specific to one of the three — a
MediaCodec quirk on a particular chipset, a permission change in an Android
release, or a USB stack difference between operating systems.

Attach `~/.mirror_stream/logs/mirror_rust.log.json` if the desktop side is
involved. It is JSON Lines, and it is written in English regardless of your
language setting, which is deliberate.
