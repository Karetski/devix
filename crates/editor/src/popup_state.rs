//! Per-view popup state (hover + completion). Lives here, not on the
//! Surface, because the popups are rendered by Panes that live in this
//! crate (`HoverPane`, `CompletionPane`) and the bookkeeping is editor
//! concerns. Surface's `View` struct still owns `Option<HoverState>` /
//! `Option<CompletionState>` fields — the types are imported across the
//! crate boundary.

use lsp_types::CompletionItem;

/// Per-view hover popup state. Hover is request-driven: dispatch records
/// `Pending` with the cursor's char index, the LSP coordinator pumps back
/// a response, and the App-side drain matches it against `anchor_char` —
/// stale answers (cursor moved) are dropped on the floor.
#[derive(Clone, Debug)]
pub struct HoverState {
    pub anchor_char: usize,
    pub status: HoverStatus,
}

#[derive(Clone, Debug)]
pub enum HoverStatus {
    Pending,
    Ready(Vec<String>),
    Empty,
}

/// Per-view completion popup state. The dispatcher records `Pending` on
/// trigger; the App-side drain swaps in items as the response lands. As
/// the user types, `query_start..cursor` slices the rope to form the
/// filter prefix; `filtered` ranks `items` by match score against that
/// prefix.
#[derive(Clone, Debug)]
pub struct CompletionState {
    /// Char offset at request time. Stale-response guard.
    pub anchor_char: usize,
    /// Char offset at which the user-typed query begins. Updated only on
    /// `TriggerCompletion`; subsequent typing extends the query rightward
    /// (the cursor) without moving its left edge.
    pub query_start: usize,
    /// All items the server returned. Owned to avoid lifetime soup.
    pub items: Vec<CompletionItem>,
    /// Per-item lowercased label, parallel to `items`. Built once when
    /// items land so the per-keystroke refilter can do case-insensitive
    /// `find` without allocating a String per item per keystroke.
    pub labels_lower: Vec<String>,
    /// Indices into `items`, in current match order. Empty until response
    /// arrives or while the typed prefix matches nothing.
    pub filtered: Vec<usize>,
    /// Index into `filtered` of the highlighted row.
    pub selected: usize,
    pub status: CompletionStatus,
}

impl CompletionState {
    /// Replace items and refresh the lowercased-label cache. The two must
    /// stay in lockstep for refilter to address them by the same index.
    pub fn set_items(&mut self, items: Vec<CompletionItem>) {
        self.labels_lower = items.iter().map(|i| i.label.to_lowercase()).collect();
        self.items = items;
    }
}

#[derive(Clone, Debug)]
pub enum CompletionStatus {
    /// Request in flight; popup shows a placeholder row.
    Pending,
    /// Items received and filtered; popup renders normally.
    Ready,
}
