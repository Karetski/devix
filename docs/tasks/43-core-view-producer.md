# Task T-43 — Core View producer (`Editor::view(root: Path) -> View`)
Stage: 4
Status: complete
Depends on: T-40, T-41, T-42, T-32
Blocks:     T-44, T-91

## Goal
Add a View producer in `devix-core::editor` answering
`Request::View { root }` with a typed View tree. Uses the existing
layout/focus/state model; emits `View::Buffer`, `View::TabStrip`,
`View::Sidebar`, `View::Split`, `View::Modal` per current behavior
mapped onto the IR. Coexists with the current ratatui paint path.

## In scope
- Method `Editor::view(&self, root: Path) -> Result<View, RequestError>`.
- For each layout-tree variant produce the matching `View::*`.
- Resource-bound `ViewNodeId`s use `/buf/<id>`, `/pane/<i>(/<j>)*`,
  `/pane/.../tabstrip`, `/pane/.../sidebar/<slot>`. Synthetic ids
  are out of scope (Stage 9).
- Highlights ship as scope names (locked: not pre-resolved per
  `frontend.md` *Resolved during initial review*).
- `transition` always `None` until T-90 picks an animation strategy.
- Wire `Request::View` through the in-process `CoreHandle`
  scaffolding from T-32 (returns `Response::View`).
- Tests: empty editor → `View::Empty`; one buffer in one frame →
  Stack of TabStrip + Buffer with correct paths.

## Out of scope
- Implementing animation hints / Capability::Animations (T-90+).
- Replacing the ratatui path in tui (T-44).
- Synthetic-id strategy (T-90).

## Files touched
- `crates/devix-core/src/editor/view.rs`: new
- `crates/devix-core/src/editor/mod.rs`: re-export
- `crates/devix-core/src/core.rs`: handle Request::View

## Acceptance criteria
- [ ] `Editor::view(Path::parse("/pane")?)` returns a View tree
      that's structurally consistent with the live editor state.
- [ ] All resource-bound ids match the canonical path roots from
      `namespace.md`.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/frontend.md` — *View IR*, *Buffer rendering specifics*.
- `docs/specs/protocol.md` — *Request*, *Response*, *ViewResponse*.
- `docs/specs/namespace.md` — *Canonical resource roots*.
