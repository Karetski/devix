//! Hit-testing: pixel coordinates → leaf or tab-strip element. Mouse-driven
//! tab activation and tab-strip scroll forwarding live here too because their
//! input source is the same hit-test cache.

use crate::Rect;

use crate::editor::frame::FrameId;

use super::{Editor, RenderCache, TabStripHit};

impl Editor {
    /// Find the tab-strip element under (col, row), if any. Used by the
    /// input layer (with a tui-side `RenderCache` reference) before
    /// falling back to body-area hit-testing.
    pub fn tab_strip_hit(&self, col: u16, row: u16, cache: &RenderCache) -> Option<TabStripHit> {
        for (fid, strip) in &cache.tab_strips {
            for hit in &strip.hits {
                if rect_contains(hit.rect, col, row) {
                    return Some(TabStripHit::Tab { frame: *fid, idx: hit.idx });
                }
            }
        }
        None
    }

    /// Frame whose tab-strip row contains (col, row). Independent of
    /// where in the strip the click landed — empty space past the last
    /// tab still resolves the frame so the wheel scrolls it.
    pub fn frame_at_strip(&self, col: u16, row: u16, cache: &RenderCache) -> Option<FrameId> {
        for (fid, strip) in &cache.tab_strips {
            if rect_contains(strip.strip_rect, col, row) {
                return Some(*fid);
            }
        }
        None
    }

    /// Whether the tab strip currently overflows its row — i.e.,
    /// scrolling can produce a visible change.
    pub fn tab_strip_can_scroll(&self, frame: FrameId, cache: &RenderCache) -> bool {
        let Some(strip) = cache.tab_strips.get(&frame) else { return false };
        strip.content_width > strip.strip_rect.width as u32
    }

    /// Apply a horizontal scroll delta (cells) to a frame's tab strip.
    pub fn scroll_tab_strip(&mut self, frame: FrameId, delta: isize, cache: &RenderCache) {
        let max_x = match cache.tab_strips.get(&frame) {
            Some(strip) => strip.content_width.saturating_sub(strip.strip_rect.width as u32) as i64,
            None => return,
        };
        let Some(f) = self.panes.find_frame_mut(frame) else { return };
        let nx = (f.tab_strip_scroll.0 as i64 + delta as i64).clamp(0, max_x);
        f.tab_strip_scroll.0 = nx as u32;
    }

    /// Activate `idx` on `frame` from a click on a visible tab. Does *not*
    /// scroll the strip — the user already picked a tab they could see.
    pub fn activate_tab(&mut self, frame: FrameId, idx: usize) {
        let Some(f) = self.panes.find_frame_mut(frame) else { return };
        if f.tabs.is_empty() { return; }
        f.select_visible(idx.min(f.tabs.len() - 1));
    }

    /// Set focus to the leaf whose Rect contains (col, row), if any.
    pub fn focus_at_screen(&mut self, col: u16, row: u16, cache: &RenderCache) {
        let area = outer_editor_area(cache);
        let Some((_, node)) = self.panes.pane_at_xy(area, col, row) else { return };
        let Some(leaf) = crate::editor::registry::pane_leaf_id(node) else { return };
        if let Some(path) = self.panes.path_to_leaf(leaf) {
            self.set_focus(path);
        }
    }
}

/// The total area the layout tree occupies, derived from cached rects.
fn outer_editor_area(cache: &RenderCache) -> Rect {
    let rects: Vec<Rect> = cache.frame_rects.values().copied()
        .chain(cache.sidebar_rects.values().copied())
        .chain(cache.tab_strips.values().map(|s| s.strip_rect))
        .collect();
    if rects.is_empty() { return Rect::default(); }
    let x = rects.iter().map(|r| r.x).min().unwrap();
    let y = rects.iter().map(|r| r.y).min().unwrap();
    let x_end = rects.iter().map(|r| r.x + r.width).max().unwrap();
    let y_end = rects.iter().map(|r| r.y + r.height).max().unwrap();
    Rect { x, y, width: x_end - x, height: y_end - y }
}

fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x
        && col < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

