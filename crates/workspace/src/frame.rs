//! Frame = editor tab group: a strip of tabs plus an active index.
//! Each tab is a ViewId.

use slotmap::new_key_type;

use crate::view::ViewId;

new_key_type! { pub struct FrameId; }

pub struct Frame {
    pub tabs: Vec<ViewId>,
    pub active_tab: usize,
}

impl Frame {
    pub fn with_view(view: ViewId) -> Self {
        Self { tabs: vec![view], active_tab: 0 }
    }

    /// Returns None if `tabs` is empty or `active_tab` is out of bounds.
    /// Construction via `with_view` guarantees a valid index, but tab-mutating
    /// callers must restore the invariant after every change.
    pub fn active_view(&self) -> Option<ViewId> {
        self.tabs.get(self.active_tab).copied()
    }
}
