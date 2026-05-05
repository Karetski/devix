//! Layout/virtualization primitives modeled on `UICollectionView`.
//!
//! Three orthogonal concerns:
//!
//! * **Layout** ([`CollectionLayout`]) — pure mapping from item index to
//!   virtual rect. Knows nothing about scroll, screen, or input. Examples:
//!   [`LinearLayout`] (1D flow), [`UniformLayout`] (constant-stride 1D for
//!   huge counts), future grid / waterfall / compositional.
//! * **Projection** ([`CollectionPass`], [`project_to_screen`]) — given a
//!   layout, a scroll offset, and a screen area, computes per-item screen
//!   geometry plus edge clip information.
//! * **Scroll math** ([`scroll_by`], [`set_scroll`], [`ensure_visible`]) —
//!   free functions that mutate a raw `(u32, u32)` scroll offset against a
//!   `(content, viewport)` pair. Surface stores scroll as a plain tuple so
//!   the model crate has no view dependency; renderers and input handlers
//!   reach for the math here.
//!
//! Virtual coordinates are `u32` so a layout can address a far larger content
//! area than the screen (e.g. a million-line buffer) — the screen `Rect`
//! stays `u16` and the projection code converts at the boundary.
//!
//! There is intentionally no "cell" type and no rendering code in this
//! module. Cells are whatever the caller decides to draw inside
//! [`CellGeometry::screen`].

use ratatui::layout::Rect;

// -- Hit --------------------------------------------------------------------

/// One clickable region produced by a render pass — the screen rect a given
/// item painted into. Renderers stash `Hit`s in their output; input handlers
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

// -- Scroll math ------------------------------------------------------------

/// Clamp a scroll offset so the viewport stays inside content.
pub fn set_scroll(
    scroll: &mut (u32, u32),
    x: u32,
    y: u32,
    content: (u32, u32),
    viewport: (u32, u32),
) {
    let max_x = content.0.saturating_sub(viewport.0);
    let max_y = content.1.saturating_sub(viewport.1);
    scroll.0 = x.min(max_x);
    scroll.1 = y.min(max_y);
}

/// Move a scroll offset by signed deltas, clamped to content/viewport.
pub fn scroll_by(
    scroll: &mut (u32, u32),
    dx: isize,
    dy: isize,
    content: (u32, u32),
    viewport: (u32, u32),
) {
    let max_x = content.0.saturating_sub(viewport.0) as i64;
    let max_y = content.1.saturating_sub(viewport.1) as i64;
    let nx = (scroll.0 as i64 + dx as i64).clamp(0, max_x);
    let ny = (scroll.1 as i64 + dy as i64).clamp(0, max_y);
    scroll.0 = nx as u32;
    scroll.1 = ny as u32;
}

