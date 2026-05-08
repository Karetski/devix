# Task T-104 — Stage-10 regression gate
Stage: 10
Status: complete (manual sanity deferred — non-TTY environment)
Depends on: T-100, T-101, T-102, T-103
Blocks:     all of Stage 11+

## Goal
Verify Editor's god-struct is meaningfully decomposed: pane
registry, focus chain, ops, modal slot all live in their own
owners with narrow public surfaces. Editor itself becomes a
coordinator over those four owners + DocStore + CursorStore +
CommandRegistry + Keymap + Theme (already shaped from Stage 5).

## In scope
- Final structural sanity: `Editor` is small enough that its public
  fields/methods can fit on one screen.
- Build + test + manual run.

## Out of scope
- New features.

## Files touched
- (no new code; possibly small cleanups)

## Acceptance criteria
- [x] `Editor` struct has at most ~8 fields, each one a typed owner.
- [x] `cargo build --workspace` passes with zero warnings.
- [x] `cargo test --workspace` passes.
- [ ] Manual: every existing feature works end-to-end.

## Notes (2026-05-07)
- Final `Editor` shape (8 fields, all typed owners): `documents:
  DocStore`, `cursors: CursorStore`, `bus: PulseBus`, `panes:
  PaneRegistry` (T-100), `modal: ModalSlot` (T-103), `focus:
  FocusChain` (T-101), `doc_index: HashMap<PathBuf, DocId>` (path-dedup
  cache), `render_cache: RenderCache`. Ops live as `impl Editor`
  methods that mutate via the typed owners only — no field-pokes (T-102).
- `cargo build --workspace`: zero warnings on a clean rebuild.
  `cargo test --workspace`: 260 tests pass across 14 binaries.
- Manual sanity (interactive TUI: edit, split, sidebar toggle, palette
  open, theme switch end-to-end) deferred — this implementation agent
  runs without a TTY. Recommended to verify locally before opening the
  PR for Stage 10.

## Spec references
- `docs/principles.md` — *Hickey — simple is not easy*.
