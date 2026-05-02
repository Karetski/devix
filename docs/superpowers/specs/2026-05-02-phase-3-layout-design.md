# Phase 3 — Layout: tabs, splits, sidebars, focus

Status: design (approved 2026-05-02)
Predecessors: PLAN.md Phases 1–2 (skeleton + editing) shipped.

## Goal

Replace the single-editor `App` model with a recursive layout tree that holds editor frames (tab groups), splits, and side-pinned sidebars. Deliver universal directional focus across editor splits and sidebars. No new editor features — this phase is structural.

## Non-goals

- Sidebar contents (Phase 8 — plugins).
- Tree-sitter / syntax highlighting (Phase 4).
- Command palette (Phase 5). Without it, sidebars have no end-user toggle.
- Multicursor (Phase 7). Phase 3 reserves the keybinding only.
- Resize splits via drag or keys.

## Decisions

| Decision | Choice |
|---|---|
| Document model | One `Document` per absolute path; multiple `View`s share it. |
| Open behavior | Opening a file replaces the current tab's view (per original spec). |
| Sidebar contents | Empty bordered placeholder, hidden by default. |
| Sidebar position | Pinned to layout root only — never nested inside editor splits. |
| Split shortcuts | `Ctrl+\` vertical, `Ctrl+-` horizontal. |
| Focus shortcut | `Ctrl+Alt+←/↓/↑/→`, universal — crosses into sidebars at editor edges. |
| Doc-cycle binding | Dropped (was `Ctrl+Alt+←/→`). |
| Multicursor binding | Moved to `Shift+Ctrl+↑/↓`. Reserved; action unwired in Phase 3. |
| Sidebar toggle | Action exists, no keybinding. Palette in Phase 5 will expose it. |
| Save | `Ctrl+S` (settled here; was an open question in PLAN.md). |
| Close tab | `Ctrl+W`. Refuses to close a dirty tab; status-line message. |
| Force close tab | `Ctrl+Shift+W`. |
| Edge wrap | `FocusDir` at workspace edge with no sidebar is a no-op (no wrap-around). |

## Architecture

### Layered data model

```
Workspace
  ├─ layout: Node                      // recursive layout tree
  ├─ frames:    SlotMap<FrameId, Frame>
  ├─ views:     SlotMap<ViewId, View>
  ├─ documents: SlotMap<DocId, Document>
  ├─ doc_index: HashMap<PathBuf, DocId>   // de-dup on open
  └─ focus: Vec<usize>                   // path of indices through `layout`

enum Node {
    Split { axis: Axis, children: Vec<(Node, u16)> },  // u16 = ratio weight
    Frame(FrameId),
    Sidebar(SidebarSlot),
}
enum Axis { Horizontal, Vertical }
enum SidebarSlot { Left, Right }

struct Frame {
    tabs: Vec<ViewId>,
    active_tab: usize,
}

struct View {
    doc: DocId,
    selection: Selection,
    target_col: Option<usize>,
    scroll_top: usize,
    view_anchored: bool,
}

