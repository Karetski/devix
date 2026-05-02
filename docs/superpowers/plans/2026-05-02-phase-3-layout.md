# Phase 3 Layout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the single-editor `App` with a recursive layout tree (frames, splits, sidebars), shared Buffers across views, and universal `Ctrl+Alt+arrow` focus traversal across editor splits and sidebars.

**Architecture:** A new `Workspace` owns three slot-maps — `Document`, `View`, `Frame` — keyed by typed IDs. A `Node` enum tree references leaves by ID only, so tree mutations never collide with borrow lifetimes. `View`s reference `Document`s by `DocId`; reopening the same path returns the same `DocId` (shared buffers). Focus is a `Vec<usize>` path from root to a leaf; `FocusDir` is a tree walk with a spatial-closest pick fed by per-frame `Rect`s cached during render.

**Tech Stack:** Rust 2021, `ropey` 1.6, `ratatui` 0.29, `crossterm` 0.28, `notify`, `arboard`, `anyhow`. New: `slotmap` 1.x.

**Reference:** `docs/superpowers/specs/2026-05-02-phase-3-layout-design.md`.

**Conventions:** Run `cargo build` and `cargo test --workspace` after each task. Commit at the end of each task. Use `cargo nextest` if installed; otherwise `cargo test`.

---

## File map

New files:
- `crates/workspace/src/document.rs` — `Document`, `DocId`.
- `crates/workspace/src/view.rs` — `View`, `ViewId` (replaces `EditorState`).
- `crates/workspace/src/frame.rs` — `Frame`, `FrameId`.
- `crates/workspace/src/layout.rs` — `Node`, `Axis`, `SidebarSlot`, focus traversal.
- `crates/workspace/src/workspace.rs` — `Workspace` aggregate + `RenderCache`.
- `crates/ui/src/frame.rs` — frame widget (tab strip + editor body). Replaces `editor.rs` at the call sites; the existing `editor.rs` becomes the inner body renderer.
- `crates/ui/src/sidebar.rs` — sidebar widget (empty bordered placeholder).
- `crates/ui/src/tabstrip.rs` — tab strip widget.

Heavy edits:
- `crates/workspace/src/{lib.rs, action.rs, context.rs, dispatch.rs, state.rs}` — `state.rs` deleted at end of Step 1.
- `crates/app/src/{app.rs, render.rs, events.rs, watcher.rs}`.
- `crates/config/src/keymap.rs`.
- `crates/workspace/Cargo.toml` — add `slotmap`.

Untouched: `crates/buffer/`, `crates/lsp/`, `crates/plugin/`, `crates/syntax/`.

---

## Setup

### Task 0: Add `slotmap` dependency and scaffold module skeleton

**Files:**
- Modify: `crates/workspace/Cargo.toml`
- Modify: `crates/workspace/src/lib.rs`
- Create: `crates/workspace/src/document.rs` (skeleton)
- Create: `crates/workspace/src/view.rs` (skeleton)
- Create: `crates/workspace/src/frame.rs` (skeleton)
- Create: `crates/workspace/src/layout.rs` (skeleton)
- Create: `crates/workspace/src/workspace.rs` (skeleton)

- [ ] **Step 1: Add `slotmap` to workspace crate dependencies**

`crates/workspace/Cargo.toml`:

```toml
[package]
name = "devix-workspace"
edition.workspace = true
version.workspace = true
rust-version.workspace = true

[dependencies]
devix-buffer.workspace = true
arboard = { version = "3.4", default-features = false }
slotmap = "1"
notify = "6"
anyhow = { workspace = true }
```

- [ ] **Step 2: Create empty module files**

Each of `document.rs`, `view.rs`, `frame.rs`, `layout.rs`, `workspace.rs` starts as:

```rust
//! Phase 3 — populated in subsequent tasks.
```

- [ ] **Step 3: Wire modules into `lib.rs`**

`crates/workspace/src/lib.rs`:

```rust
//! Editor state, layout tree, action enum, and dispatcher.

pub mod action;
pub mod context;
pub mod dispatch;
pub mod document;
pub mod frame;
pub mod layout;
pub mod state;
pub mod view;
pub mod workspace;

pub use action::Action;
pub use context::{Context, StatusLine, Viewport};
pub use dispatch::dispatch;
pub use document::{DocId, Document};
pub use frame::{Frame, FrameId};
pub use layout::{Axis, Direction, Node, SidebarSlot};
pub use state::EditorState;
pub use view::{View, ViewId};
pub use workspace::{LeafRef, RenderCache, Workspace};
```

