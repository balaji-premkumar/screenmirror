<!--
Thanks for contributing. Keep this short — a couple of sentences per section
is plenty. Delete anything that does not apply.
-->

## What this changes

<!-- And why. If it fixes an issue: "Fixes #123" -->

## How it was verified

<!--
Say what you actually ran, not what should pass. If something could not be
tested (no phone, no Windows box, no GPU), say so plainly — that is useful
information, not a failing.
-->

- [ ] `cd desktopApp/mirror_backend && cargo test --release`
- [ ] `cd mobileApp/rust && cargo test`
- [ ] `cd mobileApp && flutter analyze`
- [ ] `cd desktopApp && npx tsc --noEmit`
- [ ] Tested against a real phone

## Notes for the reviewer

<!--
Anything non-obvious: a tradeoff you made, something you deliberately left
out of scope, or a part you are unsure about.
-->

<!--
If you touched the shared-memory layout in `shared_mem.rs` or `obs_feed.rs`,
the matching structs in `obs_plugin/mirror_source.c` must change with it, and
`PLUGIN_VERSION` needs bumping so stale plugin binaries get replaced.
-->
