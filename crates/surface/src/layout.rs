//! Re-export of the layout primitives that live in `devix-core`.
//!
//! Surface used to define `Axis`, `Direction`, and `SidebarSlot` here, but
//! `editor` and `ui` had parallel copies. The canonical home is now
//! `devix_core::layout`; this module just re-exports for source-compat.

pub use devix_core::layout::{Axis, Direction, SidebarSlot};