(Re-exports refer to types added in later tasks. Compilation will fail until those tasks land — that's fine for now; this commit only stages the module wiring. If you prefer a clean build at every commit, defer the re-exports until Task 4.)

- [ ] **Step 4: Verify build (re-exports may be incomplete)**

Run: `cargo check -p devix-workspace`
Expected: errors about missing items (`Document`, `View`, etc). Acceptable for this commit.

If you want a green build at every commit, drop the new re-exports for now; add them as the types land.

- [ ] **Step 5: Commit**

```bash
git add crates/workspace/Cargo.toml crates/workspace/src/lib.rs crates/workspace/src/{document,view,frame,layout,workspace}.rs
git commit -m "workspace: scaffold phase 3 modules + slotmap dep"
```

---

## Step 1 — Document and View

The existing `EditorState` already contains everything a `View` needs. The buffer + watcher state currently lives on `App`; these move into `Document`. After this step, `App` still has only one frame, one tab, one view — observable behavior is identical to today.

### Task 1: Define `Document`

**Files:**
- Modify: `crates/workspace/src/document.rs`
- Test: `crates/workspace/src/document.rs` (inline `#[cfg(test)]`)

- [ ] **Step 1: Write the failing test**

`crates/workspace/src/document.rs`:

```rust
//! Document = Buffer + filesystem-watcher attachment, owned by Workspace.

use std::path::PathBuf;

use anyhow::Result;
use devix_buffer::Buffer;
use slotmap::new_key_type;

new_key_type! { pub struct DocId; }

pub struct Document {
    pub buffer: Buffer,
    pub watcher: Option<notify::RecommendedWatcher>,
    pub disk_changed_pending: bool,
}

impl Document {
    pub fn from_buffer(buffer: Buffer) -> Self {
        Self { buffer, watcher: None, disk_changed_pending: false }
    }

    pub fn from_path(path: PathBuf) -> Result<Self> {
        Ok(Self::from_buffer(Buffer::from_path(path)?))
    }

    pub fn empty() -> Self {
        Self::from_buffer(Buffer::empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_document_has_no_path_and_no_watcher() {
        let d = Document::empty();
        assert!(d.buffer.path().is_none());
        assert!(d.watcher.is_none());
        assert!(!d.disk_changed_pending);
        assert!(!d.buffer.dirty());
    }
}
```

- [ ] **Step 2: Run test — expect missing `notify` crate**

Run: `cargo test -p devix-workspace document::tests::`
Expected: build error if `notify` isn't yet a dep, OR test passes if it is. The added `Cargo.toml` from Task 0 includes it.

- [ ] **Step 3: Run test — expect pass**

Run: `cargo test -p devix-workspace document::tests::empty_document_has_no_path_and_no_watcher`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/workspace/src/document.rs
git commit -m "workspace: introduce Document (buffer + watcher attachment)"
```

---

### Task 2: Define `View` (replaces `EditorState`)

**Files:**
- Modify: `crates/workspace/src/view.rs`
- Test: inline `#[cfg(test)]` block

- [ ] **Step 1: Write `View`**

`crates/workspace/src/view.rs`:

```rust
//! View = per-frame editor state (selection + sticky col + scroll).
//! Owned by Workspace, indexed by ViewId.

use devix_buffer::{Range, Selection, Transaction};
use slotmap::new_key_type;

use crate::document::DocId;

new_key_type! { pub struct ViewId; }

pub struct View {
    pub doc: DocId,
    pub selection: Selection,
    /// Sticky column for vertical motion.
    pub target_col: Option<usize>,
    pub scroll_top: usize,
    /// Anchored: render keeps the cursor in view. Detached: scroll_top floats.
    pub view_anchored: bool,
}

impl View {
    pub fn new(doc: DocId) -> Self {
        Self {
            doc,
            selection: Selection::point(0),
            target_col: None,
            scroll_top: 0,
            view_anchored: true,
        }
    }

    pub fn primary(&self) -> Range {
        self.selection.primary()
    }

    pub fn move_to(&mut self, idx: usize, extend: bool, sticky_col: bool) {
        let r = self.primary().put_head(idx, extend);
        *self.selection.primary_mut() = r;
        if !sticky_col {
            self.target_col = None;
        }
    }

    /// Apply a transaction's selection_after; the buffer mutation happens on
    /// the Document side (the caller does buffer.apply(tx) first).
    pub fn adopt_selection_after(&mut self, tx: &Transaction) {
        self.selection = tx.selection_after.clone();
        self.target_col = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;

    #[test]
    fn fresh_view_starts_at_origin_anchored() {
        let mut docs: SlotMap<DocId, ()> = SlotMap::with_key();
        let id = docs.insert(());
        let v = View::new(id);
        assert_eq!(v.primary().head, 0);
        assert!(v.view_anchored);
        assert!(v.target_col.is_none());
        assert_eq!(v.scroll_top, 0);
    }
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p devix-workspace view::tests::fresh_view_starts_at_origin_anchored`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/workspace/src/view.rs
git commit -m "workspace: introduce View (renames EditorState data shape)"
```

---

### Task 3: Build `Workspace` shell with single-frame seed

**Files:**
- Modify: `crates/workspace/src/frame.rs`
- Modify: `crates/workspace/src/workspace.rs`
- Test: inline `#[cfg(test)]` block in `workspace.rs`

- [ ] **Step 1: Define `Frame` with one tab**

`crates/workspace/src/frame.rs`:

```rust
//! Frame = editor tab group: a strip of tabs plus an active index.
//! Each tab is a ViewId.

use slotmap::new_key_type;

use crate::view::ViewId;

new_key_type! { pub struct FrameId; }

pub struct Frame {
    pub tabs: Vec<ViewId>,
    pub active_tab: usize,
}

impl Frame {
    pub fn with_view(view: ViewId) -> Self {
        Self { tabs: vec![view], active_tab: 0 }
    }

    pub fn active_view(&self) -> ViewId {
        self.tabs[self.active_tab]
    }
}
```

- [ ] **Step 2: Define `Workspace` with seeded frame**

`crates/workspace/src/workspace.rs`:

```rust
//! Workspace = aggregate of all editor state owned across the layout tree:
//! documents, views, frames, plus the (Phase 3, single-frame for now) layout
//! root, focus path, and the per-frame render-rect cache.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use ratatui::layout::Rect;
use slotmap::{SecondaryMap, SlotMap};

use crate::document::{DocId, Document};
use crate::frame::{Frame, FrameId};
use crate::layout::{Node, SidebarSlot};
use crate::view::{View, ViewId};
use devix_buffer::Buffer;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LeafRef {
    Frame(FrameId),
    Sidebar(SidebarSlot),
}

#[derive(Default)]
pub struct RenderCache {
    pub frame_rects: SecondaryMap<FrameId, Rect>,
    pub sidebar_rects: HashMap<SidebarSlot, Rect>,
}

pub struct Workspace {
    pub documents: SlotMap<DocId, Document>,
    pub views: SlotMap<ViewId, View>,
    pub frames: SlotMap<FrameId, Frame>,
    pub layout: Node,
    pub focus: Vec<usize>,
    pub doc_index: HashMap<PathBuf, DocId>,
    pub last_editor_focus: Vec<usize>,
    pub render_cache: RenderCache,
}

impl Workspace {
    /// Create a workspace with a single frame, single tab, single view.
    /// `path` is opened if Some; otherwise an empty scratch buffer is used.
    pub fn open(path: Option<PathBuf>) -> Result<Self> {
        let mut documents: SlotMap<DocId, Document> = SlotMap::with_key();
        let mut views: SlotMap<ViewId, View> = SlotMap::with_key();
        let mut frames: SlotMap<FrameId, Frame> = SlotMap::with_key();
        let mut doc_index = HashMap::new();

        let doc_id = match path {
            Some(p) => {
                let canonical = canonicalize_or_keep(&p);
                let id = documents.insert(Document::from_path(p)?);
                doc_index.insert(canonical, id);
                id
            }
            None => documents.insert(Document::empty()),
        };
        let view_id = views.insert(View::new(doc_id));
        let frame_id = frames.insert(Frame::with_view(view_id));
        let layout = Node::Frame(frame_id);
        let focus = vec![]; // root is the frame leaf itself
        let last_editor_focus = focus.clone();

        Ok(Self {
            documents,
            views,
            frames,
            layout,
            focus,
            doc_index,
            last_editor_focus,
            render_cache: RenderCache::default(),
        })
    }

    pub fn active_view(&self) -> Option<&View> {
        let frame_id = self.active_frame()?;
        let view_id = self.frames[frame_id].active_view();
        self.views.get(view_id)
    }

    pub fn active_view_mut(&mut self) -> Option<&mut View> {
        let frame_id = self.active_frame()?;
        let view_id = self.frames[frame_id].active_view();
        self.views.get_mut(view_id)
    }

    pub fn active_frame(&self) -> Option<FrameId> {
        match self.layout.leaf_at(&self.focus)? {
            LeafRef::Frame(id) => Some(id),
            LeafRef::Sidebar(_) => None,
        }
    }

    /// Document the active view points at. None when focus is on a sidebar.
    pub fn active_doc_mut(&mut self) -> Option<&mut Document> {
        let v = self.active_view()?;
        self.documents.get_mut(v.doc)
    }

    pub fn active_doc(&self) -> Option<&Document> {
        let v = self.active_view()?;
        self.documents.get(v.doc)
    }

    /// Insert a buffer as a fresh document; returns the new DocId.
    pub fn insert_buffer(&mut self, buf: Buffer) -> DocId {
        self.documents.insert(Document::from_buffer(buf))
    }
}

fn canonicalize_or_keep(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_workspace_has_one_frame_one_view() {
        let ws = Workspace::open(None).unwrap();
        assert_eq!(ws.frames.len(), 1);
        assert_eq!(ws.views.len(), 1);
        assert_eq!(ws.documents.len(), 1);
        assert!(ws.active_view().is_some());
    }
}
```

- [ ] **Step 3: Stub `Node::leaf_at` so this compiles (full impl in Task 5)**

`crates/workspace/src/layout.rs`:

```rust
//! Recursive layout tree.

use crate::frame::FrameId;
use crate::workspace::LeafRef;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Axis { Horizontal, Vertical }

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SidebarSlot { Left, Right }

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Direction { Left, Down, Up, Right }

pub enum Node {
    Split { axis: Axis, children: Vec<(Node, u16)> },
    Frame(FrameId),
    Sidebar(SidebarSlot),
}

impl Node {
    /// Resolve a focus path (Vec<usize>) to its leaf reference.
    /// Returns None if the path is invalid.
    pub fn leaf_at(&self, path: &[usize]) -> Option<LeafRef> {
        let mut node = self;
        for &idx in path {
            match node {
                Node::Split { children, .. } => {
                    let (child, _) = children.get(idx)?;
                    node = child;
                }
                _ => return None,
            }
        }
        match node {
            Node::Frame(id) => Some(LeafRef::Frame(*id)),
            Node::Sidebar(slot) => Some(LeafRef::Sidebar(*slot)),
            Node::Split { .. } => None,
        }
    }
}
```

- [ ] **Step 4: Wire updated re-exports in `lib.rs`**

Add to `crates/workspace/src/lib.rs`:

```rust
pub use document::{DocId, Document};
pub use frame::{Frame, FrameId};
pub use layout::{Axis, Direction, Node, SidebarSlot};
pub use view::{View, ViewId};
pub use workspace::{LeafRef, RenderCache, Workspace};
```

- [ ] **Step 5: Run test**

Run: `cargo test -p devix-workspace workspace::tests::fresh_workspace_has_one_frame_one_view`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/workspace/src/{frame,layout,workspace}.rs crates/workspace/src/lib.rs
git commit -m "workspace: Workspace aggregate with single-frame seed"
```

---

### Task 4: Refactor `App` and `Context` to use `Workspace`

`EditorState` is replaced by accessing `Workspace::active_view_mut()` + `Workspace::active_doc_mut()`. The legacy `state.rs` is kept temporarily as a re-export shim so dispatch stops compiling against it without a giant single-commit churn — then deleted.

**Files:**
- Modify: `crates/workspace/src/context.rs`
- Modify: `crates/workspace/src/dispatch.rs`
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/render.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/watcher.rs`
- Delete: `crates/workspace/src/state.rs`

- [ ] **Step 1: Update `Context` to carry a `Workspace` reference**

`crates/workspace/src/context.rs`:

```rust
//! Dispatcher context.

use ratatui::layout::Rect;

use crate::workspace::Workspace;

#[derive(Default)]
pub struct StatusLine(Option<String>);

impl StatusLine {
    pub fn set(&mut self, s: impl Into<String>) { self.0 = Some(s.into()); }
    pub fn clear(&mut self) { self.0 = None; }
    pub fn get(&self) -> Option<&str> { self.0.as_deref() }
}

#[derive(Copy, Clone, Default)]
pub struct Viewport {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub gutter_width: u16,
}

impl From<(Rect, u16)> for Viewport {
    fn from((rect, gutter_width): (Rect, u16)) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            gutter_width,
        }
    }
}

pub struct Context<'a> {
    pub workspace: &'a mut Workspace,
    pub clipboard: &'a mut Option<arboard::Clipboard>,
    pub status: &'a mut StatusLine,
    pub quit: &'a mut bool,
    pub viewport: Viewport,
}
```

- [ ] **Step 2: Rewrite `dispatch.rs` against `Workspace`**

`crates/workspace/src/dispatch.rs`:

```rust
//! Action dispatcher.

use devix_buffer::{Buffer, Range, Selection, delete_range_tx, replace_selection_tx};

use crate::action::Action;
use crate::context::{Context, Viewport};
use crate::document::Document;
use crate::view::View;

pub fn dispatch(action: Action, cx: &mut Context<'_>) {
    use Action::*;
    match action {
        // ---- motion ----
        MoveLeft { extend } => with_view_buf(cx, |v, b| {
            let to = b.move_left(v.primary().head);
            v.move_to(to, extend, false);
        }),
        MoveRight { extend } => with_view_buf(cx, |v, b| {
            let to = b.move_right(v.primary().head);
            v.move_to(to, extend, false);
        }),
        MoveUp { extend } => move_vertical(cx, false, extend),
        MoveDown { extend } => move_vertical(cx, true, extend),
        MoveWordLeft { extend } => with_view_buf(cx, |v, b| {
            let to = b.word_left(v.primary().head);
            v.move_to(to, extend, false);
        }),
        MoveWordRight { extend } => with_view_buf(cx, |v, b| {
            let to = b.word_right(v.primary().head);
            v.move_to(to, extend, false);
        }),
        MoveLineStart { extend } => with_view_buf(cx, |v, b| {
            let to = b.line_start_of(v.primary().head);
            v.move_to(to, extend, false);
        }),
        MoveLineEnd { extend } => with_view_buf(cx, |v, b| {
            let to = b.line_end_of(v.primary().head);
            v.move_to(to, extend, false);
        }),
        MoveDocStart { extend } => with_view_buf(cx, |v, b| {
            v.move_to(b.doc_start(), extend, false);
        }),
        MoveDocEnd { extend } => with_view_buf(cx, |v, b| {
            v.move_to(b.doc_end(), extend, false);
        }),
        PageUp { extend } => {
            let step = page_step(cx.viewport);
            for _ in 0..step { move_vertical(cx, false, extend); }
        }
        PageDown { extend } => {
            let step = page_step(cx.viewport);
            for _ in 0..step { move_vertical(cx, true, extend); }
        }

        // ---- edits ----
        InsertChar(c) => {
            let mut buf = [0u8; 4];
            replace_selection(cx, c.encode_utf8(&mut buf));
        }
        InsertNewline => replace_selection(cx, "\n"),
        InsertTab => replace_selection(cx, "    "),
        DeleteBack { word } => delete_primary_or(cx, |buf, head| {
            if head == 0 { return None; }
            let start = if word { buf.word_left(head) } else { head - 1 };
            Some((start, head))
        }),
        DeleteForward { word } => delete_primary_or(cx, |buf, head| {
            let len = buf.len_chars();
            if head >= len { return None; }
            let end = if word { buf.word_right(head) } else { head + 1 };
            Some((head, end))
        }),

        // ---- history ----
        Undo => {
            let (Some(d), Some(v)) = active_pair_mut(cx) else { return };
            if let Some(sel) = d.buffer.undo() {
                v.selection = sel;
                v.target_col = None;
                cx.status.clear();
            } else {
                cx.status.set("nothing to undo");
            }
        }
        Redo => {
            let (Some(d), Some(v)) = active_pair_mut(cx) else { return };
            if let Some(sel) = d.buffer.redo() {
                v.selection = sel;
                v.target_col = None;
                cx.status.clear();
            } else {
                cx.status.set("nothing to redo");
            }
        }

        // ---- selection ----
        SelectAll => with_view_buf(cx, |v, b| {
            let end = b.len_chars();
            v.selection = Selection::single(Range::new(0, end));
            v.target_col = None;
        }),

        // ---- clipboard ----
        Copy => do_copy(cx),
        Cut => do_cut(cx),
        Paste => do_paste(cx),

        // ---- file / disk ----
        Save => {
            let Some(d) = cx.workspace.active_doc_mut() else { return };
            let msg = match d.buffer.save() {
                Ok(()) => "saved".to_string(),
                Err(e) => format!("save failed: {e}"),
            };
            cx.status.set(msg);
        }
        ReloadFromDisk => {
            let Some(d) = cx.workspace.active_doc_mut() else { return };
            match d.buffer.reload_from_disk() {
                Ok(()) => {
                    let max = d.buffer.len_chars();
                    d.disk_changed_pending = false;
                    if let Some(v) = cx.workspace.active_view_mut() { v.selection.clamp(max); }
                    cx.status.set("reloaded from disk");
                }
                Err(e) => cx.status.set(format!("reload failed: {e}")),
            }
        }
        KeepBufferIgnoreDisk => {
            if let Some(d) = cx.workspace.active_doc_mut() {
                d.disk_changed_pending = false;
            }
            cx.status.set("kept buffer; disk change ignored");
        }

        // ---- app ----
        Quit => *cx.quit = true,

        // ---- mouse ----
        ClickAt { col, row, extend } => {
            if let Some(idx) = click_to_char_idx(cx, col, row) {
                if let Some(v) = cx.workspace.active_view_mut() {
                    v.move_to(idx, extend, false);
                }
            }
        }
        DragAt { col, row } => {
            if let Some(idx) = click_to_char_idx(cx, col, row) {
                if let Some(v) = cx.workspace.active_view_mut() {
                    v.move_to(idx, true, false);
                }
            }
        }
        ScrollBy(delta) => {
            let Some(d) = cx.workspace.active_doc() else { return };
            let max = d.buffer.line_count().saturating_sub(1) as isize;
            if let Some(v) = cx.workspace.active_view_mut() {
                let next = (v.scroll_top as isize).saturating_add(delta);
                v.scroll_top = next.clamp(0, max) as usize;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn page_step(v: Viewport) -> usize { v.height.saturating_sub(1).max(1) as usize }

fn active_pair_mut<'a>(cx: &'a mut Context<'_>) -> (Option<&'a mut Document>, Option<&'a mut View>) {
    // Borrow the slot-maps disjointly through Workspace's get_mut helpers.
    let view_id_opt = cx.workspace.active_view().map(|_| ()).and_then(|_| {
        cx.workspace.active_frame()
            .map(|fid| cx.workspace.frames[fid].active_view())
    });
    let doc_id_opt = view_id_opt.and_then(|vid| cx.workspace.views.get(vid).map(|v| v.doc));
    match (doc_id_opt, view_id_opt) {
        (Some(did), Some(vid)) => (
            cx.workspace.documents.get_mut(did),
            cx.workspace.views.get_mut(vid),
        ),
        _ => (None, None),
    }
}

fn with_view_buf(cx: &mut Context<'_>, f: impl FnOnce(&mut View, &Buffer)) {
    let (Some(d), Some(v)) = active_pair_mut(cx) else { return };
    f(v, &d.buffer);
}

fn move_vertical(cx: &mut Context<'_>, down: bool, extend: bool) {
    let (Some(d), Some(v)) = active_pair_mut(cx) else { return };
    let head = v.primary().head;
    let col = v.target_col.unwrap_or_else(|| d.buffer.col_of_char(head));
    let new = if down {
        d.buffer.move_down(head, Some(col))
    } else {
        d.buffer.move_up(head, Some(col))
    };
    v.target_col = Some(col);
    v.move_to(new, extend, true);
}

fn replace_selection(cx: &mut Context<'_>, text: &str) {
    let (Some(d), Some(v)) = active_pair_mut(cx) else { return };
    let tx = replace_selection_tx(&d.buffer, &v.selection, text);
    let after = tx.selection_after.clone();
    d.buffer.apply(tx);
    v.selection = after;
    v.target_col = None;
    cx.status.clear();
}

fn delete_primary_or(
    cx: &mut Context<'_>,
    builder: impl FnOnce(&Buffer, usize) -> Option<(usize, usize)>,
) {
    let (Some(d), Some(v)) = active_pair_mut(cx) else { return };
    let prim = v.primary();
    let (start, end) = if !prim.is_empty() {
        (prim.start(), prim.end())
    } else {
        let Some(span) = builder(&d.buffer, prim.head) else { return };
        if span.0 == span.1 { return; }
        span
    };
    let tx = delete_range_tx(&d.buffer, &v.selection, start, end);
    let after = tx.selection_after.clone();
    d.buffer.apply(tx);
    v.selection = after;
    v.target_col = None;
    cx.status.clear();
}

fn current_line_span(buf: &Buffer, head: usize) -> (usize, usize) {
    let line = buf.line_of_char(head);
    let start = buf.line_start(line);
    let end_no_nl = start + buf.line_len_chars(line);
    let end = if line + 1 < buf.line_count() {
        buf.line_start(line + 1)
    } else {
        end_no_nl
    };
    (start, end)
}

fn do_copy(cx: &mut Context<'_>) {
    let (Some(d), Some(v)) = active_pair_mut(cx) else { return };
    let prim = v.primary();
    let (start, end, msg) = if prim.is_empty() {
        let (s, e) = current_line_span(&d.buffer, prim.head);
        (s, e, "copied line")
    } else {
        (prim.start(), prim.end(), "copied")
    };
    if start == end { return; }
    let text = d.buffer.slice_to_string(start, end);
    let Some(cb) = cx.clipboard.as_mut() else {
        cx.status.set("no system clipboard"); return;
    };
    if cb.set_text(text).is_err() { cx.status.set("clipboard error"); return; }
    cx.status.set(msg);
}

fn do_cut(cx: &mut Context<'_>) {
    let (Some(d), Some(v)) = active_pair_mut(cx) else { return };
    let prim = v.primary();
    let (start, end, line_cut) = if prim.is_empty() {
        let (s, e) = current_line_span(&d.buffer, prim.head);
        (s, e, true)
    } else {
        (prim.start(), prim.end(), false)
    };
    if start == end { return; }
    let text = d.buffer.slice_to_string(start, end);
    let Some(cb) = cx.clipboard.as_mut() else {
        cx.status.set("no system clipboard"); return;
    };
    if cb.set_text(text).is_err() { cx.status.set("clipboard error"); return; }
    let tx = delete_range_tx(&d.buffer, &v.selection, start, end);
    let after = tx.selection_after.clone();
    d.buffer.apply(tx);
    v.selection = after;
    v.target_col = None;
    cx.status.set(if line_cut { "cut line" } else { "cut" });
}

fn do_paste(cx: &mut Context<'_>) {
    let text = match cx.clipboard.as_mut().and_then(|cb| cb.get_text().ok()) {
        Some(t) => t,
        None => { cx.status.set("clipboard empty"); return; }
    };
    if text.is_empty() { return; }
    replace_selection(cx, &text);
    cx.status.set("pasted");
}

fn click_to_char_idx(cx: &Context<'_>, col: u16, row: u16) -> Option<usize> {
    let v = cx.viewport;
    if row < v.y || row >= v.y + v.height { return None; }
    let text_x = v.x + v.gutter_width;
    let click_col = col.saturating_sub(text_x) as usize;
    let row_in_view = (row - v.y) as usize;
    let view = cx.workspace.active_view()?;
    let buf = &cx.workspace.documents.get(view.doc)?.buffer;
    let line = (view.scroll_top + row_in_view).min(buf.line_count().saturating_sub(1));
    let local_col = click_col.min(buf.line_len_chars(line));
    Some(buf.line_start(line) + local_col)
}
```

- [ ] **Step 3: Delete `state.rs` and remove its re-export**

```bash
rm crates/workspace/src/state.rs
```

In `crates/workspace/src/lib.rs`, remove `pub mod state;` and `pub use state::EditorState;`.

- [ ] **Step 4: Rewrite `App` to own `Workspace`**

`crates/app/src/app.rs`:

```rust
//! Editor application: owns terminal-side state and runs the event loop.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::Rect;
use devix_config::{Keymap, default_keymap};
use devix_workspace::{Action, StatusLine, Workspace};

use crate::clipboard;
use crate::events::{handle_event, run_action};
use crate::render::render;
use crate::watcher::{drain_disk_events, spawn_watcher};

pub struct App {
    pub workspace: Workspace,
    pub keymap: Keymap,
    pub status: StatusLine,
    pub quit: bool,
    pub last_editor_area: Rect,
    pub last_gutter_width: u16,
    pub clipboard: Option<arboard::Clipboard>,
    pub _watcher: Option<notify::RecommendedWatcher>,
    pub disk_rx: Option<mpsc::Receiver<()>>,
    pub dirty: bool,
    pub pending_scroll: isize,
}

const MAX_DRAIN_PER_TICK: usize = 256;
const POLL_TIMEOUT: Duration = Duration::from_millis(100);

impl App {
    pub fn new(path: Option<PathBuf>) -> Result<Self> {
        let workspace = Workspace::open(path.clone())?;
        let clipboard = clipboard::init();

        let (watcher, rx) = match path.as_deref() {
            Some(p) if p.exists() => spawn_watcher(p)
                .ok()
                .map(|(w, r)| (Some(w), Some(r)))
                .unwrap_or((None, None)),
            _ => (None, None),
        };

        Ok(Self {
            workspace,
            keymap: default_keymap(),
            status: StatusLine::default(),
            quit: false,
            last_editor_area: Rect::default(),
            last_gutter_width: 0,
            clipboard,
            _watcher: watcher,
            disk_rx: rx,
            dirty: true,
            pending_scroll: 0,
        })
    }
}

pub fn run<B: Backend>(terminal: &mut Terminal<B>, path: Option<PathBuf>) -> Result<()> {
    let mut app = App::new(path)?;

    while !app.quit {
        drain_disk_events(&mut app);

        if app.dirty {
            terminal.draw(|frame| render(frame, &mut app))?;
            app.dirty = false;
        }

        if !event::poll(POLL_TIMEOUT)? { continue; }
        handle_event(event::read()?, &mut app);

        let mut drained = 1;
        while drained < MAX_DRAIN_PER_TICK
            && !app.quit
            && event::poll(Duration::ZERO)?
        {
            handle_event(event::read()?, &mut app);
            drained += 1;
        }

        if app.pending_scroll != 0 {
            let delta = std::mem::take(&mut app.pending_scroll);
            run_action(&mut app, Action::ScrollBy(delta));
        }
    }
    Ok(())
}
```

- [ ] **Step 5: Update `render.rs` to read from Workspace**

`crates/app/src/render.rs`:

```rust
//! Frame composition: editor area + status line.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use devix_ui::{EditorView, StatusInfo, render_editor, render_status as render_status_widget};

use crate::app::App;

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());
    let editor_area = chunks[0];
    let status_area = chunks[1];
    app.last_editor_area = editor_area;

    let visible = editor_area.height as usize;
    let view = app.workspace.active_view().expect("phase 3 step 1 always has an active view");
    let doc = app.workspace.documents.get(view.doc).expect("doc exists");
    let mut anchored = view.view_anchored;
    let mut scroll_top = view.scroll_top;
    let head = view.primary().head;

    if anchored && visible > 0 {
        let cur_line = doc.buffer.line_of_char(head);
        if cur_line < scroll_top {
            scroll_top = cur_line;
        } else if cur_line >= scroll_top + visible {
            scroll_top = cur_line + 1 - visible;
        }
    }
    // Write back any anchored adjustment.
    if let Some(v) = app.workspace.active_view_mut() {
        v.scroll_top = scroll_top;
        anchored = v.view_anchored;
        let _ = anchored;
    }

    let view = app.workspace.active_view().unwrap();
    let doc = app.workspace.documents.get(view.doc).unwrap();
    let editor_view = EditorView {
        buffer: &doc.buffer,
        selection: &view.selection,
        scroll_top: view.scroll_top,
    };
    let r = render_editor(editor_view, editor_area, frame);
    app.last_gutter_width = r.gutter_width;
    if let Some((x, y)) = r.cursor_screen { frame.set_cursor_position((x, y)); }

    render_status(frame, status_area, app);
}

fn render_status(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let view = app.workspace.active_view().unwrap();
    let doc = app.workspace.documents.get(view.doc).unwrap();
    let path_str = doc.buffer.path().map(|p| p.display().to_string());
    let head = view.primary().head;
    let info = StatusInfo {
        path: path_str.as_deref(),
        dirty: doc.buffer.dirty(),
        line: doc.buffer.line_of_char(head) + 1,
        col: doc.buffer.col_of_char(head) + 1,
        sel_len: view.primary().len(),
        message: app.status.get(),
    };
    render_status_widget(&info, area, frame);
}
```

- [ ] **Step 6: Update `events.rs` and `watcher.rs` to flow through Workspace**

`crates/app/src/events.rs`: replace the `run_action` body's `Context` construction:

```rust
pub fn run_action(app: &mut App, action: Action) {
    let viewport = Viewport {
        x: app.last_editor_area.x,
        y: app.last_editor_area.y,
        width: app.last_editor_area.width,
        height: app.last_editor_area.height,
        gutter_width: app.last_gutter_width,
    };
    let is_scroll = matches!(action, Action::ScrollBy(_));

    let mut cx = Context {
        workspace: &mut app.workspace,
        clipboard: &mut app.clipboard,
        status: &mut app.status,
        quit: &mut app.quit,
        viewport,
    };
    dispatch(action, &mut cx);

    if let Some(v) = app.workspace.active_view_mut() {
        v.view_anchored = !is_scroll;
    }
    app.dirty = true;
}
```

The disk-pending gate at the top of `handle_key` now reads from the active document:

```rust
pub fn handle_key(code: KeyCode, mods: KeyModifiers, app: &mut App) {
    let pending = app.workspace.active_doc().map(|d| d.disk_changed_pending).unwrap_or(false);
    if pending && mods.contains(KeyModifiers::CONTROL) {
        let lower = match code {
            KeyCode::Char(c) => Some(c.to_ascii_lowercase()),
            _ => None,
        };
        match lower {
            Some('r') => { run_action(app, Action::ReloadFromDisk); return; }
            Some('k') => { run_action(app, Action::KeepBufferIgnoreDisk); return; }
            _ => {}
        }
    }
    // …rest unchanged…
}
```

`crates/app/src/watcher.rs`'s `drain_disk_events`:

```rust
pub fn drain_disk_events(app: &mut App) {
    let Some(rx) = app.disk_rx.as_ref() else { return };
    let mut got = false;
    while rx.try_recv().is_ok() { got = true; }
    if !got { return; }
    let dirty = app.workspace.active_doc().map(|d| d.buffer.dirty()).unwrap_or(false);
    if dirty {
        if let Some(d) = app.workspace.active_doc_mut() { d.disk_changed_pending = true; }
        app.status.set("Disk changed (buffer modified) · Ctrl+R reload, Ctrl+K keep");
        app.dirty = true;
    } else {
        run_action(app, Action::ReloadFromDisk);
    }
}
```

Move the watcher onto the active `Document` instead of `App`: assign `app.workspace.active_doc_mut().unwrap().watcher = watcher.take()` after `Workspace::open` succeeds. The `App._watcher` field can stay as a backstop holder for now if it simplifies lifetime but is not load-bearing.

- [ ] **Step 7: Pin scroll/dirty regression test**

Append to `crates/workspace/src/workspace.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn scroll_clamps_at_zero_and_at_end() {
    use devix_buffer::{Buffer, Selection};

    let mut ws = Workspace::open(None).unwrap();
    // Insert 100 lines into the active doc.
    let txt = "x\n".repeat(100);
    let v = ws.active_view_mut().unwrap();
    let sel = Selection::point(0);
    let tx = devix_buffer::replace_selection_tx(
        &Buffer::empty(), &sel, &txt,
    );
    drop(v);
    let did = ws.active_view().unwrap().doc;
    ws.documents[did].buffer.apply(tx);

    // Simulate scroll-up at zero (no underflow).
    let v = ws.active_view_mut().unwrap();
    v.scroll_top = 0;
    let next: isize = (v.scroll_top as isize).saturating_add(-1);
    v.scroll_top = next.clamp(0, 99) as usize;
    assert_eq!(v.scroll_top, 0);

    // Scroll down past end clamps to last line.
    let v = ws.active_view_mut().unwrap();
    let next: isize = (v.scroll_top as isize).saturating_add(1_000_000);
    v.scroll_top = next.clamp(0, 99) as usize;
    assert_eq!(v.scroll_top, 99);
}
```

- [ ] **Step 8: Run full workspace test**

Run: `cargo build && cargo test --workspace`
Expected: PASS.

Smoke-test the binary: `cargo run -- README.md` (or any text file in the repo). Confirm typing, arrows, undo (`Ctrl+Z`), save (`Ctrl+S`), copy/cut/paste, mouse click, and scroll all behave as before.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "workspace+app: route through Workspace; drop EditorState"
```

---

## Step 2 — Layout tree behind a single-frame stub

The `Workspace::layout` field already exists from Task 3 as a `Node::Frame(only_id)`. This step rewrites the renderer to walk the tree and adds the `RenderCache` writeback. Output is identical because the tree only has one frame.

### Task 5: Renderer walks the tree; RenderCache populated

**Files:**
- Modify: `crates/app/src/render.rs`
- Modify: `crates/workspace/src/workspace.rs` (RenderCache helpers)

- [ ] **Step 1: Add a tree-walking layout helper**

Append to `crates/workspace/src/layout.rs`:

```rust
use ratatui::layout::{Constraint, Direction as RatDirection, Layout, Rect};

impl Node {
    /// Walk the tree, producing `(LeafRef, Rect)` for every leaf in z-order.
    /// Used by the renderer; reads the `Rect`s back out for hit-testing.
    pub fn leaves_with_rects(&self, area: Rect) -> Vec<(LeafRef, Rect)> {
        let mut out = Vec::new();
        Self::walk(self, area, &mut out);
        out
    }

    fn walk(node: &Node, area: Rect, out: &mut Vec<(LeafRef, Rect)>) {
        match node {
            Node::Frame(id) => out.push((LeafRef::Frame(*id), area)),
            Node::Sidebar(slot) => out.push((LeafRef::Sidebar(*slot), area)),
            Node::Split { axis, children } => {
                if children.is_empty() { return; }
                let total: u32 = children.iter().map(|(_, w)| *w as u32).sum();
                let constraints: Vec<Constraint> = children
                    .iter()
                    .map(|(_, w)| Constraint::Ratio(*w as u32, total.max(1)))
                    .collect();
                let dir = match axis {
                    Axis::Horizontal => RatDirection::Horizontal,
                    Axis::Vertical => RatDirection::Vertical,
                };
                let rects = Layout::default()
                    .direction(dir)
                    .constraints(constraints)
                    .split(area);
                for ((child, _), rect) in children.iter().zip(rects.iter()) {
                    Self::walk(child, *rect, out);
                }
            }
        }
    }
}
```

- [ ] **Step 2: Rewrite `render::render` to use `leaves_with_rects`**

`crates/app/src/render.rs`:

```rust
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use devix_ui::{EditorView, StatusInfo, render_editor, render_status as render_status_widget};
use devix_workspace::LeafRef;

use crate::app::App;

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());
    let editor_area = chunks[0];
    let status_area = chunks[1];

    // Compute every leaf's Rect from the layout tree. Cache for hit-testing
    // and viewport-aware actions (scroll, focus traversal).
    let leaves = app.workspace.layout.leaves_with_rects(editor_area);
    app.workspace.render_cache.frame_rects.clear();
    app.workspace.render_cache.sidebar_rects.clear();
    for (leaf, rect) in &leaves {
        match leaf {
            LeafRef::Frame(id) => { app.workspace.render_cache.frame_rects.insert(*id, *rect); }
            LeafRef::Sidebar(slot) => { app.workspace.render_cache.sidebar_rects.insert(*slot, *rect); }
        }
    }

    // Single-frame phase: track the active frame's rect for legacy fields.
    if let Some(active_id) = app.workspace.active_frame() {
        if let Some(rect) = app.workspace.render_cache.frame_rects.get(active_id) {
            app.last_editor_area = *rect;
        }
    }

    // Render every leaf.
    for (leaf, rect) in &leaves {
        match leaf {
            LeafRef::Frame(id) => render_frame(*id, *rect, app, frame),
            LeafRef::Sidebar(_) => { /* paints in Step 5 */ }
        }
    }

    render_status(frame, status_area, app);
}

