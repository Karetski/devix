# Reorg prompts

Two prompts, in order. The first lands the architectural fix that everything else depends on; the second proposes the crate reorg that becomes cheap once the first is done.

Each prompt is self-contained — paste it into a fresh session.

---

## Prompt 1 — Make the root pane tree include sidebars

You're working in `/Users/karetski/Developer/devix`, a Rust TUI editor. Read `ARCHITECTURE-REFACTOR.md` first — it states the design intent, then it lies about whether the code matches it.

**The architectural problem.** The doc says "everything is a Pane in one tree." The code has two trees:

1. `Surface.root: Box<dyn Pane>` (in `crates/surface/src/surface.rs`) — contains only the editor splits/tabs subtree. **Sidebars are not in it.**
2. `crates/app/src/render.rs` — every render frame, this binary-level module hand-composes the outer `[sidebar][editor area][sidebar]` layout in two passes: `populate_cache` (line ~160) computes rects and writes to `Surface.render_cache`, then `paint_leaves` (line ~197) walks the cache and builds a `TabbedPane` / `SidebarSlotPane` per leaf via `build_tabbed_pane` (line ~213) / `build_sidebar_pane` (line ~245).

This is the root cause of: `Surface.render_cache` (with `sidebar_rects`, `frame_rects`, `tab_strips`), the `LeafRef` enum, the `populate_cache`/`paint_leaves` split, the `build_*` helpers in the binary, and `SidebarSlot` living in `devix-core` despite being a layout-tree concern.

**The fix.** `Surface.root` becomes a single Pane tree that already contains the sidebars. The render path collapses to one call:

```rust
surface.root.render(area, &mut ctx);
```

The outer shape (sketch — final structure is your call):

```
SplitPane { axis: Horizontal, children: [
    SidebarSlotPane { slot: Left,  content: ... },   // present only when visible
    <existing editor split/tab subtree>,
    SidebarSlotPane { slot: Right, content: ... },
]}
```

Toggling a sidebar = mutating the root tree (insert/remove the slot pane). Not a separate visibility flag read by the binary.

