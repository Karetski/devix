# Task T-40 — View IR enum + supporting types
Stage: 4
Status: pending
Depends on: T-30, T-32
Blocks:     T-41, T-43, T-44, T-91

## Goal
Implement the closed `View` enum and its supporting types in
`devix-protocol::view`, per `frontend.md`. After this lands the IR
is constructible, serializable, and ready for the producer (T-43)
and interpreter (T-44).

## In scope
- `View` enum (Empty, Text, Stack, List, Buffer, TabStrip, Sidebar,
  Split, Popup, Modal) with all spec'd fields.
- `ViewNodeId(Path)` (resource-bound or `/synthetic/...`).
- Supporting types: `TextSpan`, `WrapMode`, `Axis`, `SidebarSlot`,
  `TabItem`, `Anchor`, `AnchorEdge`, `PopupChrome`, `GutterMode`,
  `CursorMark`, `SelectionMark`, `TransitionHint`, `TransitionKind`.
- `HighlightSpan` re-export from `devix-syntax`.
- All `Clone + Debug + Serialize + Deserialize`.
- Property: same `ViewNodeId` across two View instances must be
  meaningful as "same logical node" — this is contract-level only;
  enforcement lives in producers.

## Out of scope
- Producing View trees in core (T-43).
- Interpreting View trees in tui (T-44).
- Synthetic-id strategy choice (T-90).
- Theme palette / scope resolution (T-55).

## Files touched
- `crates/devix-protocol/src/view.rs`: full enum + supporting types
- `crates/devix-protocol/src/lib.rs`: re-exports

## Acceptance criteria
- [ ] Every `View` variant constructs and round-trips serde.
- [ ] `ViewNodeId` wraps `Path` and uses its `Hash` impl directly.
- [ ] `Axis` and `SidebarSlot` defined here are referenced from
      `devix-protocol::pulse` (no duplicate definitions).
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/frontend.md` — *View IR*, *Supporting types*,
  *ViewNodeId*.
- `docs/specs/foundations-review.md` — *Vocabulary alignment*.
