//! View IR producer — `Editor::view(root: Path) -> Result<View, RequestError>`.
//!
//! Walks the structural Pane tree and emits the closed `View` IR per
//! `docs/specs/frontend.md`. Every resource-bound node carries its
//! canonical `Path` (`/buf/<id>`, `/pane/<i>(/<j>)*`,
//! `/pane/.../tabstrip`, `/pane/.../sidebar/<slot>`); synthetic
//! wrapper nodes use the **deterministic-derivation** form locked
//! during T-90 (see foundations-review log 2026-05-07):
//! `/synthetic/<kind>/<encoded-parent-path>[/<suffix>]`. Same logical
//! node across two renders ⇒ same id, by construction — no state, no
//! per-parent cache. The alternative mint-and-cache shape was
//! considered and rejected at T-90: same answer for "child at
//! structural position i" without the cache state.
//!
//! Scope:
//! * `transition` is `None` everywhere — `Capability::Animations`
//!   gating lands when an animating frontend ships.
//! * Highlights are not populated by this producer at T-43 — the
//!   tree-sitter actor (Stage 8) will publish them; the View carries
//!   an empty `highlights` list for now.
//! * Walks the unified Pane tree (post-T-91): downcasts at each step
//!   to the concrete structural pane (`LayoutFrame`, `LayoutSidebar`,
//!   `LayoutSplit`) for typed access.

use devix_protocol::path::{Path, PathError};
use devix_protocol::protocol::RequestError;
use devix_protocol::view::{
    Axis as ViewAxis, BufferLine, CursorMark, GutterMode, SidebarSlot as ViewSidebarSlot, Style,
    TabItem, TextSpan, View, ViewNodeId,
};

use crate::Pane;
use crate::editor::cursor::CursorId;
use crate::editor::document::Document;
use crate::editor::editor::Editor;
use crate::editor::tree::{LayoutFrame, LayoutSidebar, LayoutSplit};
use crate::layout_geom::{Axis as CoreAxis, SidebarSlot as CoreSidebarSlot};
use crate::theme::Theme;

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

