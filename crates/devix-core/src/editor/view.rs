//! View IR producer — `Editor::view(root: Path) -> Result<View, RequestError>`.
//!
//! Walks the live `LayoutNode` tree and emits the closed `View` IR
//! per `docs/specs/frontend.md`. Every resource-bound node carries
//! its canonical `Path` (`/buf/<id>`, `/pane/<i>(/<j>)*`,
//! `/pane/.../tabstrip`, `/pane/.../sidebar/<slot>`); synthetic
//! wrapper nodes use the **deterministic-derivation** form locked
//! during T-90 (see foundations-review log 2026-05-07):
//! `/synthetic/<kind>/<encoded-parent-path>[/<suffix>]`. Same
//! logical node across two renders ⇒ same id, by construction —
//! no state, no per-parent cache. The alternative mint-and-cache
//! shape was considered and rejected at T-90: same answer for
//! "child at structural position i" without the cache state.
//!
//! Scope:
//! * `transition` is `None` everywhere — `Capability::Animations`
//!   gating lands when an animating frontend ships.
//! * Highlights are not populated by this producer at T-43 — the
//!   tree-sitter actor (Stage 8) will publish them; the View carries
//!   an empty `highlights` list for now.
//! * Pane-tree mutations to the layout (the Stage-9 unification)
//!   leave this walk unchanged: it's keyed off `LayoutNode` variants
//!   and updates trivially when those collapse.

use devix_protocol::path::{Path, PathError};
use devix_protocol::protocol::RequestError;
use devix_protocol::view::{
    Axis as ViewAxis, CursorMark, GutterMode, SidebarSlot as ViewSidebarSlot, TabItem, View,
    ViewNodeId,
};

use crate::editor::cursor::CursorId;
use crate::editor::document::Document;
use crate::editor::editor::Editor;
use crate::editor::tree::{LayoutFrame, LayoutNode, LayoutSidebar, LayoutSplit};
use crate::layout_geom::{Axis as CoreAxis, SidebarSlot as CoreSidebarSlot};

impl Editor {
    /// Produce the View IR rooted at `root`. Today the only
    /// supported root is `/pane`; other roots return
    /// `RequestError::UnknownPath`.
    pub fn view(&self, root: &Path) -> Result<View, RequestError> {
        if root.as_str() != "/pane" {
            return Err(RequestError::UnknownPath(root.clone()));
        }
        Ok(walk_layout(self.panes.root(), &[], self))
    }
}

/// Walk a `LayoutNode`, emitting `View`. `path_indices` is the
/// 0-based child position list from the layout root (e.g. `[0, 1]`
/// for `/pane/0/1`).
fn walk_layout(node: &LayoutNode, path_indices: &[usize], editor: &Editor) -> View {
    let pane_path = pane_path(path_indices);
    match node {
        LayoutNode::Split(s) => walk_split(s, path_indices, editor, &pane_path),
        LayoutNode::Frame(f) => walk_frame(f, &pane_path, editor),
        LayoutNode::Sidebar(s) => walk_sidebar(s, path_indices, editor, &pane_path),
    }
}

fn walk_split(
    split: &LayoutSplit,
    path_indices: &[usize],
    editor: &Editor,
    pane_path: &Path,
) -> View {
    let mut weights: Vec<u16> = Vec::with_capacity(split.children.len());
    let mut children: Vec<View> = Vec::with_capacity(split.children.len());
    for (i, (child_node, weight)) in split.children.iter().enumerate() {
        let mut child_indices = path_indices.to_vec();
        child_indices.push(i);
        weights.push(*weight);
        children.push(walk_layout(child_node, &child_indices, editor));
    }
    View::Split {
        id: ViewNodeId(pane_path.clone()),
        axis: map_axis(split.axis),
        weights,
        children,
        transition: None,
    }
}

