//! Editor state coordinator: owns the layout tree, focus, modal slot,
//! and the SlotMaps that index Documents and Views by id.

pub mod frame;
pub mod layout;
pub mod services;
pub mod surface;
pub mod tree;
pub mod view;

pub use services::RenderServices;
pub use tree::{
    LayoutFrame, find_frame, find_frame_mut, frame_ids, leaves_with_rects, pane_at_indices,
    pane_leaf_id,
};
pub use view::{ScrollMode, View, ViewId};
pub use devix_workspace::{DocId, Document};
pub use frame::FrameId;
pub use layout::{Axis, Direction, SidebarSlot};
pub use surface::{LeafRef, RenderCache, Surface, TabHit, TabStripCache, TabStripHit};