/// Walk a structural Pane, emitting `View`. `path_indices` is the
/// 0-based child position list from the layout root (e.g. `[0, 1]`
/// for `/pane/0/1`).
fn walk_layout(node: &dyn Pane, path_indices: &[usize], editor: &Editor) -> View {
    let pane_path = pane_path(path_indices);
    let any = match node.as_any() {
        Some(any) => any,
        None => return View::Empty,
    };
    if let Some(split) = any.downcast_ref::<LayoutSplit>() {
        return walk_split(split, path_indices, editor, &pane_path);
    }
    if let Some(frame) = any.downcast_ref::<LayoutFrame>() {
        return walk_frame(frame, &pane_path, editor);
    }
    if let Some(sidebar) = any.downcast_ref::<LayoutSidebar>() {
        return walk_sidebar(sidebar, path_indices, editor, &pane_path);
    }
    View::Empty
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
        children.push(walk_layout(child_node.as_ref(), &child_indices, editor));
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
    let (lines, gutter_width) = materialize_visible_lines(
        editor,
        cursor.doc,
        cursor.scroll.0 as usize,
        &editor.theme,
    );
    View::Buffer {
        id: ViewNodeId(doc_path.clone()),
        path: doc_path,
        scroll_top_line: cursor.scroll.0,
        cursor: Some(cursor_mark),
        selection: Vec::new(),
        highlights: Vec::new(),
        gutter: GutterMode::LineNumbers,
        lines,
        gutter_width,
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
    // The sidebar's `content` is a generic `Pane` whose mapping into
    // the View IR is the tui interpreter's concern (T-44); the View
    // here carries the sidebar shell + an Empty placeholder until
    // the Pane→View collapse work post-Stage-9 reaches it.
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

/// Materialize the visible window of `doc` into a `Vec<BufferLine>`
/// with pre-formatted gutter and theme-resolved style runs. T-95
/// producer-materialization. Returns the lines plus the gutter
/// width in cells.
///
/// `scroll_top` is the 0-based first visible line. The producer
/// materializes a fixed window of `MATERIALIZE_WINDOW` lines —
/// frontends with a deeper viewport can request again with a larger
/// `scroll_top` until the spec gains an explicit
/// `request_visible_rows` field.
fn materialize_visible_lines(
    editor: &Editor,
    doc_id: crate::editor::document::DocId,
    scroll_top: usize,
    theme: &Theme,
) -> (Vec<BufferLine>, u32) {
    /// Default rendered window. ratatui frontends typically have
    /// 30-100 visible rows; over-materializing by a few lines is
    /// cheap. Future versions accept a viewport hint via the
    /// request payload.
    const MATERIALIZE_WINDOW: usize = 200;

    let Some(doc) = editor.documents.get(doc_id) else {
        return (Vec::new(), 0);
    };
    let line_count = doc.buffer.line_count();
    if line_count == 0 {
        return (Vec::new(), 0);
    }
    let gutter_digits = (line_count.max(1)).to_string().len();
    let gutter_width = (gutter_digits + 2) as u32; // " 42 " — leading + trailing space
    let scope_default = theme.text_style();

    let top = scroll_top.min(line_count);
    let bottom = (top + MATERIALIZE_WINDOW).min(line_count);

    // Highlights for the entire visible byte range. Reads from the
    // editor's `highlight_cache` first (populated by the supervised
    // `HighlightActor` per Pulse::HighlightsReady); falls back to
    // the document's synchronous highlighter when the cache is cold
    // or unset. T-80 wire-up.
    let rope = doc.buffer.rope();
    let start_byte = rope.line_to_byte(top);
    let end_byte = if bottom >= line_count {
        rope.len_bytes()
    } else {
        rope.line_to_byte(bottom)
    };
    let highlights = editor.highlights_for(doc_id, start_byte, end_byte);

    let mut out = Vec::with_capacity(bottom.saturating_sub(top));
    for line_idx in top..bottom {
        let gutter = format!(
            " {:>width$} ",
            line_idx + 1,
            width = gutter_digits,
        );
        let line_text = doc.buffer.line_string_truncated(line_idx, MATERIALIZE_WINDOW * 4);
        let spans = build_line_spans(
            &line_text,
            line_idx,
            rope,
            &highlights,
            theme,
            scope_default,
        );
        out.push(BufferLine {
            line: line_idx as u32,
            gutter,
            spans,
        });
    }
    (out, gutter_width)
}

/// Build per-line `TextSpan`s by walking highlights against the
/// theme. Mirrors the legacy `editor::buffer::styled_line_spans`
/// path; emits spans in source order with style runs that paint
/// last-write wins (more-specific scopes override their parents).
fn build_line_spans(
    line_text: &str,
    line_idx: usize,
    rope: &ropey::Rope,
    highlights: &[devix_syntax::HighlightSpan],
    theme: &Theme,
    default_style: ratatui::style::Style,
) -> Vec<TextSpan> {
    let chars: Vec<char> = line_text.chars().collect();
    let n = chars.len();
    if n == 0 {
        return Vec::new();
    }
    let line_char_start = rope.line_to_char(line_idx);
    let line_byte_start = rope.char_to_byte(line_char_start);
    let line_byte_end = rope.char_to_byte(line_char_start + n);

    // Per-char effective style (rendered as ratatui::Style so we can
    // reuse the theme's resolved `style_for` shape).
    let mut styles: Vec<ratatui::style::Style> = vec![default_style; n];
    for span in highlights {
        if span.end_byte <= line_byte_start || span.start_byte >= line_byte_end {
            continue;
        }
        let Some(scope_style) = theme.style_for(&span.scope) else { continue };
        let s_byte = span.start_byte.max(line_byte_start);
        let e_byte = span.end_byte.min(line_byte_end);
        let s_char = rope.byte_to_char(s_byte).saturating_sub(line_char_start);
        let e_char = rope.byte_to_char(e_byte).saturating_sub(line_char_start);
        let s = s_char.min(n);
        let e = e_char.min(n);
        for slot in &mut styles[s..e] {
            *slot = scope_style;
        }
    }

    // Coalesce adjacent same-style cells into one span. The wire
    // form (`Style`) matches `devix_protocol::view::Style`.
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let style = styles[i];
        let mut j = i + 1;
        while j < n && styles[j] == style {
            j += 1;
        }
        let chunk: String = chars[i..j].iter().collect();
        out.push(TextSpan {
            text: chunk,
            style: ratatui_to_view_style(style),
        });
        i = j;
    }
    out
}

fn ratatui_to_view_style(s: ratatui::style::Style) -> Style {
    use devix_protocol::view::{Color as VColor, NamedColor as VNamed};
    use ratatui::style::{Color as RColor, Modifier};
    fn color_to_view(c: RColor) -> VColor {
        match c {
            RColor::Reset => VColor::Default,
            RColor::Rgb(r, g, b) => VColor::Rgb(r, g, b),
            RColor::Indexed(n) => VColor::Indexed(n),
            RColor::Black => VColor::Named(VNamed::Black),
            RColor::Red => VColor::Named(VNamed::Red),
            RColor::Green => VColor::Named(VNamed::Green),
            RColor::Yellow => VColor::Named(VNamed::Yellow),
            RColor::Blue => VColor::Named(VNamed::Blue),
            RColor::Magenta => VColor::Named(VNamed::Magenta),
            RColor::Cyan => VColor::Named(VNamed::Cyan),
            RColor::White => VColor::Named(VNamed::White),
            RColor::DarkGray => VColor::Named(VNamed::DarkGray),
            RColor::LightRed => VColor::Named(VNamed::LightRed),
            RColor::LightGreen => VColor::Named(VNamed::LightGreen),
            RColor::LightYellow => VColor::Named(VNamed::LightYellow),
            RColor::LightBlue => VColor::Named(VNamed::LightBlue),
            RColor::LightMagenta => VColor::Named(VNamed::LightMagenta),
            RColor::LightCyan => VColor::Named(VNamed::LightCyan),
            // ratatui's truecolor extras don't all fit; fall back to
            // default for the residual.
            _ => VColor::Default,
        }
    }
    Style {
        fg: s.fg.map(color_to_view),
        bg: s.bg.map(color_to_view),
        bold: s.add_modifier.contains(Modifier::BOLD),
        italic: s.add_modifier.contains(Modifier::ITALIC),
        underline: s.add_modifier.contains(Modifier::UNDERLINED),
        dim: s.add_modifier.contains(Modifier::DIM),
        reverse: s.add_modifier.contains(Modifier::REVERSED),
    }
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

    #[test]
    fn buffer_view_materializes_visible_lines() {
        use devix_text::{Selection, replace_selection_tx};

        let mut editor = Editor::open(None).unwrap();
        let did = editor.active_cursor().unwrap().doc;
        let tx = replace_selection_tx(
            &editor.documents[did].buffer,
            &Selection::point(0),
            "alpha\nbeta\ngamma\n",
        );
        editor.documents[did].buffer.apply(tx);

        let root = Path::parse("/pane").unwrap();
        let v = editor.view(&root).unwrap();
        // Walk down to the Buffer node — the editor's fresh layout
        // is Stack(TabStrip + Buffer). We accept any nesting depth
        // so the test stays robust if the structural shape changes.
        let buffer = find_buffer(&v).expect("view tree has a Buffer node");
        match buffer {
            View::Buffer { lines, gutter_width, .. } => {
                assert!(*gutter_width > 0, "gutter width populated");
                assert!(lines.len() >= 3, "at least three visible lines");
                let row0_text: String =
                    lines[0].spans.iter().map(|s| s.text.as_str()).collect();
                assert_eq!(row0_text, "alpha");
                assert!(
                    lines[0].gutter.contains('1'),
                    "line 1 gutter contains `1`",
                );
            }
            _ => unreachable!(),
        }
    }

    fn find_buffer(view: &View) -> Option<&View> {
        match view {
            View::Buffer { .. } => Some(view),
            View::Stack { children, .. } | View::Split { children, .. } => {
                children.iter().find_map(find_buffer)
            }
            _ => None,
        }
    }
}
