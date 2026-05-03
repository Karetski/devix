//! Reusable collection-view primitives, modeled on UICollectionView.
//!
//! Four concerns are kept strictly separate so this scales beyond the tab
//! strip to lists, grids, sidebars, palettes, etc.:
//!
//! * **Layout** ([`CollectionLayout`]) — pure mapping from item index to
//!   virtual rect. Knows nothing about scroll, screen, or input. Examples:
//!   [`LinearLayout`] (1D flow), [`UniformLayout`] (constant-stride 1D for
//!   huge counts), future grid / waterfall / compositional.
//! * **State** ([`CollectionState`]) — scroll offset (and, later, focus /
//!   selection). Owned by whoever owns the data being scrolled (e.g. a
//!   `Frame`); persists across renders.
//! * **Projection** ([`CollectionPass`], [`project_to_screen`]) — given a
//!   layout and a state, computes per-item screen geometry plus edge clip
//!   information. The renderer paints into the projected screen rects.
//! * **Interaction** ([`CollectionPass::item_at_screen`],
//!   [`CollectionState::scroll_by`], [`CollectionState::ensure_visible`]) —
//!   hit-testing and scroll mutation.
//!
//! Virtual coordinates are `u32` so a layout can address a far larger content
//! area than the screen (e.g. a million-line buffer) — the screen `Rect` stays
//! `u16` and the projection code converts at the boundary.
//!
//! There is intentionally no "cell" type and no rendering code in this crate.
//! Cells are whatever the caller decides to draw inside [`CellGeometry::screen`].

use ratatui::layout::Rect;

// -- Hit --------------------------------------------------------------------

/// One clickable region produced by a render pass — the screen rect a given
/// item painted into. UIKit calls the equivalent value `UICollectionView`
/// layout attributes; here we keep just `idx + rect` because that is all the
/// hit-test path needs. Renderers stash `Hit`s in their output; input handlers
/// search them at click time.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub idx: usize,
    pub rect: Rect,
}

// -- Geometry ---------------------------------------------------------------

/// Rect in a layout's virtual content coordinate space (cells, top-left origin).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct VRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl VRect {
    pub fn end_x(&self) -> u32 { self.x.saturating_add(self.w) }
    pub fn end_y(&self) -> u32 { self.y.saturating_add(self.h) }

    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x && x < self.end_x() && y >= self.y && y < self.end_y()
    }

    pub fn intersects(&self, other: VRect) -> bool {
        self.x < other.end_x()
            && other.x < self.end_x()
            && self.y < other.end_y()
            && other.y < self.end_y()
    }
}

/// Where a virtual cell ends up on screen, plus how many cells of the cell
/// are hidden on each side. `clip_*` is non-zero only for partially-visible
/// cells at the edges of the viewport.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CellGeometry {
    pub virt: VRect,
    pub screen: Rect,
    pub clip_left: u16,
    pub clip_top: u16,
    pub clip_right: u16,
    pub clip_bottom: u16,
}

/// Project a virtual rect into screen space given current scroll and area.
/// Returns `None` if the rect is fully outside the viewport.
pub fn project_to_screen(virt: VRect, scroll: (u32, u32), area: Rect) -> Option<CellGeometry> {
    let view_x = scroll.0;
    let view_y = scroll.1;
    let view_x_end = view_x.saturating_add(area.width as u32);
    let view_y_end = view_y.saturating_add(area.height as u32);

    let visible_x = virt.x.max(view_x);
    let visible_y = virt.y.max(view_y);
    let visible_x_end = virt.end_x().min(view_x_end);
    let visible_y_end = virt.end_y().min(view_y_end);
    if visible_x_end <= visible_x || visible_y_end <= visible_y {
        return None;
    }
    // Differences fit in u16 because view_x_end - view_x = area.width (u16).
    Some(CellGeometry {
        virt,
        screen: Rect {
            x: area.x + (visible_x - view_x) as u16,
            y: area.y + (visible_y - view_y) as u16,
            width: (visible_x_end - visible_x) as u16,
            height: (visible_y_end - visible_y) as u16,
        },
        clip_left: (visible_x - virt.x) as u16,
        clip_top: (visible_y - virt.y) as u16,
        clip_right: (virt.end_x() - visible_x_end) as u16,
        clip_bottom: (virt.end_y() - visible_y_end) as u16,
    })
}

// -- Layout -----------------------------------------------------------------

