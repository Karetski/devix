# Task T-51 — Migrate cursors onto namespace (`/cur/<id>`)
Stage: 5
Status: complete
Depends on: T-30, T-50
Blocks:     T-57

## Goal
Reshape `Cursor` / `CursorStore` along the same pattern as T-50:
process-monotonic `CursorId(u64)`, `Lookup<Resource = Cursor>`,
`Cursor::id_from_path`.

## In scope
- `CursorId(u64)` minted from `AtomicU64`.
- `CursorStore: Lookup<Resource = Cursor>`.
- `Cursor::id_from_path(&Path)`.
- Update existing `cursors.get(CursorId)` sites.

## Out of scope
- Same as T-50 (deferred to T-57).

## Files touched
- `crates/devix-core/src/cursor.rs`
- `crates/devix-core/src/editor/**` (call site updates)

## Acceptance criteria
- [ ] `CursorStore` implements `Lookup<Resource = Cursor>`.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/namespace.md` — *Migration table* row for cursors.
