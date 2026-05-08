//! Structural layout tree — the editor's private layout vocabulary.
//!
//! Splits, frames, sidebars are a *closed* set of node kinds: third
//! parties don't contribute new layout shapes — they extend through
//! content (a modal pane, a sidebar pane, an editor command). So the
//! structural tree is a closed enum, walked by exhaustive match. There
//! is no `Box<dyn Pane>` for the structural skeleton, no `as_any`
//! downcasts on the walk, no thread-local services smuggled into a
//! framework-neutral render trait.
//!
//! `panes::Pane` keeps its job at the *content boundary*: the modal
//! slot (`Editor.modal: Option<Box<dyn Pane>>`), plugin-contributed
//! sidebar content (`LayoutSidebar.content: Option<Box<dyn Pane>>`),
//! and the chrome widgets (tab strip, sidebar border, palette popup,
//! editor body) the structural nodes paint into their own rects.
//!
//! Render takes editor borrows as a real `&LayoutCtx<'_>` argument.

use ratatui::Frame;

use crate::{
    Axis, Event, HandleCtx, Outcome, Pane, Rect, RenderCtx, SidebarPane as SidebarChrome,
    SidebarSlot, TabInfo, TabStripPane, TabbedPane, Theme, split_rects,
};

use crate::editor::buffer::EditorPane;
use crate::editor::cursor::{Cursor, CursorId};
use crate::editor::document::Document;
use crate::editor::{LeafRef, RenderCache};
use crate::editor::frame::FrameId;

/// Closed enum of structural layout kinds.
pub enum LayoutNode {
    Split(LayoutSplit),
    Frame(LayoutFrame),
    Sidebar(LayoutSidebar),
}

/// Recursive split. Children laid out along `axis` by integer weights;
/// rect math comes from `split_rects` in `crate::layout_geom`.
pub struct LayoutSplit {
    pub axis: Axis,
    pub children: Vec<(LayoutNode, u16)>,
}

/// Editor-frame leaf: tabs over a single document body, plus per-frame
/// chrome scroll state. `frame: FrameId` is the stable identifier the
/// render cache (`render_cache.frame_rects`, `tab_strips`) keys against.
pub struct LayoutFrame {
    pub frame: FrameId,
    pub tabs: Vec<CursorId>,
    pub active_tab: usize,
    /// Tab-strip scroll offset in cells.
    pub tab_strip_scroll: (u32, u32),
    /// One-shot signal asking the next tab-strip render to scroll the
    /// active tab into view. Set by mutators that change `active_tab`
    /// (keyboard nav, new tab, close), cleared by the layout pass.
    pub recenter_active: bool,
}

impl LayoutFrame {
    pub fn with_cursor(frame: FrameId, cursor: CursorId) -> Self {
        Self {
            frame,
            tabs: vec![cursor],
            active_tab: 0,
            tab_strip_scroll: (0, 0),
            recenter_active: true,
        }
    }

    /// Activate a tab and request scroll-into-view. Use for keyboard
    /// nav and tab-mutating ops (new/close).
    pub fn set_active(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active_tab = idx;
        }
        self.recenter_active = true;
    }

    /// Activate without disturbing scroll. Use for click activation —
    /// the user already pointed at the tab they want.
    pub fn select_visible(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active_tab = idx;
        }
    }

    pub fn active_cursor(&self) -> Option<CursorId> {
        self.tabs.get(self.active_tab).copied()
    }
}

/// Sidebar-slot leaf. The slot enum (`Left` / `Right`) acts as the
/// identity; one of each can exist in the tree. `content` is the Pane
/// painted inside the chrome — the open extension point where plugins
/// (and future built-ins like a file tree) drop their leaf in.
pub struct LayoutSidebar {
    pub slot: SidebarSlot,
    pub content: Option<Box<dyn Pane>>,
}

impl LayoutSidebar {
    pub fn empty(slot: SidebarSlot) -> Self {
        Self { slot, content: None }
    }
}

/// Read-only borrows the structural tree needs at render time.
/// Replaces the previous TLS-smuggled `RenderServices`. The renderer
/// constructs one inside `Application::render` and threads it through
/// every recursive `LayoutNode::render` call.
pub struct LayoutCtx<'a> {
    pub documents: &'a crate::editor::document::DocStore,
    pub cursors: &'a crate::editor::cursor::CursorStore,
    pub theme: &'a Theme,
    pub render_cache: &'a RenderCache,
    pub focused_leaf: Option<LeafRef>,
}