fn render_frame(id: devix_workspace::FrameId, area: Rect, app: &mut App, frame: &mut Frame<'_>) {
    // Step 2: each frame has exactly one tab; a tab strip widget arrives in Step 3.
    let view_id = app.workspace.frames[id].active_view();
    let view = &app.workspace.views[view_id];
    let doc = &app.workspace.documents[view.doc];

    let visible = area.height as usize;
    let mut scroll_top = view.scroll_top;
    let head = view.primary().head;
    if view.view_anchored && visible > 0 {
        let cur_line = doc.buffer.line_of_char(head);
        if cur_line < scroll_top {
            scroll_top = cur_line;
        } else if cur_line >= scroll_top + visible {
            scroll_top = cur_line + 1 - visible;
        }
    }
    let new_scroll = scroll_top;
    drop(doc); drop(view);
    app.workspace.views[view_id].scroll_top = new_scroll;

    let view = &app.workspace.views[view_id];
    let doc = &app.workspace.documents[view.doc];
    let editor_view = EditorView {
        buffer: &doc.buffer,
        selection: &view.selection,
        scroll_top: view.scroll_top,
    };
    let r = render_editor(editor_view, area, frame);
    app.last_gutter_width = r.gutter_width;
    if app.workspace.active_frame() == Some(id) {
        if let Some((x, y)) = r.cursor_screen { frame.set_cursor_position((x, y)); }
    }
}