/// Sticky scroll: bump the smallest amount needed to bring `target` fully
/// inside `viewport`. No-op if already in view.
pub fn ensure_visible(
    scroll: &mut (u32, u32),
    target: VRect,
    content: (u32, u32),
    viewport: (u32, u32),
) {
    let mut sx = scroll.0;
    let mut sy = scroll.1;
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
    scroll.0 = sx.min(max_x);
    scroll.1 = sy.min(max_y);
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

pub use crate::Axis;

/// 1D flow layout — items along an axis with optional spacing between them.
/// Spacing is reported as decorations (id = index of the item *before* the
/// gap), so renderers can paint separators inside it. O(1) `rect_for` via
/// a precomputed prefix-sum cache; use [`UniformLayout`] for huge
/// constant-stride collections where even storing per-item sizes is
/// wasteful.
pub struct LinearLayout {
    pub axis: Axis,
    sizes: Vec<u32>,
    pub cross: u32,
    spacing: u32,
    /// Cumulative axis offsets. `offsets[i]` is the start position of item
    /// `i` along the axis; `offsets[n]` is the layout's total axis extent
    /// including the final inter-item gap *not* added past the last item.
    /// Built once in the constructor; O(1) `rect_for`.
    offsets: Vec<u32>,
}

impl LinearLayout {
    pub fn horizontal(sizes: Vec<u32>, cross: u32) -> Self {
        Self::new(Axis::Horizontal, sizes, cross, 0)
    }

    pub fn vertical(sizes: Vec<u32>, cross: u32) -> Self {
        Self::new(Axis::Vertical, sizes, cross, 0)
    }

    pub fn with_spacing(self, spacing: u32) -> Self {
        Self::new(self.axis, self.sizes, self.cross, spacing)
    }

    fn new(axis: Axis, sizes: Vec<u32>, cross: u32, spacing: u32) -> Self {
        let offsets = compute_offsets(&sizes, spacing);
        Self { axis, sizes, cross, spacing, offsets }
    }

    pub fn sizes(&self) -> &[u32] { &self.sizes }
    pub fn spacing(&self) -> u32 { self.spacing }

    fn axis_offset(&self, idx: usize) -> u32 {
        // Constructor guarantees offsets.len() == sizes.len() + 1.
        self.offsets[idx]
    }
}

/// Build the prefix-sum cache: `out[i] = sum(sizes[..i]) + spacing*i`.
/// Length is `sizes.len() + 1` so `out.last()` is the total axis extent
/// (used by `content_size`).
fn compute_offsets(sizes: &[u32], spacing: u32) -> Vec<u32> {
    let mut out = Vec::with_capacity(sizes.len() + 1);
    let mut acc: u32 = 0;
    out.push(0);
    for (i, &s) in sizes.iter().enumerate() {
        acc = acc.saturating_add(s);
        if i + 1 < sizes.len() {
            acc = acc.saturating_add(spacing);
        }
        out.push(acc);
    }
    out
}

impl CollectionLayout for LinearLayout {
    fn content_size(&self) -> (u32, u32) {
        if self.sizes.is_empty() { return (0, 0); }
        // offsets.last() == sum(sizes) + spacing*(n-1).
        let total = *self.offsets.last().unwrap();
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
        // rect_for is now O(1); the early-out keeps us O(visible) instead of
        // O(n) in the common case where many items lie past the viewport.
        let viewport_end = match self.axis {
            Axis::Horizontal => viewport.end_x(),
            Axis::Vertical => viewport.end_y(),
        };
        let mut out = Vec::new();
        for i in 0..self.sizes.len() {
            let rect = self.rect_for(i);
            if rect.intersects(viewport) { out.push(i); }
            if self.offsets[i + 1] >= viewport_end {
                break;
            }
        }
        out
    }

    fn decorations_in(&self, viewport: VRect) -> Vec<(usize, VRect)> {
        if self.spacing == 0 || self.sizes.len() < 2 { return Vec::new(); }
        let mut out = Vec::new();
        for i in 0..self.sizes.len() - 1 {
            // Gap between item i and item i+1: its start is offsets[i] + sizes[i].
            let off = self.offsets[i].saturating_add(self.sizes[i]);
            let rect = match self.axis {
                Axis::Horizontal => VRect { x: off, y: 0, w: self.spacing, h: self.cross },
                Axis::Vertical => VRect { x: 0, y: off, w: self.cross, h: self.spacing },
            };
            if rect.intersects(viewport) { out.push((i, rect)); }
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

// -- Pass (one render iteration) -------------------------------------------

/// One render pass over a layout. Borrows the layout and screen area, takes
/// the scroll offset by value (it's just two `u32`s); produces visible-item
/// / visible-decoration iterators and hit-tests.
pub struct CollectionPass<'a, L: CollectionLayout> {
    pub layout: &'a L,
    pub scroll: (u32, u32),
    pub area: Rect,
}

impl<'a, L: CollectionLayout> CollectionPass<'a, L> {
    pub fn new(layout: &'a L, scroll: (u32, u32), area: Rect) -> Self {
        Self { layout, scroll, area }
    }

    pub fn viewport(&self) -> VRect {
        VRect {
            x: self.scroll.0,
            y: self.scroll.1,
            w: self.area.width as u32,
            h: self.area.height as u32,
        }
    }

    /// Iterate visible items, projected to screen geometry.
    pub fn visible_items(&self) -> impl Iterator<Item = (usize, CellGeometry)> + '_ {
        let scroll = self.scroll;
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
        let scroll = self.scroll;
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
        let vx = (col - self.area.x) as u32 + self.scroll.0;
        let vy = (row - self.area.y) as u32 + self.scroll.1;
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
        let visible = l.items_in(VRect { x: 12, y: 0, w: 7, h: 1 });
        assert_eq!(visible, vec![2, 3]);
        let visible = l.items_in(VRect { x: 0, y: 0, w: 10, h: 1 });
        assert_eq!(visible, vec![0, 1]);
    }

    #[test]
    fn ensure_visible_sticky_does_not_move_when_already_in_view() {
        let layout = h(vec![5; 10], 1);
        let content = layout.content_size();
        let mut scroll = (8u32, 0u32);
        ensure_visible(&mut scroll, layout.rect_for(2), content, (10, 1));
        assert_eq!(scroll, (8, 0));
    }

    #[test]
    fn ensure_visible_scrolls_minimum_to_show_item() {
        let layout = h(vec![5; 10], 1);
        let content = layout.content_size();
        let mut scroll = (0u32, 0u32);
        ensure_visible(&mut scroll, layout.rect_for(9), content, (10, 1));
        assert_eq!(scroll.0, 49);
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
        assert_eq!(visible, vec![2]);
    }

    #[test]
    fn collection_pass_item_at_screen_accounts_for_scroll() {
        let layout = h(vec![5; 10], 1);
        let area = Rect { x: 0, y: 0, width: 10, height: 1 };
        let pass = CollectionPass::new(&layout, (6, 0), area);
        assert_eq!(pass.item_at_screen(0, 0), Some(1));
        assert_eq!(pass.item_at_screen(5, 0), None);
        assert_eq!(pass.item_at_screen(6, 0), Some(2));
    }
}
