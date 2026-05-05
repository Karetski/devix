//! Editor state coordinator: owns the layout tree, focus, modal slot,
//! and the SlotMaps that index Documents and Views by id.
//!
//! After Phase 4 of the refactor:
//! - `Document` / `DocId` live in `devix-workspace`.
//! - `View` / `ViewId` / `ScrollMode` live in `devix-view`.
//! - `EditorPane` and the layout composites live in `devix-editor`.
//! - Commands, registry, keymap, modal logic live in `devix-commands`.
//! - This crate keeps the `Surface` aggregate that ties them together.

pub mod frame;
pub mod layout;
pub mod surface;
pub mod tree;

pub use tree::{
    LayoutFrame, find_frame, find_frame_mut, frame_ids, leaves_with_rects, pane_at_indices,
    pane_leaf_id,
};
pub use devix_view::{ScrollMode, View, ViewId};
pub use devix_workspace::{DocId, Document};
pub use frame::FrameId;
pub use layout::{Axis, Direction, SidebarSlot};
pub use surface::{LeafRef, RenderCache, Surface, TabHit, TabStripCache, TabStripHit};

/// Compatibility module so call sites that imported `devix_surface::view::*`
/// keep working after the type moved to `devix-view`. New code should import
/// from `devix_view` directly.
pub mod view {
    pub use devix_view::*;
}
