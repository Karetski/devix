//! Structural layout panes — the editor's structural skeleton.
//!
//! Splits, frames, and sidebars are each their own `Pane` impl. The
//! editor's `Editor.panes.root: Box<dyn Pane>` is the only source of
//! layout truth. There is no `LayoutNode` enum: the closed wrapper
//! retired in T-91 phase-2 close, when `LayoutSplit.children` shifted
//! to `Vec<(Box<dyn Pane>, u16)>` and the structural tree lifted onto
//! one open primitive (Lattner's MLIR principle).
//!
//! Render takes editor borrows as a real `&LayoutCtx<'_>` argument
//! threaded through `RenderCtx::layout` (chrome / modal / plugin
//! panes pass `None` and ignore it).
//!
//! The structural cousins (frame, sidebar, split) implement `Pane`
//! directly. Plugin-contributed panes drop into `LayoutSidebar.content`
//! exactly the same way as before — the content boundary that
//! survived the structural collapse.

use ratatui::Frame;

use crate::{
    Axis, Event, Outcome, Pane, Rect, RenderCtx, SidebarPane as SidebarChrome,
    SidebarSlot, TabInfo, TabStripPane, TabbedPane, Theme, split_rects,
};

use crate::editor::buffer::EditorPane;
use crate::editor::cursor::{Cursor, CursorId};
use crate::editor::document::Document;
use crate::editor::{LeafRef, RenderCache};
use crate::editor::frame::FrameId;

/// Recursive split. Children laid out along `axis` by integer weights;
/// rect math comes from `split_rects` in `crate::layout_geom`.
pub struct LayoutSplit {
    pub axis: Axis,
    pub children: Vec<(Box<dyn Pane>, u16)>,
}

impl LayoutSplit {
    pub fn new(axis: Axis, children: Vec<(Box<dyn Pane>, u16)>) -> Self {
        Self { axis, children }
    }
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
/// the structural Pane impls via `RenderCtx::layout`.
pub struct LayoutCtx<'a> {
    pub documents: &'a crate::editor::document::DocStore,
    pub cursors: &'a crate::editor::cursor::CursorStore,
    pub theme: &'a Theme,
    pub render_cache: &'a RenderCache,
    pub focused_leaf: Option<LeafRef>,
}

/// Build a fresh frame leaf as a boxed Pane. Convenience for callers
/// that compose layout subtrees (ops, registry, tests).
pub fn frame_pane(fid: FrameId, cursor: CursorId) -> Box<dyn Pane> {
    Box::new(LayoutFrame::with_cursor(fid, cursor))
}

/// Build a fresh empty-content sidebar leaf as a boxed Pane.
pub fn sidebar_pane(slot: SidebarSlot) -> Box<dyn Pane> {
    Box::new(LayoutSidebar::empty(slot))
}

/// Build a horizontal split with the given child weights as a boxed
/// Pane. Children are themselves boxed Panes — the structural tree
/// nests through `Box<dyn Pane>` everywhere.
pub fn split_pane(axis: Axis, children: Vec<(Box<dyn Pane>, u16)>) -> Box<dyn Pane> {
    Box::new(LayoutSplit::new(axis, children))
}

// ---- Per-variant Pane impls ---------------------------------------

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
            child.render(rect, ctx);
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
            .map(|((child, _), rect)| (rect, &**child as &dyn Pane))
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
            .map(|((child, _), rect)| (rect, &mut **child as &mut dyn Pane))
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

/// Tree-mutation helpers — operate directly on `Box<dyn Pane>` and
/// downcast to `LayoutSplit` to access the structural children list.
/// The `path` in each helper is a list of `LayoutSplit.children`
/// indices; non-split mid-walk nodes return `false`.
pub mod mutate {
    use super::{Axis, LayoutSplit};
    use crate::Pane;

    fn downcast_split(node: &mut dyn Pane) -> Option<&mut LayoutSplit> {
        node.as_any_mut()?.downcast_mut::<LayoutSplit>()
    }