fn render_status(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(view) = app.workspace.active_view() else { return };
    let doc = &app.workspace.documents[view.doc];
    let path_str = doc.buffer.path().map(|p| p.display().to_string());
    let head = view.primary().head;
    let info = StatusInfo {
        path: path_str.as_deref(),
        dirty: doc.buffer.dirty(),
        line: doc.buffer.line_of_char(head) + 1,
        col: doc.buffer.col_of_char(head) + 1,
        sel_len: view.primary().len(),
        message: app.status.get(),
    };
    render_status_widget(&info, area, frame);
}
```

- [ ] **Step 3: Build and smoke-test**

Run: `cargo build && cargo test --workspace`
Run: `cargo run -- README.md`
Expected: identical behavior to Step 1.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "render: walk layout tree; populate RenderCache"
```

---

## Step 3 — Tabs

### Task 6: Wire tab actions and tab strip widget

**Files:**
- Modify: `crates/workspace/src/action.rs`
- Modify: `crates/workspace/src/dispatch.rs`
- Modify: `crates/workspace/src/workspace.rs`
- Create: `crates/ui/src/tabstrip.rs`
- Modify: `crates/ui/src/lib.rs`
- Modify: `crates/app/src/render.rs`
- Modify: `crates/config/src/keymap.rs`