**Constraints (don't break these):**

- Tab-strip click hit-testing. Today `Surface.render_cache.tab_strips` stores `TabStripCache { strip_rect, content_width, hits }` per `FrameId`. After the change, hits should resolve through tree walking (`devix_core::walk::pane_at`) plus a small per-`TabbedPane` cache populated *during* render. No global render-cache god-struct on `Surface`.
- Plugin-contributed sidebar content. See `crates/app/src/plugin.rs` (`pane_for`, `contributed_slots`, `sidebar_pane`, `plugin_slot_at`). Plugins currently hand a `LuaPane` to the binary which paints it inside `SidebarSlotPane`. Keep this working — the plugin's pane should end up as the `content` of the appropriate `SidebarSlotPane` in the root tree.
- Modal pane (`Surface.modal: Option<Box<dyn Pane>>`) stays exactly as it is. It's already correctly head-of-responder-chain. Don't touch.
- Focus chain (`Surface.focus`). Today it's an index path into the tree. After sidebars join the tree, the path semantics change — sidebar leaves become reachable. Update the focusable-leaf walker (`devix_core::walk::focusable_leaves`) and any sidebar focus toggle logic accordingly.
- Sidebar toggle commands (search `cmd::ToggleSidebar` or similar in `crates/commands/src/cmd/` and `crates/surface/src/cmd.rs` — there are two `cmd` locations, find both). They flip whatever state currently controls visibility. After the change, they mutate the root tree.
- Tests stay green. Especially `crates/surface/src/surface.rs` has tab-strip / sidebar / focus tests — read them before changing the tree shape so you understand the invariants they're guarding.

**What to delete when done:**

- `Surface.render_cache` field and the `RenderCache` / `TabStripCache` / `LeafRef` types — or as much of them as is no longer needed.
- `populate_cache` and `paint_leaves` in `crates/app/src/render.rs`.
- `build_tabbed_pane` / `build_sidebar_pane` in the same file.
- `crates/surface/src/tree.rs::leaves_with_rects` if it's only called by the cache path.

**What to consider keeping:**

- `Pane::children(area)` — the doc-blessed walker entry point. It's the right shape; the issue is the tree it walks, not the trait.
- `pane_at`, `focusable_leaves` in `devix-core::walk` — fine, just may walk a bigger tree now.
- `FrameId` — still useful as a stable key for the editor frames inside the tree (find them via `devix_surface::find_frame`).

**Process:**

- Stage commits so each compiles and `cargo test --workspace` passes. Sequence I'd suggest, but adjust if you find a cleaner cut: (a) build the new root tree shape and route render through it while leaving the old cache path alive in parallel; (b) flip render to the new path; (c) delete the old cache + helpers; (d) move `SidebarSlot` and friends out of `core` if they're now layout-implementation, not trait surface.
- Don't rename crates or move files between crates in this prompt. That's the next pass — keep this change focused on the tree fix so the reorg pass has a clean baseline.
- Read `crates/app/src/render.rs` end-to-end before starting. There's pre-paint scroll-into-view math (`layout_pass` or similar) that runs in the binary; check whether it survives unchanged or needs to move onto the relevant Pane.

**What "done" looks like:**

- One render call from the binary into the root pane.
- No global render cache on the root struct.
- Sidebar visibility = "the sidebar pane is in the tree" (not "a flag is set somewhere").
- All existing tests pass; manual smoke (open file, split, toggle sidebar, click a tab, open palette) works.

Report back: a summary of what moved, what got deleted, and any constraints you couldn't satisfy without a separate change.

---

## Prompt 2 — Crate reorg (run after Prompt 1 lands)

You're working in `/Users/karetski/Developer/devix`. Read `ARCHITECTURE-REFACTOR.md` for the design intent and `REORG-PROMPTS.md` (this file) for the prior prompt's outcome — assume the root-pane-tree fix is already merged.

**The problem.** The workspace has 11 crates: `app, commands, core, editor, plugin, surface, syntax, text, ui, view, workspace`. The architecture doc's stated endgame is 8 (and LSP was dropped, so really 7). Three carve-outs (`view`, `workspace`, `commands`) exist purely to break dep cycles, not to express concepts. Names collide: `editor` (crate) vs `EditorPane` vs `EditorView`; `view` (crate) vs `View` (struct) vs `EditorView`; `surface` (crate) vs `Surface` (struct) vs the graphics meaning of "surface"; `workspace` (crate) vs Cargo `[workspace]` vs the pre-rename meaning.

The user's complaint: each refactor has added crates and renamed things, but conceptual confusion increased instead of decreased. Goal is *fewer* names mapping *cleanly* to concepts.

**Target layout (7 crates):**

| Crate | Contents | Notes |
|---|---|---|
| `text` | rope, selection, transaction | unchanged |
| `syntax` | tree-sitter wrapper | unchanged |
| `core` | `Pane`, `Action`, `Event`, `Outcome`, `RenderCtx`, `HandleCtx`, `Rect`, `Anchor`, `Clipboard` — **traits and the types they mention only** | shrinks; today it also holds Theme, walk helpers, layout enums (`Axis`/`SidebarSlot`/`split_rects`) which should move out |
| `panes` | layout composites (`SplitPane`, `TabbedPane`, `SidebarSlotPane`), walk helpers (`pane_at`, `focusable_leaves`), chrome widgets (popup, tabstrip, palette renderer, sidebar chrome), `Theme` | new name; absorbs current `ui` crate plus the implementation half of current `core` |
| `editor` | `Document`, `View` (per-tab state), `EditorPane`, the **root struct**, commands, keymap, palette logic, modal | absorbs `view`, `workspace`, `commands`, `surface` |
| `plugin` | Lua host | unchanged |
| `devix` (bin) | terminal lifecycle, runtime, event loop, ~10-line render | shrinks dramatically after Prompt 1 |

**Concrete moves:**

1. `crates/view/` → fold into `crates/surface/` (closest to its consumers — Surface owns the `SlotMap<ViewId, View>`).
2. `crates/workspace/` → fold into `crates/editor/` (Document is the editor's model; the doc places it there in line 140 of `ARCHITECTURE-REFACTOR.md`).
3. `crates/commands/` → fold into `crates/surface/` (`Context` already wraps `&mut Surface`; commands are surface ops).
4. `crates/surface/` → fold into `crates/editor/`. Rename struct `Surface` → `Editor`. The crate name `editor` and the type name `editor::Editor` is the standard Rust pattern (`regex::Regex`, `tokio::Runtime`).
5. `crates/ui/` + the implementation half of `crates/core/` → new crate `crates/panes/`. Move out of `core`: `Theme`, `walk`, `Axis`, `Direction`, `SidebarSlot`, `split_rects`. Move out of `ui`: everything (it's all chrome widgets and layout helpers — they belong with the layout primitives). `core` shrinks to ~250 LOC of pure trait surface.
6. Rename the render-time helper `EditorView` (in `crates/editor/src/editor.rs`) to something local that doesn't collide with `View`. `BufferRender` or `EditorRender` — your call. Or inline if it's only used in one place.

**Naming decisions to make (pick decisively, don't open a question list):**

- Root struct name. Default: `Editor`. Lives at `editor::Editor`. Alternative: keep `Surface` if you find a strong reason after looking at the call sites — but the user pushed back hard on `Surface` being a confusing name so the bar for keeping it is high.
- `panes` crate name. Default: `panes`. Alternatives: `view` (overloaded), `chrome` (only fits half the contents), `widgets` (UIKit-flavored but accurate). Pick one and don't churn.

**Constraints:**

- Each commit compiles and `cargo test --workspace` passes.
- Plugin crate after the dust settles depends on `core` and `editor` (the host needs `Editor` to register actions/panes against). The architecture doc says "plugins depend only on `core`" — that's an aspirational statement about a *plugin SDK*, not the plugin host crate. Don't try to fix that boundary in this pass.
- Don't touch `app/src/render.rs` beyond what the rename forces. Prompt 1 already simplified it.
- Don't introduce backward-compat re-export modules. If a path changes, fix the imports. This is the cleanup pass — leaving compatibility shims is what got us into the current vocabulary mess.

**Suggested commit sequence (each green):**

1. Fold `view` into `surface`. Update imports. Delete `crates/view/`, drop the workspace dep, drop the `pub mod view` re-export shim in `crates/surface/src/lib.rs`.
2. Fold `commands` into `surface`. Update imports. Delete `crates/commands/`, drop the dep.
3. Fold `workspace` into `editor`. Update imports. Delete `crates/workspace/`, drop the dep. Watch for the `notify` dep moving with `Document`.
4. Fold `surface` into `editor`. Rename struct `Surface` → `Editor`. Rename `crates/editor/` continues to be the destination; `crates/surface/` is deleted. Update every `&mut Surface` → `&mut Editor`, every `app.surface` → `app.editor`.
5. Create `crates/panes/`. Move `crates/ui/src/*` and the implementation half of `crates/core/src/*` (Theme, walk, layout enums, split_rects). Drop `crates/ui/`. `core` shrinks.
6. Rename render-helper `EditorView` to its new name. Inline if trivial.

**What "done" looks like:**

- 7 crates: `app, text, syntax, core, panes, editor, plugin`.
- `core` is ~250 LOC of trait surface only.
- The four-concept design (Pane, Action, Document, Editor) maps 1:1 to four type names with no synonyms.
- `cargo test --workspace` green.
- `ARCHITECTURE-REFACTOR.md` updated to reflect the actual final layout (the doc currently lies about the crate count — replace the lie with the truth).

Report back: final crate inventory, any name collisions you hit and how you resolved them, anything you found that resists the consolidation (some boundary that turned out to pay rent after all).
