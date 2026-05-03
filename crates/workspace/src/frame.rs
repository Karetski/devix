//! Frame = editor tab group: a strip of tabs plus an active index.
//! Each tab is a ViewId.

use devix_collection::CollectionState;
use slotmap::new_key_type;

use crate::view::ViewId;

new_key_type! { pub struct FrameId; }

pub struct Frame {
    pub tabs: Vec<ViewId>,
    pub active_tab: usize,
    /// Scroll state for this frame's tab strip. Owned here so it survives
    /// across renders / resizes; mutated through the `devix_collection` API.
    pub tab_strip_state: CollectionState,
    /// One-shot signal asking the next tab-strip render to scroll the active
    /// tab into view. Set by mutators that change `active_tab` (keyboard nav,
    /// new tab, close), cleared by the renderer once consumed. Click-to-select
    /// intentionally leaves it false so the strip stays put under the cursor.
    pub recenter_active: bool,
}

impl Frame {
    pub fn with_view(view: ViewId) -> Self {
        Self {
            tabs: vec![view],
            active_tab: 0,
            tab_strip_state: CollectionState::default(),
            recenter_active: true,
        }
    }

    /// Activate a tab and request scroll-to-visible. Use for keyboard nav and
    /// tab-mutating operations (new/close) where the strip should follow.
    pub fn set_active(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active_tab = idx;
        }
        self.recenter_active = true;
    }

    /// Activate a tab without disturbing scroll. Use for click activation —
    /// the user already pointed at the tab they want.
    pub fn select_visible(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active_tab = idx;
        }
    }

    /// Returns None if `tabs` is empty or `active_tab` is out of bounds.
    /// Construction via `with_view` guarantees a valid index, but tab-mutating
    /// callers must restore the invariant after every change.
    pub fn active_view(&self) -> Option<ViewId> {
        self.tabs.get(self.active_tab).copied()
    }
}