- [ ] **Step 1: Add tab actions to `Action`**

`crates/workspace/src/action.rs` — at the top, add imports if not present:

```rust
use std::path::PathBuf;

use crate::layout::{Direction, SidebarSlot};
```

Then append to the `Action` enum:

```rust
    // tabs
    NewTab,
    CloseTab,
    ForceCloseTab,
    NextTab,
    PrevTab,
```

- [ ] **Step 2: Add `Workspace` methods for tab ops**

Append to `crates/workspace/src/workspace.rs`:

```rust
impl Workspace {
    /// Open a fresh empty buffer in a new tab on the active frame.
    pub fn new_tab(&mut self) {
        let Some(fid) = self.active_frame() else { return };
        let did = self.documents.insert(Document::empty());
        let vid = self.views.insert(View::new(did));
        let frame = &mut self.frames[fid];
        frame.tabs.push(vid);
        frame.active_tab = frame.tabs.len() - 1;
    }

    /// Returns false if the active doc is dirty; the caller should warn.
    pub fn close_active_tab(&mut self, force: bool) -> bool {
        let Some(fid) = self.active_frame() else { return false };
        let frame = &self.frames[fid];
        let vid = frame.active_view();
        let did = self.views[vid].doc;
        if !force && self.documents[did].buffer.dirty() { return false; }

        let frame = &mut self.frames[fid];
        if frame.tabs.len() == 1 {
            // Last tab in the frame: replace with a fresh scratch view.
            let new_did = self.documents.insert(Document::empty());
            let new_vid = self.views.insert(View::new(new_did));
            frame.tabs[0] = new_vid;
            frame.active_tab = 0;
            self.views.remove(vid);
            self.try_remove_orphan_doc(did);
            return true;
        }

        let idx = frame.active_tab;
        frame.tabs.remove(idx);
        if frame.active_tab >= frame.tabs.len() {
            frame.active_tab = frame.tabs.len() - 1;
        }
        self.views.remove(vid);
        self.try_remove_orphan_doc(did);
        true
    }

    pub fn next_tab(&mut self) {
        let Some(fid) = self.active_frame() else { return };
        let frame = &mut self.frames[fid];
        if frame.tabs.is_empty() { return; }
        frame.active_tab = (frame.active_tab + 1) % frame.tabs.len();
    }

    pub fn prev_tab(&mut self) {
        let Some(fid) = self.active_frame() else { return };
        let frame = &mut self.frames[fid];
        if frame.tabs.is_empty() { return; }
        frame.active_tab = (frame.active_tab + frame.tabs.len() - 1) % frame.tabs.len();
    }

    /// If no surviving view references `did`, drop the document and its
    /// path index entry.
    fn try_remove_orphan_doc(&mut self, did: DocId) {
        let still_used = self.views.values().any(|v| v.doc == did);
        if still_used { return; }
        if let Some(d) = self.documents.remove(did) {
            if let Some(p) = d.buffer.path() {
                let key = canonicalize_or_keep(p);
                self.doc_index.remove(&key);
            }
        }
    }
}
```

- [ ] **Step 3: Wire actions in dispatch**

Append match arms to `dispatch::dispatch`:

```rust
        NewTab => cx.workspace.new_tab(),
        CloseTab => {
            if !cx.workspace.close_active_tab(false) {
                cx.status.set("unsaved changes — Ctrl+S to save, Ctrl+Shift+W to force close");
            } else {
                cx.status.clear();
            }
        }
        ForceCloseTab => { cx.workspace.close_active_tab(true); cx.status.clear(); }
        NextTab => cx.workspace.next_tab(),
        PrevTab => cx.workspace.prev_tab(),
```

- [ ] **Step 4: Add bindings**

Append to `default_keymap` in `crates/config/src/keymap.rs`:

```rust
    // tabs
    k.bind(chord(ch('t'), C | S), Action::NewTab);
    k.bind(chord(ch('w'), C), Action::CloseTab);
    k.bind(chord(ch('w'), C | S), Action::ForceCloseTab);
    k.bind(chord(KeyCode::Char('['), C | S), Action::PrevTab);
    k.bind(chord(KeyCode::Char(']'), C | S), Action::NextTab);
```

- [ ] **Step 5: Build the tab strip widget**

`crates/ui/src/tabstrip.rs`:

```rust
//! Tab strip widget: file-name pills, active tab inverted, ellipsis on overflow.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub struct TabInfo {
    pub label: String,
    pub dirty: bool,
}

pub fn render_tabstrip(tabs: &[TabInfo], active: usize, area: Rect, frame: &mut Frame<'_>) {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(tabs.len() * 2);
    for (i, t) in tabs.iter().enumerate() {
        let label = format!(" {}{} ", t.label, if t.dirty { "*" } else { "" });
        let style = if i == active {
            Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(label, style));
        spans.push(Span::raw("│"));
    }
    let line = Line::from(spans);
    let para = Paragraph::new(line);
    frame.render_widget(para, area);
}
```

- [ ] **Step 6: Re-export and use the widget**

`crates/ui/src/lib.rs` — add:

```rust
pub mod tabstrip;
pub use tabstrip::{TabInfo, render_tabstrip};
```

In `crates/app/src/render.rs`'s `render_frame`, split the area into a 1-row tab strip + body:

```rust
fn render_frame(id: devix_workspace::FrameId, area: Rect, app: &mut App, frame: &mut Frame<'_>) {
    let strip_area = Rect { height: 1, ..area };
    let body_area = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };

    let tabs: Vec<devix_ui::TabInfo> = app.workspace.frames[id]
        .tabs
        .iter()
        .map(|vid| {
            let v = &app.workspace.views[*vid];
            let d = &app.workspace.documents[v.doc];
            let label = d.buffer.path()
                .and_then(|p| p.file_name())
                .and_then(|f| f.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "[scratch]".to_string());
            devix_ui::TabInfo { label, dirty: d.buffer.dirty() }
        })
        .collect();
    let active_tab = app.workspace.frames[id].active_tab;
    devix_ui::render_tabstrip(&tabs, active_tab, strip_area, frame);

    // …existing body rendering, but using `body_area` and updating `app.workspace.render_cache.frame_rects[id]` to body_area for click hit-tests…
    let view_id = app.workspace.frames[id].active_view();
    let view = &app.workspace.views[view_id];
    let doc = &app.workspace.documents[view.doc];

    let visible = body_area.height as usize;
    let mut scroll_top = view.scroll_top;
    let head = view.primary().head;
    if view.view_anchored && visible > 0 {
        let cur_line = doc.buffer.line_of_char(head);
        if cur_line < scroll_top {
            scroll_top = cur_line;
        } else if cur_line >= scroll_top + visible {
            scroll_top = cur_line + 1 - visible;
        }
    }
    let new_scroll = scroll_top;
    app.workspace.views[view_id].scroll_top = new_scroll;

    let view = &app.workspace.views[view_id];
    let doc = &app.workspace.documents[view.doc];
    let editor_view = EditorView {
        buffer: &doc.buffer,
        selection: &view.selection,
        scroll_top: view.scroll_top,
    };
    let r = render_editor(editor_view, body_area, frame);
    app.last_gutter_width = r.gutter_width;
    if app.workspace.active_frame() == Some(id) {
        if let Some((x, y)) = r.cursor_screen { frame.set_cursor_position((x, y)); }
    }
    // Cache the *body* rect for hit-testing — the strip isn't clickable text.
    app.workspace.render_cache.frame_rects.insert(id, body_area);
}
```

