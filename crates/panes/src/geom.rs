//! Geometry primitives shared by every Pane.
//!
//! `Rect` is re-exported from `ratatui` rather than re-defined; the editor
//! draws into ratatui frames and a parallel rectangle type would force
//! conversions at every boundary for no gain. If we ever swap out ratatui,
//! the import surface is one line.

pub use ratatui::layout::Rect;

/// Where on the parent rect a popup-style child anchors itself. Used by
/// overlays (hover, completion, future tooltips) so they can position
/// themselves relative to a specific cell rather than to a fixed corner.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub col: u16,
    pub row: u16,
    pub edge: AnchorEdge,
}

/// Which side of the anchor cell the popup grows toward. Determines whether
/// the overlay paints above-or-below / left-or-right of its anchor and how
/// it should clip if the screen edge is close.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AnchorEdge {
    Above,
    Below,
    Left,
    Right,
}
