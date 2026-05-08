# Task T-92 — Move rect caches (frame/sidebar/tabstrip) to devix-tui
Stage: 9
Status: deferred — depends on T-91 LayoutNode collapse
Depends on: T-91, T-44
Blocks:     T-95

## Goal
Move `frame_rects` / `sidebar_rects` / `tab_strips` from
`devix-core::RenderCache` into `devix-tui` per
`namespace.md` *Migration table*. The TUI client maintains its
own rect cache keyed by `Path`, not by `FrameId` / `SidebarSlot`.

## In scope
- New module `crates/devix-tui/src/render_cache.rs`: `Path`-keyed
  rect cache populated during interpreter walk.
- Hit-test helpers (clicks, drag-and-drop start) use this cache
  instead of asking core.
- Core's `RenderCache` shrinks or disappears entirely.

## Out of scope
- New widget kinds.
- Animation diff cache.

## Files touched
- `crates/devix-core/src/editor/...`: drop the moved fields
- `crates/devix-tui/src/render_cache.rs`: new
- `crates/devix-tui/src/interpreter.rs`: populate cache during walk

## Acceptance criteria
- [ ] Click / hit-test still works pre/post-task identically.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/namespace.md` — *Migration table* (TUI-internal
  caches).