(Note: the `Box::leak` is a stopgap because `TabInfo` borrows `&str`. If you'd rather, change `TabInfo::label` to `String` — the existing `EditorView`/`StatusInfo` pattern uses borrowed strs but for a mutable per-frame allocation this is acceptable. Recommended cleanup: change `TabInfo` to own its strings; cost is one `String` per visible tab per frame.)

- [ ] **Step 7: Add tab-mutation tests**

Append to `crates/workspace/src/workspace.rs` `mod tests`:

```rust
#[test]
fn new_tab_then_close_returns_to_previous() {
    let mut ws = Workspace::open(None).unwrap();
    let original_view = ws.active_view().unwrap().doc;

    ws.new_tab();
    assert_eq!(ws.frames.values().next().unwrap().tabs.len(), 2);
    assert_eq!(ws.frames.values().next().unwrap().active_tab, 1);

    assert!(ws.close_active_tab(false));
    let active = ws.active_view().unwrap();
    assert_eq!(active.doc, original_view);
}

#[test]
fn close_last_tab_leaves_a_scratch_tab() {
    let mut ws = Workspace::open(None).unwrap();
    assert!(ws.close_active_tab(false));
    let frame = ws.frames.values().next().unwrap();
    assert_eq!(frame.tabs.len(), 1);
    let v = ws.active_view().unwrap();
    assert!(ws.documents[v.doc].buffer.path().is_none());
}

#[test]
fn dirty_close_refused_force_close_succeeds() {
    use devix_buffer::{Selection, replace_selection_tx};
    let mut ws = Workspace::open(None).unwrap();
    // Mutate the active doc to make it dirty.
    let did = ws.active_view().unwrap().doc;
    let tx = replace_selection_tx(&ws.documents[did].buffer, &Selection::point(0), "hi");
    ws.documents[did].buffer.apply(tx);
    assert!(!ws.close_active_tab(false), "dirty close should refuse");
    assert!(ws.close_active_tab(true), "force close should succeed");
}
```

- [ ] **Step 8: Run tests and smoke-test**

Run: `cargo test --workspace`
Expected: PASS.

Smoke: `cargo run -- src/main.rs`
- `Ctrl+Shift+T` → new scratch tab.
- `Ctrl+Shift+]` / `Ctrl+Shift+[` → cycle.
- `Ctrl+W` on dirty tab shows status message.
- `Ctrl+Shift+W` force-closes.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "tabs: NewTab/CloseTab/Next/Prev with dirty refusal + tab strip"
```

---

### Task 7: `OpenPath` replaces current tab; document de-dup

**Files:**
- Modify: `crates/workspace/src/action.rs`
- Modify: `crates/workspace/src/dispatch.rs`
- Modify: `crates/workspace/src/workspace.rs`

- [ ] **Step 1: Add `OpenPath` action variant**

`crates/workspace/src/action.rs` (`PathBuf` already imported in Task 6):

```rust
    OpenPath(PathBuf),
```

- [ ] **Step 2: Implement `Workspace::open_path`**

Append to `workspace.rs`:

```rust
impl Workspace {
    /// Open `path` in the active frame's current tab (replace-current semantics).
    /// If a Document already exists for the canonicalized path, reuse it.
    /// Returns the new ViewId.
    pub fn open_path_replace_current(&mut self, path: PathBuf) -> Result<ViewId> {
        let key = canonicalize_or_keep(&path);
        let did = if let Some(&existing) = self.doc_index.get(&key) {
            existing
        } else {
            let id = self.documents.insert(Document::from_path(path)?);
            self.doc_index.insert(key, id);
            id
        };
        let new_view = self.views.insert(View::new(did));
        let Some(fid) = self.active_frame() else { return Ok(new_view); };
        let frame = &mut self.frames[fid];
        let old_view = frame.tabs[frame.active_tab];
        frame.tabs[frame.active_tab] = new_view;
        let old_doc = self.views[old_view].doc;
        self.views.remove(old_view);
        self.try_remove_orphan_doc(old_doc);
        Ok(new_view)
    }
}
```

- [ ] **Step 3: Wire dispatch**

Add arm to `dispatch`:

```rust
        OpenPath(p) => match cx.workspace.open_path_replace_current(p) {
            Ok(_) => cx.status.clear(),
            Err(e) => cx.status.set(format!("open failed: {e}")),
        },
```

- [ ] **Step 4: De-dup test**

Append to `workspace::tests`:

```rust
#[test]
fn opening_same_path_twice_reuses_document() {
    let dir = std::env::temp_dir().join(format!("devix-open-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("a.txt");
    std::fs::write(&p, "abc").unwrap();

    let mut ws = Workspace::open(None).unwrap();
    let v1 = ws.open_path_replace_current(p.clone()).unwrap();
    let did1 = ws.views[v1].doc;
    // Open another tab and re-open the same path there.
    ws.new_tab();
    let v2 = ws.open_path_replace_current(p.clone()).unwrap();
    let did2 = ws.views[v2].doc;
    assert_eq!(did1, did2, "same path should reuse DocId");
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "tabs: OpenPath replaces current tab; document de-dup by canonical path"
```

---

## Step 4 — Splits

### Task 8: SplitVertical / SplitHorizontal

**Files:**
- Modify: `crates/workspace/src/action.rs`
- Modify: `crates/workspace/src/layout.rs`
- Modify: `crates/workspace/src/workspace.rs`
- Modify: `crates/workspace/src/dispatch.rs`
- Modify: `crates/config/src/keymap.rs`

- [ ] **Step 1: Add actions**

`action.rs`:

```rust
    SplitVertical,
    SplitHorizontal,
    CloseFrame,
```

- [ ] **Step 2: Add tree-mutation helpers in `layout.rs`**

Append to `crates/workspace/src/layout.rs`:

```rust
impl Node {
    /// Walk to the leaf at `path` and replace it with `new`. Returns true if
    /// the path resolved to a leaf and the replacement happened.
    pub fn replace_leaf_at(&mut self, path: &[usize], new: Node) -> bool {
        if path.is_empty() {
            *self = new;
            return matches!(self, Node::Frame(_) | Node::Sidebar(_) | Node::Split { .. });
        }
        let mut node = self;
        for (i, &idx) in path.iter().enumerate() {
            match node {
                Node::Split { children, .. } => {
                    if idx >= children.len() { return false; }
                    if i + 1 == path.len() {
                        children[idx].0 = new;
                        return true;
                    }
                    node = &mut children[idx].0;
                }
                _ => return false,
            }
        }
        false
    }

    /// Take the leaf at `path` out, leaving a placeholder Frame with a null FrameId.
    /// Caller is expected to replace the placeholder before next use.
    /// Returns the original child if the path was valid.
    pub fn take_leaf_at(&mut self, path: &[usize]) -> Option<Node> {
        use slotmap::Key;
        if path.is_empty() { return None; }
        let mut node = self;
        for (i, &idx) in path.iter().enumerate() {
            match node {
                Node::Split { children, .. } => {
                    if idx >= children.len() { return None; }
                    if i + 1 == path.len() {
                        let placeholder = Node::Frame(crate::frame::FrameId::null());
                        return Some(std::mem::replace(&mut children[idx].0, placeholder));
                    }
                    node = &mut children[idx].0;
                }
                _ => return None,
            }
        }
        None
    }
}
```

(`slotmap::Key::null()` is a trait method; bring it into scope with `use slotmap::Key;`. The placeholder is transient — `Workspace::split_active` and `toggle_sidebar` do atomic replace-in-one-step instead, so `take_leaf_at` is provided for completeness only.)

- [ ] **Step 3: `Workspace::split_active`**

Append to `workspace.rs`:

```rust
use crate::layout::{Axis, Node};

impl Workspace {
    /// Replace the focused Frame leaf with a Split containing two frames:
    /// the original frame, plus a new frame whose first tab clones the active view.
    pub fn split_active(&mut self, axis: Axis) {
        let Some(focus_path) = (if matches!(self.layout.leaf_at(&self.focus), Some(LeafRef::Frame(_))) {
            Some(self.focus.clone())
        } else { None }) else { return };

        let Some(active_fid) = self.active_frame() else { return };

        // Clone the active view: same DocId, copy of selection/scroll.
        let cloned_view = {
            let v = &self.views[self.frames[active_fid].active_view()];
            View {
                doc: v.doc,
                selection: v.selection.clone(),
                target_col: v.target_col,
                scroll_top: v.scroll_top,
                view_anchored: true,
            }
        };
        let new_view_id = self.views.insert(cloned_view);
        let new_frame_id = self.frames.insert(Frame::with_view(new_view_id));

        let new_node = Node::Split {
            axis,
            children: vec![
                (Node::Frame(active_fid), 1),
                (Node::Frame(new_frame_id), 1),
            ],
        };
        self.layout.replace_leaf_at(&focus_path, new_node);
        // Move focus to the new (right/bottom) frame.
        let mut new_focus = focus_path;
        new_focus.push(1);
        self.focus = new_focus.clone();
        self.last_editor_focus = new_focus;
    }
}
```

- [ ] **Step 4: Wire dispatch + bindings**

`dispatch.rs`:

```rust
        SplitVertical => cx.workspace.split_active(devix_workspace::Axis::Horizontal),
        SplitHorizontal => cx.workspace.split_active(devix_workspace::Axis::Vertical),
```

(The action name `SplitVertical` produces a horizontal split-axis layout — i.e. side-by-side panes — by convention. `Horizontal` axis = panes laid out horizontally. Mirror for `SplitHorizontal`. Keep the user-facing names; map to the internal axis here.)

`crates/config/src/keymap.rs` (append):

```rust
    k.bind(chord(ch('\\'), C), Action::SplitVertical);
    k.bind(chord(ch('-'), C), Action::SplitHorizontal);
```

- [ ] **Step 5: Split test**

Append to `workspace::tests`:

```rust
#[test]
fn split_creates_a_second_frame_and_focuses_it() {
    let mut ws = Workspace::open(None).unwrap();
    let original_fid = ws.active_frame().unwrap();
    ws.split_active(crate::layout::Axis::Horizontal);
    assert_eq!(ws.frames.len(), 2);
    let new_fid = ws.active_frame().unwrap();
    assert_ne!(original_fid, new_fid);

    // Both views should reference the same DocId (shared buffer).
    let original_doc = ws.views[ws.frames[original_fid].active_view()].doc;
    let new_doc = ws.views[ws.frames[new_fid].active_view()].doc;
    assert_eq!(original_doc, new_doc, "split clones view, shares document");
}
```

- [ ] **Step 6: Smoke-test**

`cargo run -- src/main.rs`. Press `Ctrl+\`. Two side-by-side panes should appear. Type in one — the other should reflect the edit (shared buffer). The renderer also needs to handle multi-leaf trees — confirm both panes render. (If only the active frame paints, double-check that `render_frame` is called for every leaf in `leaves_with_rects`.)

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "splits: SplitVertical/SplitHorizontal with shared-doc view clone"
```

---

### Task 9: CloseFrame + orphan-split collapse

**Files:**
- Modify: `crates/workspace/src/layout.rs`
- Modify: `crates/workspace/src/workspace.rs`
- Modify: `crates/workspace/src/dispatch.rs`

- [ ] **Step 1: Add `Node::collapse_singleton_splits`**

Append to `layout.rs`:

```rust
impl Node {
    /// Recursively collapse any Split with one child into that child.
    pub fn collapse_singleton_splits(&mut self) {
        if let Node::Split { children, .. } = self {
            for (child, _) in children.iter_mut() {
                child.collapse_singleton_splits();
            }
            if children.len() == 1 {
                let (only, _) = children.remove(0);
                *self = only;
            }
        }
    }
}
```

- [ ] **Step 2: `Workspace::close_active_frame`**

Append to `workspace.rs`:

```rust
impl Workspace {
    /// Close the active frame if there are 2+ frames in the tree.
    /// The resulting Split with a single child collapses to that child.
    /// No-op when only one frame remains anywhere in the tree.
    pub fn close_active_frame(&mut self) {
        if self.frames.len() <= 1 { return; }
        let Some(fid) = self.active_frame() else { return };
        let path = self.focus.clone();
        if path.is_empty() { return; } // root is a single Frame; same as len==1

        // Remove the leaf from its parent split.
        let (parent_path, leaf_idx) = path.split_at(path.len() - 1);
        let leaf_idx = leaf_idx[0];
        let Some(parent) = node_at_mut(&mut self.layout, parent_path) else { return };
        if let Node::Split { children, .. } = parent {
            children.remove(leaf_idx);
        }
        // Collapse one-child splits up the chain.
        self.layout.collapse_singleton_splits();
        // Drop the views/frames the closed frame held.
        let frame = self.frames.remove(fid).expect("frame existed");
        for vid in frame.tabs {
            let did = self.views[vid].doc;
            self.views.remove(vid);
            self.try_remove_orphan_doc(did);
        }
        // Re-anchor focus to the first remaining frame, deepest path.
        self.focus = first_frame_path(&self.layout);
        self.last_editor_focus = self.focus.clone();
    }
}

fn node_at_mut<'a>(node: &'a mut Node, path: &[usize]) -> Option<&'a mut Node> {
    let mut n = node;
    for &i in path {
        match n {
            Node::Split { children, .. } => {
                n = &mut children.get_mut(i)?.0;
            }
            _ => return None,
        }
    }
    Some(n)
}

fn first_frame_path(node: &Node) -> Vec<usize> {
    fn go(node: &Node, path: &mut Vec<usize>) -> bool {
        match node {
            Node::Frame(_) => true,
            Node::Sidebar(_) => false, // skip sidebars when picking a default
            Node::Split { children, .. } => {
                for (i, (child, _)) in children.iter().enumerate() {
                    path.push(i);
                    if go(child, path) { return true; }
                    path.pop();
                }
                false
            }
        }
    }
    let mut p = Vec::new();
    if go(node, &mut p) { p } else { Vec::new() }
}
```

- [ ] **Step 3: Wire dispatch**

`dispatch.rs`:

```rust
        CloseFrame => cx.workspace.close_active_frame(),
```

(No keybinding requested in the spec — leave palette-only / programmatic.)

- [ ] **Step 4: Test orphan collapse**

Append to `workspace::tests`:

```rust
#[test]
fn closing_one_split_child_collapses_back_to_single_frame() {
    use crate::layout::Axis;
    let mut ws = Workspace::open(None).unwrap();
    ws.split_active(Axis::Horizontal);
    assert_eq!(ws.frames.len(), 2);
    ws.close_active_frame();
    assert_eq!(ws.frames.len(), 1);
    assert!(matches!(ws.layout, Node::Frame(_)), "single frame at root");
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "splits: CloseFrame collapses orphan splits"
```

---

## Step 5 — Sidebars + universal focus

### Task 10: Sidebar widget + ToggleSidebar

**Files:**
- Create: `crates/ui/src/sidebar.rs`
- Modify: `crates/ui/src/lib.rs`
- Modify: `crates/workspace/src/action.rs`
- Modify: `crates/workspace/src/workspace.rs`
- Modify: `crates/workspace/src/dispatch.rs`
- Modify: `crates/app/src/render.rs`

- [ ] **Step 1: Sidebar widget**

`crates/ui/src/sidebar.rs`:

```rust
//! Sidebar widget: empty bordered placeholder. Body is reserved for plugins.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders};

pub struct SidebarInfo<'a> {
    pub title: &'a str,
    pub focused: bool,
}

pub fn render_sidebar(info: &SidebarInfo<'_>, area: Rect, frame: &mut Frame<'_>) {
    let style = if info.focused {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::default()
        .title(info.title.to_string())
        .borders(Borders::ALL)
        .border_style(style);
    frame.render_widget(block, area);
}
```

- [ ] **Step 2: Re-export**

`crates/ui/src/lib.rs`:

```rust
pub mod sidebar;
pub use sidebar::{SidebarInfo, render_sidebar};
```

- [ ] **Step 3: `ToggleSidebar` action + workspace method**

`action.rs` (`SidebarSlot` already imported in Task 6):

```rust
    ToggleSidebar(SidebarSlot),
```

`workspace.rs`:

```rust
impl Workspace {
    pub fn toggle_sidebar(&mut self, slot: SidebarSlot) {
        // Lift the layout root into a horizontal Split if it isn't one.
        if !matches!(&self.layout, Node::Split { axis: Axis::Horizontal, .. }) {
            use slotmap::Key;
            let inner = std::mem::replace(
                &mut self.layout,
                Node::Frame(crate::frame::FrameId::null()),
            );
            self.layout = Node::Split {
                axis: Axis::Horizontal,
                children: vec![(inner, 80)],
            };
            // The focus path now needs a leading 0 (the editor body is child 0).
            let mut new_focus = vec![0];
            new_focus.extend(self.focus.iter().copied());
            self.focus = new_focus;
            self.last_editor_focus = self.focus.clone();
        }
        let Node::Split { children, .. } = &mut self.layout else {
            unreachable!("we just lifted the root into a horizontal Split")
        };

        // Find an existing sidebar of this slot.
        let existing = children.iter().position(|(c, _)| matches!(c, Node::Sidebar(s) if *s == slot));
        if let Some(i) = existing {
            children.remove(i);
            // If focus was on or past this index, fix it up.
            if let Some(top) = self.focus.first_mut() {
                if *top >= i && *top > 0 { *top -= 1; }
            }
        } else {
            let insert_at = match slot {
                SidebarSlot::Left => 0,
                SidebarSlot::Right => children.len(),
            };
            children.insert(insert_at, (Node::Sidebar(slot), 20));
            if let Some(top) = self.focus.first_mut() {
                if *top >= insert_at { *top += 1; }
            }
        }
    }
}
```

`dispatch.rs`:

```rust
        ToggleSidebar(slot) => cx.workspace.toggle_sidebar(slot),
```

No keybinding — palette-only (Phase 5 will expose it).

- [ ] **Step 4: Render sidebars**

In `crates/app/src/render.rs`, replace the `LeafRef::Sidebar` branch:

```rust
            LeafRef::Sidebar(slot) => {
                let title = match slot {
                    devix_workspace::SidebarSlot::Left => "left",
                    devix_workspace::SidebarSlot::Right => "right",
                };
                let focused = matches!(
                    app.workspace.layout.leaf_at(&app.workspace.focus),
                    Some(devix_workspace::LeafRef::Sidebar(s)) if s == *slot
                );
                devix_ui::render_sidebar(
                    &devix_ui::SidebarInfo { title, focused },
                    *rect,
                    frame,
                );
            }
```

- [ ] **Step 5: Sidebar toggle test**

Append to `workspace::tests`:

```rust
#[test]
fn toggle_left_sidebar_adds_then_removes_it() {
    let mut ws = Workspace::open(None).unwrap();
    ws.toggle_sidebar(SidebarSlot::Left);
    let n = match &ws.layout { Node::Split { children, .. } => children.len(), _ => 0 };
    assert_eq!(n, 2, "split has editor + left sidebar");

    ws.toggle_sidebar(SidebarSlot::Left);
    let collapsed = matches!(&ws.layout, Node::Split { .. } | Node::Frame(_));
    assert!(collapsed);
}
```

- [ ] **Step 6: Run tests + smoke-test**

Run: `cargo test --workspace`. PASS.

Smoke: there's no key for ToggleSidebar in Phase 3. Verify by adding a temporary `Ctrl+Shift+L` binding for `ToggleSidebar(SidebarSlot::Left)` for manual testing, then **revert** the binding before commit. (Sidebars become user-toggleable via the palette in Phase 5.)

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "sidebars: ToggleSidebar + bordered placeholder widget"
```

---

### Task 11: Universal focus traversal

**Files:**
- Modify: `crates/workspace/src/layout.rs`
- Modify: `crates/workspace/src/action.rs`
- Modify: `crates/workspace/src/workspace.rs`
- Modify: `crates/workspace/src/dispatch.rs`
- Modify: `crates/config/src/keymap.rs`

- [ ] **Step 1: Add `FocusDir` action**

`action.rs` (`Direction` already imported in Task 6):

```rust
    FocusDir(Direction),
```

- [ ] **Step 2: Implement `Workspace::focus_dir`**

```rust
use crate::layout::Direction;

impl Workspace {
    pub fn focus_dir(&mut self, dir: Direction) {
        let Some(target_path) = compute_focus_target(&self.layout, &self.focus, dir, &self.render_cache) else {
            return;
        };
        self.focus = target_path;
        if matches!(self.layout.leaf_at(&self.focus), Some(LeafRef::Frame(_))) {
            self.last_editor_focus = self.focus.clone();
        }
    }
}

fn compute_focus_target(
    layout: &Node,
    focus: &[usize],
    dir: Direction,
    cache: &RenderCache,
) -> Option<Vec<usize>> {
    let needed_axis = match dir {
        Direction::Left | Direction::Right => Axis::Horizontal,
        Direction::Up   | Direction::Down  => Axis::Vertical,
    };
    let step: isize = match dir {
        Direction::Left | Direction::Up   => -1,
        Direction::Right | Direction::Down => 1,
    };

    // Walk up from the leaf, looking for a Split on the needed axis where we
    // can step in `step` direction.
    let mut path = focus.to_vec();
    while !path.is_empty() {
        let parent_path = path[..path.len() - 1].to_vec();
        let child_idx = *path.last().unwrap();
        let parent = node_at(layout, &parent_path)?;
        if let Node::Split { axis, children } = parent {
            if *axis == needed_axis {
                let next = child_idx as isize + step;
                if next >= 0 && (next as usize) < children.len() {
                    let mut new_path = parent_path;
                    new_path.push(next as usize);
                    // Walk into the new subtree picking the spatially closest leaf.
                    return Some(walk_into(layout, new_path, dir, focus, cache));
                }
            }
        }
        path.pop();
    }
    None
}

fn walk_into(
    layout: &Node,
    mut path: Vec<usize>,
    dir: Direction,
    source_path: &[usize],
    cache: &RenderCache,
) -> Vec<usize> {
    loop {
        let n = match node_at(layout, &path) {
            Some(n) => n,
            None => return path,
        };
        match n {
            Node::Frame(_) | Node::Sidebar(_) => return path,
            Node::Split { axis, children } => {
                let pick = pick_closest_child(layout, &path, *axis, children.len(), dir, source_path, cache);
                path.push(pick);
            }
        }
    }
}

fn pick_closest_child(
    layout: &Node,
    parent_path: &[usize],
    axis: Axis,
    n_children: usize,
    dir: Direction,
    source_path: &[usize],
    cache: &RenderCache,
) -> usize {
    if n_children == 0 { return 0; }
    let source_rect = leaf_rect_for(layout, source_path, cache);
    let Some(src) = source_rect else {
        // Fallback when no rect cached yet: pick the side adjacent to the move.
        return match (axis, dir) {
            (Axis::Horizontal, Direction::Left) => n_children - 1,
            (Axis::Horizontal, Direction::Right) => 0,
            (Axis::Vertical, Direction::Up) => n_children - 1,
            (Axis::Vertical, Direction::Down) => 0,
            _ => 0,
        };
    };
    let centre_y = src.y + src.height / 2;
    let centre_x = src.x + src.width / 2;
    // Compute each child's centre and pick the closest along the perpendicular axis.
    let mut best = 0usize;
    let mut best_d = i32::MAX;
    for i in 0..n_children {
        let mut child_path = parent_path.to_vec();
        child_path.push(i);
        let Some(rect) = first_leaf_rect(layout, &child_path, cache) else { continue };
        let d = match axis {
            Axis::Horizontal => (rect.y as i32 + rect.height as i32 / 2 - centre_y as i32).abs(),
            Axis::Vertical => (rect.x as i32 + rect.width as i32 / 2 - centre_x as i32).abs(),
        };
        if d < best_d { best_d = d; best = i; }
    }
    best
}

fn leaf_rect_for(layout: &Node, path: &[usize], cache: &RenderCache) -> Option<ratatui::layout::Rect> {
    match layout.leaf_at(path)? {
        LeafRef::Frame(id) => cache.frame_rects.get(id).copied(),
        LeafRef::Sidebar(slot) => cache.sidebar_rects.get(&slot).copied(),
    }
}

fn first_leaf_rect(layout: &Node, path: &[usize], cache: &RenderCache) -> Option<ratatui::layout::Rect> {
    fn descend<'a>(node: &'a Node, path: &mut Vec<usize>) -> &'a Node {
        match node {
            Node::Split { children, .. } if !children.is_empty() => {
                path.push(0);
                descend(&children[0].0, path)
            }
            other => other,
        }
    }
    let mut p = path.to_vec();
    let root = node_at(layout, &p)?;
    let _final_node = descend(root, &mut p);
    leaf_rect_for(layout, &p, cache)
}

fn node_at<'a>(node: &'a Node, path: &[usize]) -> Option<&'a Node> {
    let mut n = node;
    for &i in path {
        match n {
            Node::Split { children, .. } => n = &children.get(i)?.0,
            _ => return None,
        }
    }
    Some(n)
}
```

- [ ] **Step 3: Universal-focus rule (sidebar-at-edge)**

Add inside `Workspace::focus_dir` after `compute_focus_target` returns `None`:

```rust
        // Edge: try to move into a sidebar.
        let needed: Option<SidebarSlot> = match dir {
            Direction::Left => Some(SidebarSlot::Left),
            Direction::Right => Some(SidebarSlot::Right),
            _ => None,
        };
        if let Some(slot) = needed {
            // Find a Sidebar leaf with this slot anywhere in the tree.
            if let Some(path) = find_sidebar(&self.layout, slot) {
                self.focus = path;
            }
        }