struct Document {
    buffer: Buffer,
    watcher: Option<notify::RecommendedWatcher>,
    disk_changed_pending: bool,
}
```

The tree holds **IDs only**. Tree mutations never touch borrow lifetimes of the underlying state. A single `Document` can be referenced by multiple `View`s in different `Frame`s — that's how the same file shared across splits behaves.

`SlotMap` (the `slotmap` crate) keeps stable IDs across removal, so closing a tab/frame never invalidates IDs held elsewhere.

### Document de-duplication

`Workspace::open_path(path)` canonicalizes the path, looks up `doc_index`, and either reuses the existing `DocId` (creating a new `View` against it) or creates a new `Document`. The `notify` watcher is owned by the `Document`, so one watcher per file regardless of how many views look at it.

### Focus model

Focus is stored as a `Vec<usize>` path from root to a leaf — not just the leaf ID — so directional traversal is a tree walk, not a search.

`focus_dir(direction)` algorithm:

1. Walk *up* the focus path until reaching a `Split` whose axis matches the direction (Horizontal split for ←/→, Vertical for ↑/↓) and the current child is **not** at the boundary in the direction of motion.
2. Step to the sibling child in that direction.
3. Walk *down* into that subtree to a leaf, picking the spatially closest leaf using the source cursor's screen position against the cached `Rect`s of candidate leaves.
4. If step 1 reaches the root without finding a candidate, the move is a no-op — **except** for the universal-focus rule below.

Universal-focus rule: when `focus_dir(Left)` from the leftmost editor frame would otherwise no-op and a left sidebar is visible, focus moves into that sidebar. Mirror for Right. Up/Down never crosses into a sidebar (sidebars are full-height; vertical motion inside one is the sidebar's own concern, deferred to plugins).

`Workspace::active_view_mut()` returns `None` when focus is on a sidebar. View-affecting actions (`MoveLeft`, `Type`, `Save`, etc.) check this and no-op when there's no active view.

Actions that need an editor frame target — `OpenPath`, `NewTab`, `CloseTab`, `ForceCloseTab`, split actions — resolve their target via `Workspace::last_editor_focus`: the most-recently-focused frame, refreshed every time `focus` lands on a `Frame` leaf. If invoked while focus is on a sidebar, they operate on `last_editor_focus` *and* move focus back to that frame.

### Actions

Added to `Action`:

```rust
SplitVertical,
SplitHorizontal,
CloseFrame,
FocusDir(Direction),

NewTab,
CloseTab,
ForceCloseTab,
NextTab,
PrevTab,

ToggleSidebar(SidebarSlot),

OpenPath(PathBuf),
Save,
```

`Direction = Left | Down | Up | Right`.

### Dispatch

`dispatch::dispatch(ctx, action)` already exists. Refactored to:

- Resolve `ctx.workspace.active_view_mut()` for view-affecting actions; no-op if focus is on a sidebar.
- Operate on `ctx.workspace` directly for layout/tab/sidebar actions.

### Keymap (final Phase 3 state)

| Binding | Action |
|---|---|
| `Ctrl+\` | `SplitVertical` |
| `Ctrl+-` | `SplitHorizontal` |
| `Ctrl+Alt+←/↓/↑/→` | `FocusDir(...)` |
| `Ctrl+Shift+T` | `NewTab` |
| `Ctrl+W` | `CloseTab` |
| `Ctrl+Shift+W` | `ForceCloseTab` |
| `Ctrl+Shift+[` / `]` | `PrevTab` / `NextTab` |
| `Ctrl+S` | `Save` |
| `Shift+Ctrl+↑/↓` | (reserved for multicursor; unwired) |

Existing motion/edit/clipboard bindings unchanged.

## Rendering

Render stays pure (`fn render(state, frame)`). Tree → `Rect` algorithm:

```
fn layout(node, area):
    Split(axis, children) -> ratatui Layout with weighted ratios; recurse
    Frame(id)             -> [(LeafRef::Frame(id), area)]
    Sidebar(slot)         -> [(LeafRef::Sidebar(slot), area)]
