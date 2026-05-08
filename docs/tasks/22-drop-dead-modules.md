# Task T-22 — Drop dead modules + consolidate re-exports
Stage: 2
Status: complete
Depends on: T-13
Blocks:     T-31 (PulseBus replaces EventSink), T-61 (Effect dissolves)

## Goal
Remove modules and `pub use` chains that were only there for the
pre-split crate layout. Reduces noise before Stage 3 lands real
foundation skeletons.

## In scope
- Delete `pub use` re-exports inside `devix-core` that pointed at
  the now-merged `devix-editor`/`devix-plugin`/`devix-panes` paths.
- Delete `widgets/mod.rs` re-export consolidator now that widgets
  live directly under `devix-tui::widgets`.
- Delete the dead `panes/event.rs` module (its `Event` type does not
  survive InputEvent's relocation, but until Stage 4 we keep
  `panes/event.rs`'s import path satisfied via re-export — flip that
  re-export to `pub use crate::input::Event;` if it survived T-12).

## Out of scope
- Rewriting `EventSink`, `Effect`, `Wakeup` (Stage 6).
- Adding new types.

## Files touched
- `crates/devix-core/src/lib.rs` (re-export trim)
- `crates/devix-tui/src/lib.rs` (re-export trim)
- `crates/devix-tui/src/widgets/mod.rs` (delete or shrink)

## Acceptance criteria
- [ ] No `pub use` line points at a path that no longer exists.
- [ ] `cargo build --workspace` passes with zero warnings.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/crates.md` — *File-level migration* (deletions).