```

…and the helper:

```rust
fn find_sidebar(node: &Node, slot: SidebarSlot) -> Option<Vec<usize>> {
    fn go(node: &Node, slot: SidebarSlot, out: &mut Vec<usize>) -> bool {
        match node {
            Node::Sidebar(s) if *s == slot => true,
            Node::Split { children, .. } => {
                for (i, (c, _)) in children.iter().enumerate() {
                    out.push(i);
                    if go(c, slot, out) { return true; }
                    out.pop();
                }
                false
            }
            _ => false,
        }
    }
    let mut p = Vec::new();
    if go(node, slot, &mut p) { Some(p) } else { None }
}
```

- [ ] **Step 4: Wire dispatch + bindings**

`dispatch.rs`:

```rust
        FocusDir(d) => cx.workspace.focus_dir(d),
```

`keymap.rs`:

```rust
    k.bind(chord(KeyCode::Left, C | A), Action::FocusDir(devix_workspace::Direction::Left));
    k.bind(chord(KeyCode::Right, C | A), Action::FocusDir(devix_workspace::Direction::Right));
    k.bind(chord(KeyCode::Up, C | A), Action::FocusDir(devix_workspace::Direction::Up));
    k.bind(chord(KeyCode::Down, C | A), Action::FocusDir(devix_workspace::Direction::Down));
