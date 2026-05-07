# Task T-50 — Migrate buffers onto namespace (`/buf/<id>`)
Stage: 5
Status: pending
Depends on: T-30
Blocks:     T-57

## Goal
Reshape `Document` / `DocStore` so the path-facing id is a
process-monotonic `u64` (slotmap stays internal). `DocStore`
implements `Lookup<Resource = Document>` mounted at `/buf/<id>`.
Add `Document::id_from_path(&Path) -> Option<DocId>`.

## In scope
- `DocId(u64)` minted from a global `AtomicU64` (model on
  `FrameId` in `crates/devix-core/src/editor/frame.rs`). Internal
  slotmap key remains private.
- `DocStore: Lookup<Resource = Document>`: `lookup`, `lookup_mut`,
  `paths()`. `paths()` enumerates `/buf/<id>` for live buffers only.
- `Document::id_from_path(&Path)` per `namespace.md` Q3 (locked).
- Decide `lookup_mut` borrow-check policy (Q1): use direct slotmap
  access on `DocStore` for ops that genuinely need disjoint mutable
  borrows; document this on `DocStore` as comment + lean rule.
- Update every existing `documents.get(DocId)` site to go through
  `Lookup::lookup` where the ergonomic difference is small; keep
  direct slotmap access for split-borrow ops.
- Tests: round-trip path → DocId → path; closed-and-reopened
  buffer mints a new id (per `namespace.md` *Property*).

## Out of scope
- Pulses carrying paths (T-57 finishes the sweep).
- Cursor migration (T-51).
- Globbing / patterns (deferred per `namespace.md` Q2).

## Files touched
- `crates/devix-core/src/document.rs`
- `crates/devix-core/src/editor/**` (call site updates)

## Acceptance criteria
- [ ] `DocStore` implements `Lookup<Resource = Document>`.
- [ ] No code outside `DocStore` references the slotmap key.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/namespace.md` — *Segment encoding rules → Resource
  ids*, *Migration table*, *Open Q1*, *Open Q3*.
- `docs/specs/foundations-review.md` — *Gate T-30*.
