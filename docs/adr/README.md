# Architecture decision records

Short notes on choices that were not obvious, kept so the next person does not
have to re-derive the reasoning — or worse, quietly undo it.

Write one when a decision closes off an option someone would reasonably reach
for. Do not write one for a choice the code already explains.

Format: `NNNN-short-title.md`, with **Context**, **Decision**, **Consequences**.
Never delete one. A decision that is later reversed gets a new record marked
`Supersedes NNNN`, and the old one gains a `Superseded by NNNN` line — the
history of what was tried is the useful part.

| # | Decision |
|---|---|
| [0001](0001-aoa-over-adb.md) | Use AOA rather than ADB |
| [0002](0002-ffplay-for-playback.md) | Delegate playback to a child ffplay process |
| [0003](0003-event-codes-not-strings.md) | The backend emits event codes, not sentences |
| [0004](0004-two-cargo-workspaces.md) | Two Cargo workspaces, not one |
