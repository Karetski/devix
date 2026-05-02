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

    pub fn active_view(&self) -> ViewId {
        self.tabs[self.active_tab]
    }
}