/// Maps item indices to virtual rects. Implementations may override
/// [`Self::items_in`] / [`Self::decorations_in`] for cheap virtualization
/// when item counts are large.
pub trait CollectionLayout {
    /// Total virtual content size (cells).
    fn content_size(&self) -> (u32, u32);

    /// Total number of items.
    fn item_count(&self) -> usize;

    /// Virtual rect for a single item. Must be valid for `0..item_count()`.
    fn rect_for(&self, idx: usize) -> VRect;

    /// Items intersecting `viewport`, in z-order. Default scans linearly;
    /// non-trivial layouts (sparse grids, sectioned data) should override.
    fn items_in(&self, viewport: VRect) -> Vec<usize> {
        (0..self.item_count())
            .filter(|i| self.rect_for(*i).intersects(viewport))
            .collect()
    }

    /// Decorations (separators, gutters, headers) intersecting `viewport`,
    /// keyed by an opaque layout-defined id. Default: none.
    fn decorations_in(&self, _viewport: VRect) -> Vec<(usize, VRect)> {
        Vec::new()
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// 1D flow layout — items along an axis with optional spacing between them.
/// Spacing is reported as decorations (id = index of the item *before* the
/// gap), so renderers can paint separators inside it. O(n) `rect_for`; use
/// [`UniformLayout`] for huge constant-stride collections.
pub struct LinearLayout {
    pub axis: Axis,
    pub sizes: Vec<u32>,
    pub cross: u32,
    pub spacing: u32,
}

impl LinearLayout {
    pub fn horizontal(sizes: Vec<u32>, cross: u32) -> Self {
        Self { axis: Axis::Horizontal, sizes, cross, spacing: 0 }
    }

    pub fn vertical(sizes: Vec<u32>, cross: u32) -> Self {
        Self { axis: Axis::Vertical, sizes, cross, spacing: 0 }
    }

    pub fn with_spacing(mut self, spacing: u32) -> Self {
        self.spacing = spacing;
        self
    }

    fn axis_offset(&self, idx: usize) -> u32 {
        let mut x: u32 = 0;
        for i in 0..idx {
            x = x.saturating_add(self.sizes[i]);
            x = x.saturating_add(self.spacing);
        }
        x
    }
}

impl CollectionLayout for LinearLayout {
    fn content_size(&self) -> (u32, u32) {
        let n = self.sizes.len();
        if n == 0 { return (0, 0); }
        let total: u32 = self.sizes.iter().copied().sum::<u32>()
            + self.spacing * ((n as u32).saturating_sub(1));
        match self.axis {
            Axis::Horizontal => (total, self.cross),
            Axis::Vertical => (self.cross, total),
        }
    }

    fn item_count(&self) -> usize { self.sizes.len() }

    fn rect_for(&self, idx: usize) -> VRect {
        let off = self.axis_offset(idx);
        let size = self.sizes[idx];
        match self.axis {
            Axis::Horizontal => VRect { x: off, y: 0, w: size, h: self.cross },
            Axis::Vertical => VRect { x: 0, y: off, w: self.cross, h: size },
        }
    }

    fn items_in(&self, viewport: VRect) -> Vec<usize> {
        let mut out = Vec::new();
        let mut off: u32 = 0;
        for (i, size) in self.sizes.iter().enumerate() {
            let rect = match self.axis {
                Axis::Horizontal => VRect { x: off, y: 0, w: *size, h: self.cross },
                Axis::Vertical => VRect { x: 0, y: off, w: self.cross, h: *size },
            };
            if rect.intersects(viewport) { out.push(i); }
            off = off.saturating_add(*size).saturating_add(self.spacing);
            let past = match self.axis {
                Axis::Horizontal => off >= viewport.end_x(),
                Axis::Vertical => off >= viewport.end_y(),
            };
            if past && !rect.intersects(viewport) { break; }
        }
        out
    }

    fn decorations_in(&self, viewport: VRect) -> Vec<(usize, VRect)> {
        if self.spacing == 0 || self.sizes.len() < 2 { return Vec::new(); }
        let mut out = Vec::new();
        let mut off: u32 = 0;
        for i in 0..self.sizes.len() {
            off = off.saturating_add(self.sizes[i]);
            if i + 1 < self.sizes.len() {
                let rect = match self.axis {
                    Axis::Horizontal => VRect { x: off, y: 0, w: self.spacing, h: self.cross },
                    Axis::Vertical => VRect { x: 0, y: off, w: self.cross, h: self.spacing },
                };
                if rect.intersects(viewport) { out.push((i, rect)); }
                off = off.saturating_add(self.spacing);
            }
        }
        out
    }
}

/// Constant-stride 1D layout — every item has the same size along the major
/// axis. Designed for huge collections (editor lines, log entries) where
/// materializing per-item sizes is impractical: `rect_for` is O(1) and
/// `items_in` is O(visible).
pub struct UniformLayout {
    pub axis: Axis,
    pub count: usize,
    pub item_size: u32,
    pub cross: u32,
    pub spacing: u32,
}

impl UniformLayout {
    pub fn horizontal(count: usize, item_size: u32, cross: u32) -> Self {
        Self { axis: Axis::Horizontal, count, item_size, cross, spacing: 0 }
    }

    pub fn vertical(count: usize, item_size: u32, cross: u32) -> Self {
        Self { axis: Axis::Vertical, count, item_size, cross, spacing: 0 }
    }

    pub fn with_spacing(mut self, spacing: u32) -> Self {
        self.spacing = spacing;
        self
    }

    fn stride(&self) -> u64 {
        self.item_size as u64 + self.spacing as u64
    }
}

impl CollectionLayout for UniformLayout {
    fn content_size(&self) -> (u32, u32) {
        if self.count == 0 { return (0, 0); }
        let total = (self.item_size as u64 * self.count as u64
            + self.spacing as u64 * (self.count as u64 - 1))
            .min(u32::MAX as u64) as u32;
        match self.axis {
            Axis::Horizontal => (total, self.cross),
            Axis::Vertical => (self.cross, total),
        }
    }

    fn item_count(&self) -> usize { self.count }

    fn rect_for(&self, idx: usize) -> VRect {
        let off = (idx as u64 * self.stride()).min(u32::MAX as u64) as u32;
        match self.axis {
            Axis::Horizontal => VRect { x: off, y: 0, w: self.item_size, h: self.cross },
            Axis::Vertical => VRect { x: 0, y: off, w: self.cross, h: self.item_size },
        }
    }

    fn items_in(&self, viewport: VRect) -> Vec<usize> {
        if self.count == 0 || self.item_size == 0 { return Vec::new(); }
        let stride = self.stride();
        let item_size = self.item_size as u64;
        let (lo, hi) = match self.axis {
            Axis::Horizontal => (viewport.x as u64, viewport.end_x() as u64),
            Axis::Vertical => (viewport.y as u64, viewport.end_y() as u64),
        };
        if hi == 0 { return Vec::new(); }
        // First visible item: smallest i where i*stride + item_size > lo. The
        // spacing-skipping case matters here — a viewport landing inside a gap
        // should not include the previous item.
        let first = if lo < item_size {
            0usize
        } else {
            let n = lo - item_size + 1;
            (n.div_ceil(stride)) as usize
        };
        let last = ((hi - 1) / stride) as usize;
        if first >= self.count { return Vec::new(); }
        let last = last.min(self.count - 1);
        if first > last { return Vec::new(); }
        (first..=last).collect()
    }
}

// -- State ------------------------------------------------------------------

/// Persistent state for one collection view. Owned by the data layer (e.g. a
/// `Frame` that hosts a tab strip). Mutated via the methods on this type so
/// scroll math stays in one place.
#[derive(Default, Copy, Clone, Debug, PartialEq, Eq)]
pub struct CollectionState {
    pub scroll_x: u32,
    pub scroll_y: u32,
}

impl CollectionState {
    pub fn scroll(&self) -> (u32, u32) { (self.scroll_x, self.scroll_y) }

    pub fn set_scroll(&mut self, x: u32, y: u32, content: (u32, u32), viewport: (u32, u32)) {
        let max_x = content.0.saturating_sub(viewport.0);
        let max_y = content.1.saturating_sub(viewport.1);
        self.scroll_x = x.min(max_x);
        self.scroll_y = y.min(max_y);
    }

    pub fn scroll_by(&mut self, dx: isize, dy: isize, content: (u32, u32), viewport: (u32, u32)) {
        let max_x = content.0.saturating_sub(viewport.0) as i64;
        let max_y = content.1.saturating_sub(viewport.1) as i64;
        let nx = (self.scroll_x as i64 + dx as i64).clamp(0, max_x);
        let ny = (self.scroll_y as i64 + dy as i64).clamp(0, max_y);
        self.scroll_x = nx as u32;
        self.scroll_y = ny as u32;
    }

    /// Sticky scroll: bump the smallest amount needed to bring `target` fully
    /// inside `viewport`. No-op if already in view.
    pub fn ensure_visible(&mut self, target: VRect, content: (u32, u32), viewport: (u32, u32)) {
        let mut sx = self.scroll_x;
        let mut sy = self.scroll_y;
        if target.x < sx {
            sx = target.x;
        } else if target.end_x() > sx.saturating_add(viewport.0) {
            sx = target.end_x().saturating_sub(viewport.0);
        }
        if target.y < sy {
            sy = target.y;
        } else if target.end_y() > sy.saturating_add(viewport.1) {
            sy = target.end_y().saturating_sub(viewport.1);
        }
        let max_x = content.0.saturating_sub(viewport.0);
        let max_y = content.1.saturating_sub(viewport.1);
        self.scroll_x = sx.min(max_x);
        self.scroll_y = sy.min(max_y);
    }
}

// -- Pass (one render iteration) -------------------------------------------

/// One render pass over a layout. Borrows the layout, state, and screen area;
/// produces visible-item / visible-decoration iterators and hit-tests.
pub struct CollectionPass<'a, L: CollectionLayout> {
    pub layout: &'a L,
    pub state: &'a CollectionState,
    pub area: Rect,
}

impl<'a, L: CollectionLayout> CollectionPass<'a, L> {
    pub fn new(layout: &'a L, state: &'a CollectionState, area: Rect) -> Self {
        Self { layout, state, area }
    }