fn walk_frame(frame: &LayoutFrame, pane_path: &Path, editor: &Editor) -> View {
    let tab_strip = build_tab_strip(frame, pane_path, editor);
    let body = build_active_buffer(frame, editor);
    // A frame stacks the tab strip on top of the active buffer's
    // body. The synthetic stack id is derived from the frame's path.
    let stack_id = synthetic_id("stack", pane_path, None);
    View::Stack {
        id: stack_id,
        axis: ViewAxis::Vertical,
        weights: vec![1, 1000],
        children: vec![tab_strip, body],
        spacing: 0,
        transition: None,
    }
}

fn build_tab_strip(frame: &LayoutFrame, pane_path: &Path, editor: &Editor) -> View {
    let strip_path = pane_path.join("tabstrip").expect("tabstrip is a valid segment");
    let active = frame.active_tab.try_into().unwrap_or(0u32);
    let mut tabs = Vec::with_capacity(frame.tabs.len());
    for (idx, cursor_id) in frame.tabs.iter().enumerate() {
        let tab_path = pane_path
            .join("tab")
            .and_then(|p| p.join(&idx.to_string()))
            .expect("tab path segments are valid");
        let (label, doc_path) = tab_label_and_doc(*cursor_id, editor);
        tabs.push(TabItem {
            id: ViewNodeId(tab_path),
            label,
            dirty: doc_dirty(*cursor_id, editor),
            doc: doc_path,
        });
    }
    View::TabStrip {
        id: ViewNodeId(strip_path),
        tabs,
        active,
    }
}

fn build_active_buffer(frame: &LayoutFrame, editor: &Editor) -> View {
    let Some(cursor_id) = frame.tabs.get(frame.active_tab).copied() else {
        return View::Empty;
    };
    let Some(cursor) = editor.cursors.get(cursor_id) else {
        return View::Empty;
    };
    let doc_path = doc_path_for(cursor.doc).unwrap_or_else(|| {
        // Fallback: encode raw key; should never happen in
        // well-formed editor state.
        Path::parse("/buf/0").expect("/buf/0 is a valid path")
    });
    let head = cursor.selection.primary();
    let Some(doc) = editor.documents.get(cursor.doc) else {
        return View::Empty;
    };
    let cursor_mark = char_to_line_col(doc, head.head);
    View::Buffer {
        id: ViewNodeId(doc_path.clone()),
        path: doc_path,
        scroll_top_line: cursor.scroll.0,
        cursor: Some(cursor_mark),
        selection: Vec::new(),
        highlights: Vec::new(),
        gutter: GutterMode::LineNumbers,
        active: true,
        transition: None,
    }
}

fn walk_sidebar(
    sidebar: &LayoutSidebar,
    path_indices: &[usize],
    _editor: &Editor,
    pane_path: &Path,
) -> View {
    // Sidebar's content is a `Pane` (not a LayoutNode), so we can't
    // recurse via `walk_layout`. The full content rendering is the
    // tui interpreter's concern (T-44); the View IR carries the
    // sidebar shell + an Empty placeholder until the Pane→View
    // collapse in Stage 9.
    let _ = path_indices;
    let slot = map_sidebar_slot(sidebar.slot);
    let slot_segment = match sidebar.slot {
        CoreSidebarSlot::Left => "left",
        CoreSidebarSlot::Right => "right",
    };
    let sidebar_path = pane_path
        .join("sidebar")
        .and_then(|p| p.join(slot_segment))
        .expect("sidebar path segments are valid");
    View::Sidebar {
        id: ViewNodeId(sidebar_path),
        slot,
        title: String::new(),
        focused: false,
        content: Box::new(View::Empty),
        transition: None,
    }
}

// -- Helpers ----------------------------------------------------------------

fn pane_path(indices: &[usize]) -> Path {
    let mut p = Path::parse("/pane").expect("/pane is canonical");
    for i in indices {
        p = p.join(&i.to_string()).expect("integer segment is valid");
    }
    p
}

fn synthetic_id(kind: &str, parent_pane_path: &Path, suffix: Option<&str>) -> ViewNodeId {
    // Encode the parent path's slashes as `_` so the whole derived
    // id sits inside one segment under `/synthetic/<kind>/...`. This
    // is a placeholder per T-43; T-90 picks the formal strategy.
    let parent = parent_pane_path.as_str().trim_start_matches('/').replace('/', "_");
    let mut encoded = if parent.is_empty() {
        "root".to_string()
    } else {
        parent
    };
    if let Some(s) = suffix {
        encoded.push('_');
        encoded.push_str(s);
    }
    let path = build_synthetic_path(kind, &encoded).expect("synthetic id parts are valid");
    ViewNodeId(path)
}

