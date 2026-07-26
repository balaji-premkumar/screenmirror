# 0003 — The backend emits event codes, not sentences

## Context

The Rust backend's log lines are not developer-only diagnostics. The desktop
app renders them in its activity panel, so a user reads them while trying to
work out why their phone will not connect.

They were built at the call site:

```rust
log_event("ERROR", "USB", "streaming",
          &format!("Open failed: {:?}. Connection abandoned.", e));
```

That compiles the English into the shared library and sends it across the FFI
boundary as finished prose. Translating it would mean translating inside Rust
— at a layer that has no idea what language the user chose, and that would need
rebuilding and reshipping for every new locale.

## Decision

A call site names a stable code and supplies parameters:

```rust
log_event!(codes::USB_STREAMING_OPEN_FAILED, "error" => format!("{e:?}"));
```

The wording lives in `packages/mirror-i18n/catalog/<locale>.json`, along with
the event's severity and component. The interface looks it up in the user's
language.

## Consequences

Adding a language means translating a JSON file. No Rust changes, no rebuild of
the native library.

The catalog is a single file shared by both sides: the Rust crate embeds it
with `include_str!`, and the desktop UI imports the same path through a
`@catalog` alias. They cannot disagree, because there is only one file.

Severity moved out of the call site and into the catalog. A code's severity is
now decided in the same place as its wording, which is where it belongs — the
call site was choosing `WARN` or `ERROR` for the same condition in different
files.

Three tests protect it: a code without a translation fails the build, a catalog
entry nothing can emit fails the build, and a placeholder that is not
`lower_snake_case` fails the build. That last one exists because substitution
is by exact name, so `{Error}` would silently render as literal `{Error}`.

The costs:

- Adding a log line is two edits, not one — the code and the catalog entry.
- Codes are permanent. Rewording is free; renaming is not, because a user's
  saved log and a translator's file both key off the code.
- `message` is still populated in English, because the on-disk log file has to
  be readable when pasted into a bug report, and because an interface running
  against a newer backend needs *something* to show for a code it has never
  heard of.
