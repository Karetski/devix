# Task T-63 — Drop Effect / EventSink / Wakeup; Stage-6 regression gate
Stage: 6
Status: complete
Depends on: T-60, T-61, T-62
Blocks:     all of Stage 7+

## Goal
Now that no producer calls into the legacy event types, delete
them. After this Stage 6 is complete; the bus is the single
event vocabulary in the workspace.

## In scope
- Delete `Effect`, `EventSink`, `DiskSink`, `MsgSink`, `Wakeup`
  types and modules.
- Drop unused channel infrastructure that supported them.
- Final regression: build + test + manual run.

## Out of scope
- New abstractions.

## Files touched
- `crates/devix-core/src/**`: deletions
- `crates/devix-tui/src/**`: deletions

## Acceptance criteria
- [ ] None of `Effect`, `EventSink`, `DiskSink`, `MsgSink`,
      `Wakeup` exist.
- [ ] `cargo build --workspace` passes with zero warnings.
- [ ] `cargo test --workspace` passes.
- [ ] Manual `cargo run --bin devix` opens / edits / saves
      without behavioral change.

## Spec references
- `docs/specs/crates.md` — *crates/app/src/**/* (`effect.rs` →
  dissolved; `event_sink.rs` → dissolved)*.