```

Sidebars at the root use a fixed width (default 30 cells). Hidden sidebars are absent from the root's children — they don't reserve space.

`Frame` widget: 1-row tab strip on top with file-name pills (active tab inverted, ellipsis on overflow), then the active `View`'s editor body below. The existing `ui::editor` widget moves into `ui::frame` and renders against `(View, Document)` instead of an `EditorState`.

`Sidebar` widget: bordered empty area with the slot name in the title. Border style flips when focused.

`RenderCache { frame_rects: SecondaryMap<FrameId, Rect>, sidebar_rects: HashMap<SidebarSlot, Rect> }` is mutated during the render walk and read by dispatch (for viewport-aware actions and the spatial-closest focus hint). Writes happen during render; reads happen between frames.

`OverlayLayer` painted last each frame, populated with at most one item in Phase 3: a brief status toast (e.g., "unsaved changes — Ctrl+Shift+W to force close"). Phase 6 popups (completion, hover) will reuse this layer.

The existing `dirty` flag and coalesced-scroll behavior are preserved end to end.

## Implementation order

Each step keeps the editor runnable.

**Step 1 — Introduce `Document` and `View` (no layout tree yet).**
- New `crates/workspace/src/{document.rs, view.rs}` with slot-maps on `Workspace`.
- Refactor `EditorState` into `View` (selection + scroll + sticky col); buffer + watcher move into `Document`.
- `App` holds `Workspace` with one frame, one tab, one view. Behavior identical to today.

**Step 2 — Layout tree behind a single-frame stub.**
- Add `Node` enum and `Workspace::layout` (root = `Frame(only_id)`).
- Renderer walks the tree; output identical because the tree only has one frame.
- Add `RenderCache`; replace `App::last_editor_area` / `last_gutter_width`.

**Step 3 — Tabs.**
- `Frame { tabs, active_tab }`, tab strip widget.
- Wire `NewTab`, `CloseTab`, `ForceCloseTab`, `NextTab`, `PrevTab`.
- `OpenPath` replaces the current tab's view; old `View` is GC'd.
- Document de-dup (`open_path` reuses existing `DocId` when canonical path matches).
- Closing the last tab in a frame leaves the frame with one scratch tab (empty buffer, no path). Closing the last frame is impossible — `CloseFrame` no-ops on the only frame, so the workspace always has at least one frame with one tab.

**Step 4 — Splits.**
- `SplitVertical` / `SplitHorizontal`: replace the focused `Frame` leaf with a `Split` containing the original frame and a new frame whose first tab is a `View` clone (same `DocId`, copy of current selection/scroll).
- `CloseFrame`: collapses a `Split` with one remaining child into that child (no orphan splits).

**Step 5 — Sidebars + universal focus.**
- `Sidebar` leaf, `SidebarSlot::{Left, Right}`. `ToggleSidebar` action (no key in Phase 3).
- `FocusDir` implementation including spatial-closest pick using `RenderCache`.
- Sidebars paint empty bordered placeholders.
- Mouse-click-to-focus: clicking inside a leaf's `Rect` sets `focus`.

**Step 6 — Save, dirty-close, keymap finalize.**
- `Ctrl+S` Save.
- Dirty `CloseTab` refuses with status message; `ForceCloseTab` bypass.
- Move multicursor binding to `Shift+Ctrl+↑/↓` (action unwired). Drop doc-cycle binding.

## Tests

- `workspace/` unit tests: tree mutations (split, close, collapse-orphan-splits), focus traversal at edges and across splits with multi-level trees, document de-dup on canonicalized paths.
- Pin a regression test for the recent scroll-coalescing + `dirty`-flag behavior in Step 1, so the refactor can't quietly break it.
- No render-output integration tests in Phase 3 — golden-frame testing isn't justified at this stage.

## Files touched

- New: `crates/workspace/src/{layout.rs, document.rs, view.rs, frame.rs}`; `crates/ui/src/sidebar.rs`.
- Renamed: `crates/ui/src/editor.rs` → `crates/ui/src/frame.rs` (renders the active view inside a frame).
- Heavy edits: `crates/app/src/{app.rs, render.rs, events.rs, watcher.rs}`; `crates/workspace/src/{state.rs, dispatch.rs, action.rs}`; `crates/config/src/keymap.rs`.
- Untouched: `buffer/`, `lsp/`, `plugin/`, `syntax/`.

## Out of scope / follow-ups

- Resize splits via mouse drag or keys.
- Sidebar visibility keybinding — needs the palette (Phase 5).
- Sidebar contents — Phase 8 (plugins).
- Real popups (completion, hover) — Phase 6 reuses the `OverlayLayer` introduced here.