impl LayoutNode {
    pub fn frame(frame: FrameId, cursor: CursorId) -> Self {
        LayoutNode::Frame(LayoutFrame::with_cursor(frame, cursor))
    }

    pub fn sidebar(slot: SidebarSlot) -> Self {
        LayoutNode::Sidebar(LayoutSidebar::empty(slot))
    }

    /// Identity of this node as a leaf, or `None` if it's a split.
    pub fn leaf_id(&self) -> Option<LeafRef> {
        match self {
            LayoutNode::Frame(f) => Some(LeafRef::Frame(f.frame)),
            LayoutNode::Sidebar(s) => Some(LeafRef::Sidebar(s.slot)),
            LayoutNode::Split(_) => None,
        }
    }

    /// Frames and sidebars accept focus; splits don't.
    pub fn is_focusable(&self) -> bool {
        matches!(self, LayoutNode::Frame(_) | LayoutNode::Sidebar(_))
    }

    /// Direct children of this node laid out within `area`. Splits
    /// distribute by weight; leaves return empty.
    pub fn children_at(&self, area: Rect) -> Vec<(Rect, &LayoutNode)> {
        match self {
            LayoutNode::Split(s) => {
                if s.children.is_empty() {
                    return Vec::new();
                }
                let weights: Vec<u16> = s.children.iter().map(|(_, w)| *w).collect();
                let rects = split_rects(area, s.axis, &weights);
                s.children
                    .iter()
                    .zip(rects)
                    .map(|((child, _), rect)| (rect, child))
                    .collect()
            }
            LayoutNode::Frame(_) | LayoutNode::Sidebar(_) => Vec::new(),
        }
    }

    /// Mutable counterpart of [`Self::children_at`].
    pub fn children_at_mut(&mut self, area: Rect) -> Vec<(Rect, &mut LayoutNode)> {
        match self {
            LayoutNode::Split(s) => {
                if s.children.is_empty() {
                    return Vec::new();
                }
                let weights: Vec<u16> = s.children.iter().map(|(_, w)| *w).collect();
                let rects = split_rects(area, s.axis, &weights);
                s.children
                    .iter_mut()
                    .zip(rects)
                    .map(|((child, _), rect)| (rect, child))
                    .collect()
            }
            LayoutNode::Frame(_) | LayoutNode::Sidebar(_) => Vec::new(),
        }
    }

    /// Resolve a path of `Split.children` indices to the target node.
    pub fn at_path(&self, path: &[usize]) -> Option<&LayoutNode> {
        let mut cur = self;
        for &idx in path {
            match cur {
                LayoutNode::Split(s) => {
                    let (child, _) = s.children.get(idx)?;
                    cur = child;
                }
                _ => return None,
            }
        }
        Some(cur)
    }

    /// Mutable counterpart of [`Self::at_path`].
    pub fn at_path_mut(&mut self, path: &[usize]) -> Option<&mut LayoutNode> {
        let mut cur: &mut LayoutNode = self;
        for &idx in path {
            match cur {
                LayoutNode::Split(s) => {
                    if idx >= s.children.len() {
                        return None;
                    }
                    cur = &mut s.children[idx].0;
                }
                _ => return None,
            }
        }
        Some(cur)
    }

    /// Resolve a path and the rect that node occupies inside `area`.
    pub fn at_path_with_rect(&self, area: Rect, path: &[usize]) -> Option<(Rect, &LayoutNode)> {
        let mut cur_node = self;
        let mut cur_area = area;
        for &idx in path {
            let kids = cur_node.children_at(cur_area);
            let (rect, child) = kids.into_iter().nth(idx)?;
            cur_node = child;
            cur_area = rect;
        }
        Some((cur_area, cur_node))
    }

    /// Deepest node containing `(col, row)`. Recurses through splits in
    /// reverse so later children win on overlap (z-order).
    pub fn pane_at(&self, area: Rect, col: u16, row: u16) -> Option<(Rect, &LayoutNode)> {
        if !rect_contains(area, col, row) {
            return None;
        }
        let kids = self.children_at(area);
        for (child_rect, child) in kids.iter().rev() {
            if let Some(found) = child.pane_at(*child_rect, col, row) {
                return Some(found);
            }
        }
        Some((area, self))
    }

