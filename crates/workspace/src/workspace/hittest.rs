//! Hit-testing: pixel coordinates → leaf or tab-strip element. Mouse-driven
//! tab activation and tab-strip scroll forwarding live here too because their
//! input source is the same hit-test cache.

use ratatui::layout::Rect;

use crate::frame::FrameId;

use super::{TabStripHit, Workspace};
use super::focus::path_to_leaf;

impl Workspace {
    /// Find the tab-strip element under (col, row), if any. Used by the input
    /// layer before falling back to body-area hit-testing.
    pub fn tab_strip_hit(&self, col: u16, row: u16) -> Option<TabStripHit> {
        for (fid, strip) in &self.render_cache.tab_strips {
            for hit in &strip.hits {
                if rect_contains(hit.rect, col, row) {
                    return Some(TabStripHit::Tab { frame: fid, idx: hit.idx });
                }
            }
        }
        None
    }

    /// Frame whose tab-strip row contains (col, row). Independent of where in
    /// the strip the click landed — empty space past the last tab still
    /// resolves the frame so the wheel scrolls it.
    pub fn frame_at_strip(&self, col: u16, row: u16) -> Option<FrameId> {
        for (fid, strip) in &self.render_cache.tab_strips {
            if rect_contains(strip.strip_rect, col, row) {
                return Some(fid);
            }
        }
        None
    }

    /// Whether the tab strip currently overflows its row — i.e., scrolling
    /// can produce a visible change. Used by the input layer to decide
    /// whether to consume a wheel event or pass it through to the editor.
    pub fn tab_strip_can_scroll(&self, frame: FrameId) -> bool {
        let Some(strip) = self.render_cache.tab_strips.get(frame) else { return false };
        strip.content_width > strip.strip_rect.width as u32
    }

    /// Apply a horizontal scroll delta (cells) to a frame's tab strip. Routes
    /// through the frame's `CollectionState` so all scroll math lives in one
    /// place. No-op when content fits in the strip.
    pub fn scroll_tab_strip(&mut self, frame: FrameId, delta: isize) {
        let Some(strip) = self.render_cache.tab_strips.get(frame) else { return };
        let content = (strip.content_width, 1);
        let viewport = (strip.strip_rect.width as u32, 1);
        let Some(f) = self.frames.get_mut(frame) else { return };
        f.tab_strip_state.scroll_by(delta, 0, content, viewport);
    }

    /// Activate `idx` on `frame` from a click on a visible tab. Does *not*
    /// scroll the strip — the user already picked a tab they could see.
    /// Out-of-range indices clamp to a valid value.
    pub fn activate_tab(&mut self, frame: FrameId, idx: usize) {
        let Some(f) = self.frames.get_mut(frame) else { return };
        if f.tabs.is_empty() { return; }
        f.select_visible(idx.min(f.tabs.len() - 1));
    }

    /// Set focus to the leaf whose Rect contains (col, row), if any.
    pub fn focus_at_screen(&mut self, col: u16, row: u16) {
        let leaves = self.layout.leaves_with_rects(self.outer_editor_area());
        for (leaf, rect) in leaves {
            if (col >= rect.x && col < rect.x + rect.width)
                && (row >= rect.y && row < rect.y + rect.height)
            {
                if let Some(path) = path_to_leaf(&self.layout, leaf) {
                    self.focus = path;
                    return;
                }
            }
        }
    }

    /// The total area the layout tree occupies, derived from cached rects.
    /// Used by hit-testing without re-running a layout pass. Includes tab-strip
    /// rows so clicks on the strip can resolve to the owning frame.
    fn outer_editor_area(&self) -> Rect {
        let rects: Vec<Rect> = self.render_cache.frame_rects.values().copied()
            .chain(self.render_cache.sidebar_rects.values().copied())
            .chain(
                self.render_cache.tab_strips.values()
                    .map(|s| s.strip_rect)
            )
            .collect();
        if rects.is_empty() { return Rect::default(); }
        let x = rects.iter().map(|r| r.x).min().unwrap();
        let y = rects.iter().map(|r| r.y).min().unwrap();
        let x_end = rects.iter().map(|r| r.x + r.width).max().unwrap();
        let y_end = rects.iter().map(|r| r.y + r.height).max().unwrap();
        Rect { x, y, width: x_end - x, height: y_end - y }
    }
}

fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x
        && col < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}