    /// Replace the node at `path` with `new`. Empty path replaces the
    /// root. Returns `false` if the path goes out of range or hits a
    /// non-Split mid-walk.
    pub fn replace_at(
        root: &mut Box<dyn Pane>,
        path: &[usize],
        new: Box<dyn Pane>,
    ) -> bool {
        if path.is_empty() {
            *root = new;
            return true;
        }
        let mut cur: &mut Box<dyn Pane> = root;
        for (i, &idx) in path.iter().enumerate() {
            let last = i + 1 == path.len();
            let split = match downcast_split(cur.as_mut()) {
                Some(s) => s,
                None => return false,
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
    pub fn remove_at(root: &mut Box<dyn Pane>, path: &[usize]) -> bool {
        if path.is_empty() {
            return false;
        }
        let (parent_path, last) = path.split_at(path.len() - 1);
        let leaf_idx = last[0];
        let mut cur: &mut Box<dyn Pane> = root;
        for &idx in parent_path {
            let split = match downcast_split(cur.as_mut()) {
                Some(s) => s,
                None => return false,
            };
            if idx >= split.children.len() {
                return false;
            }
            cur = &mut split.children[idx].0;
        }
        let split = match downcast_split(cur.as_mut()) {
            Some(s) => s,
            None => return false,
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
    pub fn collapse_singletons(root: &mut Box<dyn Pane>) {
        if let Some(split) = downcast_split(root.as_mut()) {
            for (child, _) in split.children.iter_mut() {
                collapse_singletons(child);
            }
            if split.children.len() == 1 {
                let (only, _) = split.children.remove(0);
                *root = only;
            }
        }
    }

    /// Replace the root with a horizontal Split holding the previous
    /// root as its sole child (weight 80). Used by `toggle_sidebar`
    /// when the first sidebar opens against a non-split root.
    pub fn lift_into_horizontal_split(root: &mut Box<dyn Pane>) {
        let placeholder: Box<dyn Pane> = Box::new(LayoutSplit {
            axis: Axis::Horizontal,
            children: Vec::new(),
        });
        let inner = std::mem::replace(root, placeholder);
        let split = downcast_split(root.as_mut())
            .expect("just installed a Split");
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

    fn empty_frame_pane(fid: FrameId) -> Box<dyn Pane> {
        Box::new(LayoutFrame {
            frame: fid,
            tabs: Vec::new(),
            active_tab: 0,
            tab_strip_scroll: (0, 0),
            recenter_active: false,
        })
    }

    #[test]
    fn split_pane_trait_returns_children_with_weighted_rects() {
        let f1 = fake_frame_id();
        let f2 = fake_frame_id();
        let split = LayoutSplit {
            axis: Axis::Horizontal,
            children: vec![(empty_frame_pane(f1), 1), (empty_frame_pane(f2), 3)],
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
        assert!(Pane::children(&s, full()).is_empty());
    }

    #[test]
    fn replace_at_root_swaps_root() {
        let f1 = fake_frame_id();
        let f2 = fake_frame_id();
        let mut tree: Box<dyn Pane> = empty_frame_pane(f1);
        assert!(mutate::replace_at(&mut tree, &[], empty_frame_pane(f2)));
        let frame = tree
            .as_any()
            .and_then(|a| a.downcast_ref::<LayoutFrame>())
            .unwrap();
        assert_eq!(frame.frame, f2);
    }

    #[test]
    fn remove_at_drops_one_child_and_collapse_flattens() {
        let f1 = fake_frame_id();
        let f2 = fake_frame_id();
        let mut tree: Box<dyn Pane> = split_pane(
            Axis::Horizontal,
            vec![(empty_frame_pane(f1), 1), (empty_frame_pane(f2), 1)],
        );
        assert!(mutate::remove_at(&mut tree, &[1]));
        mutate::collapse_singletons(&mut tree);
        let frame = tree
            .as_any()
            .and_then(|a| a.downcast_ref::<LayoutFrame>())
            .unwrap();
        assert_eq!(frame.frame, f1);
    }

    #[test]
    fn lift_into_horizontal_split_wraps_root_with_weight_eighty() {
        let f1 = fake_frame_id();
        let mut tree: Box<dyn Pane> = empty_frame_pane(f1);
        mutate::lift_into_horizontal_split(&mut tree);
        let split = tree
            .as_any()
            .and_then(|a| a.downcast_ref::<LayoutSplit>())
            .expect("root is now a Split");
        assert_eq!(split.axis, Axis::Horizontal);
        assert_eq!(split.children.len(), 1);
        assert_eq!(split.children[0].1, 80);
    }
}