    /// Walk the tree, collecting every leaf with the rect it occupies.
    pub fn leaves_with_rects(&self, area: Rect) -> Vec<(LeafRef, Rect)> {
        let mut out = Vec::new();
        collect_leaves(self, area, &mut out);
        out
    }

    /// Every `FrameId` in the tree, in tree order.
    pub fn frames(&self) -> Vec<FrameId> {
        let mut out = Vec::new();
        collect_frames(self, &mut out);
        out
    }

    /// Whether a sidebar leaf for `slot` is present anywhere in the tree.
    pub fn sidebar_present(&self, slot: SidebarSlot) -> bool {
        match self {
            LayoutNode::Sidebar(s) => s.slot == slot,
            LayoutNode::Frame(_) => false,
            LayoutNode::Split(s) => s.children.iter().any(|(c, _)| c.sidebar_present(slot)),
        }
    }

    /// Find a frame leaf by id.
    pub fn find_frame(&self, fid: FrameId) -> Option<&LayoutFrame> {
        match self {
            LayoutNode::Frame(f) if f.frame == fid => Some(f),
            LayoutNode::Frame(_) | LayoutNode::Sidebar(_) => None,
            LayoutNode::Split(s) => s.children.iter().find_map(|(c, _)| c.find_frame(fid)),
        }
    }

    pub fn find_frame_mut(&mut self, fid: FrameId) -> Option<&mut LayoutFrame> {
        match self {
            LayoutNode::Frame(f) if f.frame == fid => Some(f),
            LayoutNode::Frame(_) | LayoutNode::Sidebar(_) => None,
            LayoutNode::Split(s) => s
                .children
                .iter_mut()
                .find_map(|(c, _)| c.find_frame_mut(fid)),
        }
    }

    pub fn find_sidebar_mut(&mut self, slot: SidebarSlot) -> Option<&mut LayoutSidebar> {
        match self {
            LayoutNode::Sidebar(s) if s.slot == slot => Some(s),
            LayoutNode::Sidebar(_) | LayoutNode::Frame(_) => None,
            LayoutNode::Split(s) => s
                .children
                .iter_mut()
                .find_map(|(c, _)| c.find_sidebar_mut(slot)),
        }
    }

    /// Path of `Split.children` indices that leads to `target`, or
    /// `None` if no such leaf exists.
    pub fn path_to_leaf(&self, target: LeafRef) -> Option<Vec<usize>> {
        fn go(node: &LayoutNode, target: LeafRef, out: &mut Vec<usize>) -> bool {
            if node.leaf_id() == Some(target) {
                return true;
            }
            if let LayoutNode::Split(s) = node {
                for (i, (child, _)) in s.children.iter().enumerate() {
                    out.push(i);
                    if go(child, target, out) {
                        return true;
                    }
                    out.pop();
                }
            }
            false
        }
        let mut p = Vec::new();
        if go(self, target, &mut p) { Some(p) } else { None }
    }

    /// Render this node into `area`. `LayoutCtx` carries the editor
    /// borrows leaves need (documents, cursors, theme, focus). No TLS,
    /// no smuggling.
    pub fn render(&self, area: Rect, frame: &mut Frame<'_>, ctx: &LayoutCtx<'_>) {
        match self {
            LayoutNode::Split(_) => {
                for (rect, child) in self.children_at(area) {
                    child.render(rect, frame, ctx);
                }
            }
            LayoutNode::Frame(f) => render_frame(f, area, frame, ctx),
            LayoutNode::Sidebar(s) => render_sidebar(s, area, frame, ctx),
        }
    }

    /// Dispatch an input event to this node. The dispatcher resolves
    /// the focused leaf (or the leaf under the mouse) and calls this on
    /// the resulting `&mut LayoutNode`. Splits and frames don't yet
    /// consume input directly — frames respond to chord-driven commands
    /// dispatched by the keymap, not to handler walks.
    pub fn handle_at(&mut self, ev: &Event, area: Rect, hctx: &mut HandleCtx<'_>) -> Outcome {
        match self {
            LayoutNode::Split(_) | LayoutNode::Frame(_) => Outcome::Ignored,
            LayoutNode::Sidebar(s) => match s.content.as_mut() {
                Some(content) => content.handle(ev, sidebar_inner_rect(area), hctx),
                None => Outcome::Ignored,
            },
        }
    }
}