fn build_synthetic_path(kind: &str, encoded: &str) -> Result<Path, PathError> {
    Path::parse("/synthetic")?.join(kind)?.join(encoded)
}

fn map_axis(a: CoreAxis) -> ViewAxis {
    match a {
        CoreAxis::Horizontal => ViewAxis::Horizontal,
        CoreAxis::Vertical => ViewAxis::Vertical,
    }
}

fn map_sidebar_slot(s: CoreSidebarSlot) -> ViewSidebarSlot {
    match s {
        CoreSidebarSlot::Left => ViewSidebarSlot::Left,
        CoreSidebarSlot::Right => ViewSidebarSlot::Right,
    }
}

/// Encode a `DocId` as its canonical `/buf/<id>` path. T-50 swapped
/// in a process-monotonic counter, closing the slotmap-key shim
/// the T-43 producer used at first. `DocId::to_path` is the
/// round-trip pair to `DocId::id_from_path`.
fn doc_path_for(doc: crate::editor::document::DocId) -> Option<Path> {
    Some(doc.to_path())
}

fn tab_label_and_doc(cursor_id: CursorId, editor: &Editor) -> (String, Path) {
    let label = editor
        .cursors
        .get(cursor_id)
        .and_then(|c| editor.documents.get(c.doc))
        .map(doc_label)
        .unwrap_or_else(|| "untitled".to_string());
    let doc_path = editor
        .cursors
        .get(cursor_id)
        .and_then(|c| doc_path_for(c.doc))
        .unwrap_or_else(|| Path::parse("/buf/0").unwrap());
    (label, doc_path)
}

fn doc_label(doc: &Document) -> String {
    doc.buffer
        .path()
        .and_then(|p| p.file_name())
        .and_then(|name| name.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "untitled".to_string())
}

fn doc_dirty(cursor_id: CursorId, editor: &Editor) -> bool {
    editor
        .cursors
        .get(cursor_id)
        .and_then(|c| editor.documents.get(c.doc))
        .map(|d| d.buffer.dirty())
        .unwrap_or(false)
}

/// Translate a char index in the document to a (line, column) pair
/// for the view IR's `CursorMark`.
fn char_to_line_col(doc: &Document, char_idx: usize) -> CursorMark {
    let buf = &doc.buffer;
    let line = buf.line_of_char(char_idx);
    let line_start = buf.line_start(line);
    let col = char_idx.saturating_sub(line_start);
    CursorMark {
        line: line as u32,
        col: col as u32,
    }
}

#[cfg(test)]
mod tests {
    use devix_protocol::path::Path;

    use super::*;
    use crate::editor::editor::Editor;

    #[test]
    fn view_at_unknown_root_returns_unknown_path_error() {
        let editor = Editor::open(None).unwrap();
        let bad = Path::parse("/buf/42").unwrap();
        let err = editor.view(&bad).unwrap_err();
        assert!(matches!(err, RequestError::UnknownPath(_)));
    }

    #[test]
    fn view_at_pane_root_returns_a_view_tree() {
        let editor = Editor::open(None).unwrap();
        let root = Path::parse("/pane").unwrap();
        let v = editor.view(&root).unwrap();
        // A fresh editor has at least one frame; the producer
        // should emit a Stack (frame) wrapping a Buffer + TabStrip.
        match v {
            View::Stack { .. } | View::Split { .. } | View::Buffer { .. } => {}
            other => panic!("unexpected view variant: {:?}", other),
        }
    }

    #[test]
    fn synthetic_id_is_deterministic_across_calls() {
        let p = Path::parse("/pane/0").unwrap();
        let a = synthetic_id("stack", &p, None);
        let b = synthetic_id("stack", &p, None);
        assert_eq!(a, b);
        assert!(a.0.as_str().starts_with("/synthetic/stack/"));
    }
}
