//! Layout primitives shared across the workspace.
//!
//! `Axis`, `Direction`, and `SidebarSlot` live here because three or more
//! crates (surface tree, editor pane composites, ui virtualization) need the
//! same Horizontal/Vertical concept and were each defining their own copy.
//! Putting them in `core` lets every layer use the same identity without
//! creating cross-crate cycles.

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Down,
    Up,
    Right,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SidebarSlot {
    Left,
    Right,
}

/// Partition `area` along `axis` proportionally to the integer `weights`.
/// Returns one sub-rect per weight; the last rect absorbs any remainder so
/// the union of returned rects exactly equals `area`. Mirrors the
/// `Layout::Ratio` math the codebase used to call into `ratatui::Layout`
/// for, kept here as a pure helper so model crates don't need a renderer
/// dependency.
pub fn split_rects(area: crate::geom::Rect, axis: Axis, weights: &[u16]) -> Vec<crate::geom::Rect> {
    use crate::geom::Rect;
    if weights.is_empty() {
        return Vec::new();
    }
    let total: u32 = weights.iter().map(|w| (*w as u32).max(1)).sum::<u32>().max(1);
    let mut out = Vec::with_capacity(weights.len());
    let last = weights.len() - 1;
    match axis {
        Axis::Horizontal => {
            let full = area.width as u32;
            let mut consumed: u32 = 0;
            for (i, w) in weights.iter().enumerate() {
                let part = if i == last {
                    full.saturating_sub(consumed)
                } else {
                    full * (*w as u32).max(1) / total
                };
                let x = area.x.saturating_add(consumed as u16);
                out.push(Rect {
                    x,
                    y: area.y,
                    width: part as u16,
                    height: area.height,
                });
                consumed = consumed.saturating_add(part);
            }
        }
        Axis::Vertical => {
            let full = area.height as u32;
            let mut consumed: u32 = 0;
            for (i, w) in weights.iter().enumerate() {
                let part = if i == last {
                    full.saturating_sub(consumed)
                } else {
                    full * (*w as u32).max(1) / total
                };
                let y = area.y.saturating_add(consumed as u16);
                out.push(Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: part as u16,
                });
                consumed = consumed.saturating_add(part);
            }
        }
    }
    out
}