// ---- Per-variant Pane impls (T-91 phase 2 prep) -------------------
//
// Each variant struct implements `Pane` directly. `LayoutNode`'s own
// `Pane` impl delegates to the variant. This lets walks treat any
// node — variant struct or wrapping enum — as a Pane uniformly.
// Phase 2 carve will retire the enum and store one of these structs
// directly in `Box<dyn Pane>`.

impl Pane for LayoutFrame {
    fn render(&self, area: Rect, ctx: &mut crate::pane::RenderCtx<'_, '_>) {
        if let Some(layout) = ctx.layout {
            render_frame(self, area, ctx.frame, layout);
        }
    }

    fn handle(
        &mut self,
        _ev: &Event,
        _area: Rect,
        _hctx: &mut crate::pane::HandleCtx<'_>,
    ) -> Outcome {
        Outcome::Ignored
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl Pane for LayoutSidebar {
    fn render(&self, area: Rect, ctx: &mut crate::pane::RenderCtx<'_, '_>) {
        if let Some(layout) = ctx.layout {
            render_sidebar(self, area, ctx.frame, layout);
        }
    }

    fn handle(
        &mut self,
        ev: &Event,
        area: Rect,
        hctx: &mut crate::pane::HandleCtx<'_>,
    ) -> Outcome {
        match self.content.as_mut() {
            Some(content) => content.handle(ev, sidebar_inner_rect(area), hctx),
            None => Outcome::Ignored,
        }
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl Pane for LayoutSplit {
    fn render(&self, area: Rect, ctx: &mut crate::pane::RenderCtx<'_, '_>) {
        if self.children.is_empty() {
            return;
        }
        let weights: Vec<u16> = self.children.iter().map(|(_, w)| *w).collect();
        let rects = split_rects(area, self.axis, &weights);
        for ((child, _), rect) in self.children.iter().zip(rects) {
            Pane::render(child, rect, ctx);
        }
    }

    fn handle(
        &mut self,
        _ev: &Event,
        _area: Rect,
        _hctx: &mut crate::pane::HandleCtx<'_>,
    ) -> Outcome {
        Outcome::Ignored
    }

    fn children(&self, area: Rect) -> Vec<(Rect, &dyn Pane)> {
        if self.children.is_empty() {
            return Vec::new();
        }
        let weights: Vec<u16> = self.children.iter().map(|(_, w)| *w).collect();
        let rects = split_rects(area, self.axis, &weights);
        self.children
            .iter()
            .zip(rects)
            .map(|((child, _), rect)| (rect, child as &dyn Pane))
            .collect()
    }

    fn children_mut(&mut self, area: Rect) -> Vec<(Rect, &mut dyn Pane)> {
        if self.children.is_empty() {
            return Vec::new();
        }
        let weights: Vec<u16> = self.children.iter().map(|(_, w)| *w).collect();
        let rects = split_rects(area, self.axis, &weights);
        self.children
            .iter_mut()
            .zip(rects)
            .map(|((child, _), rect)| (rect, child as &mut dyn Pane))
            .collect()
    }

    fn is_focusable(&self) -> bool {
        false
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// `Pane` impl for `LayoutNode` (T-91 phase 1). Delegates to the
/// per-variant `Pane` impls above; phase 2 will retire this wrapper
/// once `PaneRegistry`'s root and `LayoutSplit.children` accept
/// arbitrary `Box<dyn Pane>` directly rather than going through the
/// enum.
impl Pane for LayoutNode {
    fn render(&self, area: Rect, ctx: &mut crate::pane::RenderCtx<'_, '_>) {
        match self {
            LayoutNode::Split(s) => Pane::render(s, area, ctx),
            LayoutNode::Frame(f) => Pane::render(f, area, ctx),
            LayoutNode::Sidebar(s) => Pane::render(s, area, ctx),
        }
    }

    fn handle(
        &mut self,
        ev: &Event,
        area: Rect,
        hctx: &mut crate::pane::HandleCtx<'_>,
    ) -> Outcome {
        match self {
            LayoutNode::Split(s) => Pane::handle(s, ev, area, hctx),
            LayoutNode::Frame(f) => Pane::handle(f, ev, area, hctx),
            LayoutNode::Sidebar(s) => Pane::handle(s, ev, area, hctx),
        }
    }

    fn children(&self, area: Rect) -> Vec<(Rect, &dyn Pane)> {
        match self {
            LayoutNode::Split(s) => Pane::children(s, area),
            LayoutNode::Frame(f) => Pane::children(f, area),
            LayoutNode::Sidebar(s) => Pane::children(s, area),
        }
    }

    fn children_mut(&mut self, area: Rect) -> Vec<(Rect, &mut dyn Pane)> {
        match self {
            LayoutNode::Split(s) => Pane::children_mut(s, area),
            LayoutNode::Frame(f) => Pane::children_mut(f, area),
            LayoutNode::Sidebar(s) => Pane::children_mut(s, area),
        }
    }

    fn is_focusable(&self) -> bool {
        // Structural Split nodes don't accept focus; Frame and Sidebar leaves do.
        matches!(self, LayoutNode::Frame(_) | LayoutNode::Sidebar(_))
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x
        && col < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

fn collect_leaves(node: &LayoutNode, area: Rect, out: &mut Vec<(LeafRef, Rect)>) {
    if let Some(id) = node.leaf_id() {
        out.push((id, area));
        return;
    }
    for (rect, child) in node.children_at(area) {
        collect_leaves(child, rect, out);
    }
}

fn collect_frames(node: &LayoutNode, out: &mut Vec<FrameId>) {
    match node {
        LayoutNode::Frame(f) => out.push(f.frame),
        LayoutNode::Sidebar(_) => {}
        LayoutNode::Split(s) => {
            for (c, _) in &s.children {
                collect_frames(c, out);
            }
        }
    }
}

fn sidebar_inner_rect(area: Rect) -> Rect {
    let x = area.x.saturating_add(1);
    let y = area.y.saturating_add(1);
    let w = area.width.saturating_sub(2);
    let h = area.height.saturating_sub(2);
    Rect { x, y, width: w, height: h }
}

fn render_frame(f: &LayoutFrame, area: Rect, frame: &mut Frame<'_>, ctx: &LayoutCtx<'_>) {
    let Some(cid) = f.active_cursor() else { return };
    let Some(cursor) = ctx.cursors.get(cid) else { return };
    let Some(doc) = ctx.documents.get(cursor.doc) else { return };

    // Editor body height — 1 row reserved for the tab strip.
    let body_height = area.height.saturating_sub(1) as usize;
    let (start, end) = visible_byte_range(doc, cursor, body_height);
    let highlights = doc.highlights(start, end);

    let active = matches!(ctx.focused_leaf, Some(LeafRef::Frame(fid)) if fid == f.frame);

    let strip_tabs: Vec<TabInfo> = f
        .tabs
        .iter()
        .filter_map(|cid| {
            let c = ctx.cursors.get(*cid)?;
            let d = ctx.documents.get(c.doc)?;
            let label = d
                .buffer
                .path()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "[scratch]".to_string());
            Some(TabInfo { label, dirty: d.buffer.dirty() })
        })
        .collect();

    let tabbed = TabbedPane {
        strip: TabStripPane {
            tabs: strip_tabs,
            active: f.active_tab,
            scroll: f.tab_strip_scroll,
        },
        body: EditorPane {
            buffer: &doc.buffer,
            selection: &cursor.selection,
            scroll: cursor.scroll,
            theme: ctx.theme,
            highlights,
            active,
        },
    };
    let mut rctx = RenderCtx { frame, layout: None };
    tabbed.render(area, &mut rctx);
}

fn render_sidebar(s: &LayoutSidebar, area: Rect, frame: &mut Frame<'_>, ctx: &LayoutCtx<'_>) {
    let title = match s.slot {
        SidebarSlot::Left => "left",
        SidebarSlot::Right => "right",
    };
    let focused = matches!(ctx.focused_leaf, Some(LeafRef::Sidebar(slot)) if slot == s.slot);
    let chrome = SidebarChrome { title: title.to_string(), focused };
    let mut rctx = RenderCtx { frame, layout: None };
    chrome.render(area, &mut rctx);
    if let Some(content) = s.content.as_ref() {
        let inner = sidebar_inner_rect(area);
        if inner.width > 0 && inner.height > 0 {
            content.render(inner, &mut rctx);
        }
    }
}

fn visible_byte_range(
    doc: &Document,
    cursor: &Cursor,
    height_rows: usize,
) -> (usize, usize) {
    let line_count = doc.buffer.line_count();
    let rope = doc.buffer.rope();
    let top = cursor.scroll_top().min(line_count);
    let bottom = (cursor.scroll_top() + height_rows).min(line_count);
    let start = rope.line_to_byte(top);
    let end = if bottom >= line_count {
        rope.len_bytes()
    } else {
        rope.line_to_byte(bottom)
    };
    (start, end)
}

/// Tree-mutation helpers — same shape as before, but typed against
/// `LayoutNode` instead of `Box<dyn Pane>`.
pub mod mutate {
    use super::{Axis, LayoutNode, LayoutSplit};

    /// Replace the node at `path` with `new`. Empty path replaces the
    /// root. Returns `false` if the path goes out of range or hits a
    /// non-Split mid-walk.
    pub fn replace_at(root: &mut LayoutNode, path: &[usize], new: LayoutNode) -> bool {
        if path.is_empty() {
            *root = new;
            return true;
        }
        let mut cur: &mut LayoutNode = root;
        for (i, &idx) in path.iter().enumerate() {
            let last = i + 1 == path.len();
            let split = match cur {
                LayoutNode::Split(s) => s,
                _ => return false,
            };
            if idx >= split.children.len() {
                return false;
            }
            if last {
                split.children[idx].0 = new;
                return true;
            }
            cur = &mut split.children[idx].0;
        }
        false
    }

    /// Remove the child at `path` from its parent split. The path must
    /// have at least one element.
    pub fn remove_at(root: &mut LayoutNode, path: &[usize]) -> bool {
        if path.is_empty() {
            return false;
        }
        let (parent_path, last) = path.split_at(path.len() - 1);
        let leaf_idx = last[0];
        let mut cur: &mut LayoutNode = root;
        for &idx in parent_path {
            let split = match cur {
                LayoutNode::Split(s) => s,
                _ => return false,
            };
            if idx >= split.children.len() {
                return false;
            }
            cur = &mut split.children[idx].0;
        }
        let split = match cur {
            LayoutNode::Split(s) => s,
            _ => return false,
        };
        if leaf_idx >= split.children.len() {
            return false;
        }
        split.children.remove(leaf_idx);
        true
    }

    /// Recursively replace any `LayoutSplit` with one child by that
    /// child. Post-order so a chain of single-child splits collapses
    /// fully in one pass.
    pub fn collapse_singletons(root: &mut LayoutNode) {
        if let LayoutNode::Split(s) = root {
            for (child, _) in s.children.iter_mut() {
                collapse_singletons(child);
            }
            if s.children.len() == 1 {
                let (only, _) = s.children.remove(0);
                *root = only;
            }
        }
    }

    /// Replace the root with a horizontal Split holding the previous
    /// root as its sole child (weight 80). Used by `toggle_sidebar` when
    /// the first sidebar opens against a non-split root.
    pub fn lift_into_horizontal_split(root: &mut LayoutNode) {
        let placeholder = LayoutNode::Split(LayoutSplit {
            axis: Axis::Horizontal,
            children: Vec::new(),
        });
        let inner = std::mem::replace(root, placeholder);
        let split = match root {
            LayoutNode::Split(s) => s,
            _ => unreachable!("just installed a Split"),
        };
        split.children.push((inner, 80));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> Rect {
        Rect { x: 0, y: 0, width: 100, height: 50 }
    }

    fn fake_frame_id() -> FrameId {
        crate::editor::frame::mint_id()
    }

    fn frame_node(fid: FrameId) -> LayoutNode {
        LayoutNode::Frame(LayoutFrame {
            frame: fid,
            tabs: Vec::new(),
            active_tab: 0,
            tab_strip_scroll: (0, 0),
            recenter_active: false,
        })
    }

    fn sidebar_node(slot: SidebarSlot) -> LayoutNode {
        LayoutNode::Sidebar(LayoutSidebar::empty(slot))
    }

    fn split_node(axis: Axis, children: Vec<(LayoutNode, u16)>) -> LayoutNode {
        LayoutNode::Split(LayoutSplit { axis, children })
    }

    #[test]
    fn split_pane_trait_returns_children_with_weighted_rects() {
        // T-91 phase 2: the LayoutSplit Pane impl exposes children
        // through `Pane::children` independently of the enum.
        let f1 = fake_frame_id();
        let f2 = fake_frame_id();
        let split = LayoutSplit {
            axis: Axis::Horizontal,
            children: vec![(frame_node(f1), 1), (frame_node(f2), 3)],
        };
        let kids = Pane::children(&split, full());
        assert_eq!(kids.len(), 2);
        // 1:3 weight on a width-100 area → 25 / 75 split.
        assert_eq!(kids[0].0.width, 25);
        assert_eq!(kids[1].0.width, 75);
    }

    #[test]
    fn frame_pane_trait_is_focusable_no_children() {
        let f = LayoutFrame {
            frame: fake_frame_id(),
            tabs: Vec::new(),
            active_tab: 0,
            tab_strip_scroll: (0, 0),
            recenter_active: false,
        };
        assert!(f.is_focusable());
        assert!(Pane::children(&f, full()).is_empty());
    }

    #[test]
    fn sidebar_pane_trait_handles_empty_slot() {
        let s = LayoutSidebar::empty(SidebarSlot::Left);
        assert!(s.is_focusable());
        // No content → Pane::children empty, leaf in the layout tree.
        assert!(Pane::children(&s, full()).is_empty());
    }

    #[test]
    fn frame_leaf_pane_at_returns_full_rect() {
        let fid = fake_frame_id();
        let tree = frame_node(fid);
        let (rect, leaf) = tree.pane_at(full(), 50, 25).unwrap();
        assert_eq!(rect, full());
        assert_eq!(leaf.leaf_id(), Some(LeafRef::Frame(fid)));
    }

    #[test]
    fn split_distributes_children_by_weight() {
        let f1 = fake_frame_id();
        let f2 = fake_frame_id();
        let tree = split_node(Axis::Horizontal, vec![(frame_node(f1), 1), (frame_node(f2), 3)]);
        let (rect, leaf) = tree.pane_at(full(), 10, 25).unwrap();
        assert_eq!(rect.x, 0);
        assert_eq!(rect.width, 25);
        assert_eq!(leaf.leaf_id(), Some(LeafRef::Frame(f1)));
        let (_, leaf) = tree.pane_at(full(), 80, 25).unwrap();
        assert_eq!(leaf.leaf_id(), Some(LeafRef::Frame(f2)));
    }

    #[test]
    fn leaves_with_rects_visits_every_leaf_in_tree_order() {
        let f1 = fake_frame_id();
        let f2 = fake_frame_id();
        let f3 = fake_frame_id();
        let inner = split_node(Axis::Vertical, vec![(frame_node(f2), 1), (frame_node(f3), 1)]);
        let outer = split_node(Axis::Horizontal, vec![(frame_node(f1), 1), (inner, 1)]);
        let leaves = outer.leaves_with_rects(full());
        let ids: Vec<FrameId> = leaves
            .iter()
            .filter_map(|(leaf, _)| match leaf {
                LeafRef::Frame(id) => Some(*id),
                _ => None,
            })
            .collect();
        assert_eq!(ids, vec![f1, f2, f3]);
    }

    #[test]
    fn sidebar_leaf_id_round_trips() {
        let tree = sidebar_node(SidebarSlot::Left);
        let (_, leaf) = tree.pane_at(full(), 10, 10).unwrap();
        assert_eq!(leaf.leaf_id(), Some(LeafRef::Sidebar(SidebarSlot::Left)));
    }

    #[test]
    fn replace_at_root_swaps_root() {
        let f1 = fake_frame_id();
        let f2 = fake_frame_id();
        let mut tree = frame_node(f1);
        assert!(mutate::replace_at(&mut tree, &[], frame_node(f2)));
        assert_eq!(tree.leaf_id(), Some(LeafRef::Frame(f2)));
    }

    #[test]
    fn remove_at_drops_one_child_and_collapse_flattens() {
        let f1 = fake_frame_id();
        let f2 = fake_frame_id();
        let mut tree = split_node(Axis::Horizontal, vec![(frame_node(f1), 1), (frame_node(f2), 1)]);
        assert!(mutate::remove_at(&mut tree, &[1]));
        mutate::collapse_singletons(&mut tree);
        assert_eq!(tree.leaf_id(), Some(LeafRef::Frame(f1)));
    }

    #[test]
    fn path_to_leaf_finds_frame_in_split() {
        let f1 = fake_frame_id();
        let f2 = fake_frame_id();
        let tree = split_node(Axis::Horizontal, vec![(frame_node(f1), 1), (frame_node(f2), 1)]);
        assert_eq!(tree.path_to_leaf(LeafRef::Frame(f1)), Some(vec![0]));
        assert_eq!(tree.path_to_leaf(LeafRef::Frame(f2)), Some(vec![1]));
    }
}
