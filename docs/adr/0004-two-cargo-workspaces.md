# 0004 — Two Cargo workspaces, not one

## Context

There are three Rust crates that build for the host — `mirror-protocol`,
`mirror-i18n` and `mirror_backend` — and one that cross-compiles to Android,
`rust_lib_stream_mobile_app`.

The obvious move when introducing shared crates is one workspace containing all
four. One `Cargo.lock`, one `target/`, one `cargo test` that runs everything.

That does not work here. `cargo ndk` runs from `mobileApp/rust` and builds the
workspace it finds itself in. With a single workspace that pulls in
`mirror_backend`, and through it `ffmpeg-sys-next`, which does not cross-compile
to Android — the mobile build would fail on a dependency it has no use for.

## Decision

Two workspaces. The root one holds `packages/*` and `desktopApp/mirror_backend`
and excludes `mobileApp/rust`, which declares its own `[workspace]` and reaches
the shared crates by relative path.

## Consequences

Path dependencies work across workspace boundaries, so the sharing that
motivated this is unaffected: the mobile crate still depends on
`mirror-protocol`, and a change to the frame format still fails to compile on
both sides at once.

The mobile build stays fast and does not need FFmpeg present.

The costs:

- Two `Cargo.lock` files. A shared dependency can be at different versions in
  each, and nothing warns about it.
- `cargo test --workspace` at the root does not run the mobile crate's tests.
  CI runs them as a separate step; a contributor has to remember, which is why
  `CONTRIBUTING.md` lists both commands.
- `[workspace.dependencies]` at the root cannot be used by the mobile crate, so
  its versions are written out in full.

If `ffmpeg-sys-next` ever gains a working Android target, or if the backend
stops depending on it, this should be revisited — the single workspace is the
better arrangement whenever it is possible.