    pub fn viewport(&self) -> VRect {
        VRect {
            x: self.state.scroll_x,
            y: self.state.scroll_y,
            w: self.area.width as u32,
            h: self.area.height as u32,
        }
    }

    /// Iterate visible items, projected to screen geometry.
    pub fn visible_items(&self) -> impl Iterator<Item = (usize, CellGeometry)> + '_ {
        let scroll = self.state.scroll();
        let area = self.area;
        let layout = self.layout;
        layout.items_in(self.viewport()).into_iter()
            .filter_map(move |idx| {
                project_to_screen(layout.rect_for(idx), scroll, area).map(|g| (idx, g))
            })
    }

    /// Iterate visible decorations, projected to screen geometry. Decoration
    /// ids are layout-defined.
    pub fn visible_decorations(&self) -> impl Iterator<Item = (usize, CellGeometry)> + '_ {
        let scroll = self.state.scroll();
        let area = self.area;
        self.layout.decorations_in(self.viewport()).into_iter()
            .filter_map(move |(id, v)| project_to_screen(v, scroll, area).map(|g| (id, g)))
    }

    /// Item under screen-space (col, row), if any.
    pub fn item_at_screen(&self, col: u16, row: u16) -> Option<usize> {
        if col < self.area.x || col >= self.area.x.saturating_add(self.area.width)
            || row < self.area.y || row >= self.area.y.saturating_add(self.area.height)
        {
            return None;
        }
        let vx = (col - self.area.x) as u32 + self.state.scroll_x;
        let vy = (row - self.area.y) as u32 + self.state.scroll_y;
        self.layout.items_in(self.viewport())
            .into_iter()
            .find(|&idx| self.layout.rect_for(idx).contains(vx, vy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(sizes: Vec<u32>, spacing: u32) -> LinearLayout {
        LinearLayout::horizontal(sizes, 1).with_spacing(spacing)
    }

    #[test]
    fn vrect_intersects_overlapping_rects() {
        let a = VRect { x: 0, y: 0, w: 10, h: 1 };
        let b = VRect { x: 5, y: 0, w: 10, h: 1 };
        assert!(a.intersects(b));
        let c = VRect { x: 10, y: 0, w: 1, h: 1 };
        assert!(!a.intersects(c), "edges that touch but don't overlap are not intersecting");
    }

    #[test]
    fn project_to_screen_full_visible() {
        let virt = VRect { x: 5, y: 0, w: 4, h: 1 };
        let area = Rect { x: 100, y: 50, width: 20, height: 1 };
        let g = project_to_screen(virt, (0, 0), area).unwrap();
        assert_eq!(g.screen, Rect { x: 105, y: 50, width: 4, height: 1 });
        assert_eq!((g.clip_left, g.clip_right), (0, 0));
    }

    #[test]
    fn project_to_screen_clipped_left() {
        let virt = VRect { x: 10, y: 0, w: 8, h: 1 };
        let area = Rect { x: 0, y: 0, width: 20, height: 1 };
        let g = project_to_screen(virt, (12, 0), area).unwrap();
        assert_eq!(g.screen.x, 0);
        assert_eq!(g.screen.width, 6);
        assert_eq!(g.clip_left, 2);
        assert_eq!(g.clip_right, 0);
    }

    #[test]
    fn project_to_screen_off_screen_returns_none() {
        let virt = VRect { x: 0, y: 0, w: 10, h: 1 };
        let area = Rect { x: 0, y: 0, width: 5, height: 1 };
        assert!(project_to_screen(virt, (20, 0), area).is_none());
    }

    #[test]
    fn linear_layout_rects_account_for_spacing() {
        let l = h(vec![3, 5, 4], 1);
        assert_eq!(l.content_size(), (3 + 1 + 5 + 1 + 4, 1));
        assert_eq!(l.rect_for(0), VRect { x: 0, y: 0, w: 3, h: 1 });
        assert_eq!(l.rect_for(1), VRect { x: 4, y: 0, w: 5, h: 1 });
        assert_eq!(l.rect_for(2), VRect { x: 10, y: 0, w: 4, h: 1 });
    }

    #[test]
    fn linear_layout_decorations_sit_between_items() {
        let l = h(vec![3, 5, 4], 1);
        let decs = l.decorations_in(VRect { x: 0, y: 0, w: 100, h: 1 });
        assert_eq!(decs.len(), 2);
        assert_eq!(decs[0], (0, VRect { x: 3, y: 0, w: 1, h: 1 }));
        assert_eq!(decs[1], (1, VRect { x: 9, y: 0, w: 1, h: 1 }));
    }

    #[test]
    fn linear_layout_items_in_skips_off_screen() {
        let l = h(vec![5; 10], 1);
        // viewport x=12..19 hits item 2 (12..17) and item 3 (18..23).
        let visible = l.items_in(VRect { x: 12, y: 0, w: 7, h: 1 });
        assert_eq!(visible, vec![2, 3]);
        let visible = l.items_in(VRect { x: 0, y: 0, w: 10, h: 1 });
        assert_eq!(visible, vec![0, 1]);
    }

    #[test]
    fn ensure_visible_sticky_does_not_move_when_already_in_view() {
        let layout = h(vec![5; 10], 1);
        let content = layout.content_size();
        let mut st = CollectionState { scroll_x: 8, scroll_y: 0 };
        st.ensure_visible(layout.rect_for(2), content, (10, 1));
        assert_eq!(st.scroll(), (8, 0));
    }

    #[test]
    fn ensure_visible_scrolls_minimum_to_show_item() {
        let layout = h(vec![5; 10], 1);
        let content = layout.content_size();
        let mut st = CollectionState::default();
        st.ensure_visible(layout.rect_for(9), content, (10, 1));
        assert_eq!(st.scroll_x, 49);
    }

    #[test]
    fn uniform_layout_rect_for_is_constant_stride() {
        let l = UniformLayout::vertical(1_000_000, 1, 80);
        assert_eq!(l.rect_for(0), VRect { x: 0, y: 0, w: 80, h: 1 });
        assert_eq!(l.rect_for(42), VRect { x: 0, y: 42, w: 80, h: 1 });
    }

    #[test]
    fn uniform_layout_items_in_only_walks_visible() {
        let l = UniformLayout::vertical(1_000, 1, 80);
        let visible = l.items_in(VRect { x: 0, y: 100, w: 80, h: 5 });
        assert_eq!(visible, vec![100, 101, 102, 103, 104]);
    }

    #[test]
    fn uniform_layout_with_spacing_skips_gap() {
        let l = UniformLayout::vertical(4, 2, 80).with_spacing(1);
        assert_eq!(l.rect_for(2), VRect { x: 0, y: 6, w: 80, h: 2 });
        let visible = l.items_in(VRect { x: 0, y: 5, w: 80, h: 4 });
        // Item 1 at y=3..5 ends exactly at viewport start → not visible.
        // Item 2 at y=6..8 → visible.
        assert_eq!(visible, vec![2]);
    }

    #[test]
    fn collection_pass_item_at_screen_accounts_for_scroll() {
        let layout = h(vec![5; 10], 1);
        let st = CollectionState { scroll_x: 6, scroll_y: 0 };
        let area = Rect { x: 0, y: 0, width: 10, height: 1 };
        let pass = CollectionPass::new(&layout, &st, area);
        assert_eq!(pass.item_at_screen(0, 0), Some(1));
        assert_eq!(pass.item_at_screen(5, 0), None);
        assert_eq!(pass.item_at_screen(6, 0), Some(2));
    }
}
