# Task T-92 — Move rect caches (frame/sidebar/tabstrip) to devix-tui
Stage: 9
Status: complete — Application owns RenderCache; Editor APIs take it as parameter
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
- [x] Click / hit-test still works pre/post-task identically.
- [x] `cargo build --workspace` passes.
- [x] `cargo test --workspace` passes.

## Notes (2026-05-08) — full carve

- `Editor.render_cache` field retired. The cache is owned by
  `devix-tui::Application.layout_cache` and threaded through every
  consumer.
- `Editor::layout(area, &mut RenderCache)` populates the cache during
  the pre-paint pass; `Application::render` passes its own
  `layout_cache`.
- Editor APIs that consult geometry — `focus_dir`, `focus_at_screen`,
  `tab_strip_hit`, `frame_at_strip`, `tab_strip_can_scroll`,
  `scroll_tab_strip` — take `&RenderCache` as a parameter.
- `commands::Context` gained `layout_cache: &'a RenderCache` so
  commands (e.g. `cmd::FocusDir`, `cmd::ClickAt`) can pass the cache
  through to the editor.
- TUI's `AppContext` carries `&RenderCache` and routes it into
  `EditorContext` on each `run()`.
- `Path`-keying of the cache is *not* performed: the migration table
  notes the cache is "TUI-internal; no path." Keys remain `FrameId`
  / `SidebarSlot` for now; a Path-keyed cache lands when T-95
  retires the legacy direct-paint path and the View interpreter
  becomes the sole rect producer.
- Type definitions (`RenderCache`, `TabStripCache`, `TabHit`) stay in
  `devix-core::editor::editor` to avoid a `devix-core ↔ devix-tui`
  cycle for command-side code that needs to type their parameters.
  Storage / lifecycle is tui-side; that satisfies the spec's
  "owning crate: devix-tui" intent.

## Spec references
- `docs/specs/namespace.md` — *Migration table* (TUI-internal
  caches).
