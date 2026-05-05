//! `FrameId` — a stable identifier for an editor frame.
//!
//! Phase 3c follow-up: frames now own their state directly on
//! `LayoutFrame` (in `crate::tree`); there's no `Frame` struct or
//! `Editor.frames` slotmap any more. `FrameId` survives because the
//! render cache (`render_cache.frame_rects`, `tab_strips`) keys against
//! it across renders — the layout tree's pointer identity is too
//! fragile for that role (refs move on tree mutation).
//!
//! Ids are minted via a process-wide monotonic counter. Single-process
//! editor — no need for slotmap recycling. The counter starts at 1 so
//! the slotmap-style "null" pattern (id == 0) keeps working as a
//! sentinel if we ever need it.

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct FrameId(u64);

static NEXT: AtomicU64 = AtomicU64::new(1);

/// Mint a fresh `FrameId`. Process-monotonic; never recycled.
pub fn mint_id() -> FrameId {
    FrameId(NEXT.fetch_add(1, Ordering::Relaxed))
}

impl FrameId {
    /// Sentinel "no frame" id — useful for placeholder values during a
    /// take-and-replace mutation step. Distinct from every minted id
    /// because [`mint_id`] starts at 1.
    pub fn null() -> Self {
        FrameId(0)
    }
}