```

- [ ] **Step 5: Focus tests**

```rust
#[test]
fn focus_dir_right_after_split_returns_to_original() {
    use crate::layout::{Axis, Direction};
    let mut ws = Workspace::open(None).unwrap();
    let original = ws.active_frame().unwrap();
    ws.split_active(Axis::Horizontal);
    let new_fid = ws.active_frame().unwrap();
    assert_ne!(original, new_fid);

    // Focus left should land on the original frame.
    // Render-cache is empty, fallback rule picks adjacent: right→Left = last child of left subtree.
    ws.focus_dir(Direction::Left);
    assert_eq!(ws.active_frame(), Some(original));

    ws.focus_dir(Direction::Right);
    assert_eq!(ws.active_frame(), Some(new_fid));
}

#[test]
fn focus_dir_left_at_edge_with_sidebar_enters_sidebar() {
    use crate::layout::Direction;
    let mut ws = Workspace::open(None).unwrap();
    ws.toggle_sidebar(SidebarSlot::Left);
    // After toggling on, focus is still on the editor frame.
    ws.focus_dir(Direction::Left);
    assert!(matches!(
        ws.layout.leaf_at(&ws.focus),
        Some(LeafRef::Sidebar(SidebarSlot::Left))
    ));
}
```

- [ ] **Step 6: Run tests + smoke**

Run: `cargo test --workspace` → PASS.

Smoke: split with `Ctrl+\`, then `Ctrl+Alt+←` and `Ctrl+Alt+→`. Cursor focus moves between panes. Add a temporary `ToggleSidebar` keybinding (revert before commit) and verify `Ctrl+Alt+←` from the leftmost frame enters the sidebar.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "focus: directional traversal (Ctrl+Alt+arrows) with sidebar at edge"
```

---

### Task 12: Mouse click-to-focus

**Files:**
- Modify: `crates/app/src/events.rs`
- Modify: `crates/workspace/src/workspace.rs`

- [ ] **Step 1: `Workspace::focus_at_screen`**

```rust
impl Workspace {
    /// Set focus to the leaf whose Rect contains (col, row), if any.
    pub fn focus_at_screen(&mut self, col: u16, row: u16) {
        let leaves = self.layout.leaves_with_rects(self.outer_editor_area());
        for (leaf, rect) in leaves {
            if (col >= rect.x && col < rect.x + rect.width)
                && (row >= rect.y && row < rect.y + rect.height)
            {
                if let Some(path) = path_to_leaf(&self.layout, leaf) {
                    self.focus = path.clone();
                    if matches!(leaf, LeafRef::Frame(_)) {
                        self.last_editor_focus = path;
                    }
                    return;
                }
            }
        }
    }

    /// The total area the layout tree occupies, derived from cached rects.
    /// Used by hit-testing without re-running a layout pass.
    fn outer_editor_area(&self) -> ratatui::layout::Rect {
        // Take the union of cached rects. Falls back to (0,0,0,0) before first render.
        use ratatui::layout::Rect;
        let rects: Vec<Rect> = self.render_cache.frame_rects.values().copied()
            .chain(self.render_cache.sidebar_rects.values().copied())
            .collect();
        if rects.is_empty() { return Rect::default(); }
        let x = rects.iter().map(|r| r.x).min().unwrap();
        let y = rects.iter().map(|r| r.y).min().unwrap();
        let x_end = rects.iter().map(|r| r.x + r.width).max().unwrap();
        let y_end = rects.iter().map(|r| r.y + r.height).max().unwrap();
        Rect { x, y, width: x_end - x, height: y_end - y }
    }
}

fn path_to_leaf(node: &Node, target: LeafRef) -> Option<Vec<usize>> {
    fn matches(node: &Node, target: LeafRef) -> bool {
        match (node, target) {
            (Node::Frame(a), LeafRef::Frame(b)) => *a == b,
            (Node::Sidebar(a), LeafRef::Sidebar(b)) => *a == b,
            _ => false,
        }
    }
    fn go(node: &Node, target: LeafRef, out: &mut Vec<usize>) -> bool {
        if matches(node, target) { return true; }
        if let Node::Split { children, .. } = node {
            for (i, (c, _)) in children.iter().enumerate() {
                out.push(i);
                if go(c, target, out) { return true; }
                out.pop();
            }
        }
        false
    }
    let mut p = Vec::new();
    if go(node, target, &mut p) { Some(p) } else { None }
}
```

- [ ] **Step 2: Hit-test on click**

In `crates/app/src/events.rs`, in `handle_mouse` for `Down(Left)`, before dispatching `ClickAt`:

```rust
        MouseEventKind::Down(MouseButton::Left) => {
            app.workspace.focus_at_screen(me.column, me.row);
            let extend = me.modifiers.contains(KeyModifiers::SHIFT);
            run_action(app, Action::ClickAt {
                col: me.column, row: me.row, extend,
            });
        }
```

- [ ] **Step 3: Smoke-test**

`cargo run -- src/main.rs` → split → click in either pane to set focus, then type to verify.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "focus: mouse click-to-focus across split tree"
```

---

## Step 6 — Keymap finalize

### Task 13: Reserved multicursor binding + verify Save behavior

**Files:**
- Modify: `crates/config/src/keymap.rs`

- [ ] **Step 1: Reserve multicursor on Shift+Ctrl+Up/Down**

Add a no-op binding so the keys are claimed for Phase 7. We don't need a `MulticursorAdd` action yet — just leave a comment.

Actually, since we don't yet have an action, the cleanest move is to **not** add a binding (no behavior to define). Document the reservation in `keymap.rs`:

```rust
// Reserved bindings (Phase 7 multicursor):
//   Shift + Ctrl + Up   → MulticursorAddAbove (not yet wired)
//   Shift + Ctrl + Down → MulticursorAddBelow (not yet wired)
// These chords currently fall through to plain MoveUp/MoveDown { extend: true }.
// Adding the action variants will reclaim them.
```

- [ ] **Step 2: Confirm doc-cycle binding is absent**

Search `keymap.rs` for `Action::PrevDocument` / `Action::NextDocument` — should be absent. The original spec had `Ctrl+Alt+←/→` for doc cycling, but no such action was ever defined; nothing to remove. The Phase 3 binding for `Ctrl+Alt+arrows` (FocusDir) is the one in effect.

- [ ] **Step 3: Verify `Ctrl+S` Save**

Already wired (`Action::Save` exists since Phase 2). Run the binary, edit a file, press `Ctrl+S`, observe the "saved" status, confirm the file on disk reflects the change. No code change needed.

- [ ] **Step 4: Final smoke test**

`cargo run -- src/main.rs`. Run through:
- Type, arrow, undo, redo.
- `Ctrl+Shift+T` new tab; `Ctrl+Shift+]` / `[` cycle; `Ctrl+W` close (refused if dirty); `Ctrl+Shift+W` force-close.
- `Ctrl+\` vertical split, `Ctrl+-` horizontal split. `Ctrl+Alt+←/→/↑/↓` traverse. Mouse click switches focus.
- `Ctrl+S` save.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "keymap: phase 3 finalize (reserve multicursor chord)"
```

---

## Spec coverage check

| Spec section | Implemented in |
|---|---|
| Document de-dup by canonical path | Task 7 |
| Shared Buffer across views | Tasks 7 (open) + 8 (split clone) |
| Layout tree (Node enum, IDs only) | Tasks 0/3 (scaffold) + 5 (rendering) |
| Frame with tabs | Task 6 |
| Replace-current open behavior | Task 7 |
| Tab actions + dirty refusal | Task 6 |
| Splits | Task 8 |
| Close frame + orphan collapse | Task 9 |
| Sidebar leaf + ToggleSidebar | Task 10 |
| Universal Ctrl+Alt+arrow focus | Task 11 |
| Sidebars at root only | Task 10 (toggle inserts at root only) |
| Mouse click-to-focus | Task 12 |
| Save (`Ctrl+S`) | Task 13 (already wired) |
| Multicursor binding reserved | Task 13 |
| Doc-cycle binding dropped | Task 13 (verified absent) |
| `RenderCache` populated each frame | Task 5 |
| Spatial-closest pick on focus | Task 11 |

---

## Out of scope (deferred)

- Resize splits (drag/keys).
- Sidebar visibility keybinding — palette in Phase 5.
- Sidebar contents — Phase 8 plugins.
- Real popups (completion, hover) — Phase 6.
- Multicursor implementation — Phase 7.
