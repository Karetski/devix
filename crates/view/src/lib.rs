//! View = per-frame editor state (selection + sticky col + scroll).
//!
//! Lives in its own crate so consumers that operate on view state
//! (commands, palette, render) don't need to depend on the entire
//! `devix-surface` aggregate just to read a `View`.

pub mod view;

pub use view::{ScrollMode, View, ViewId};
