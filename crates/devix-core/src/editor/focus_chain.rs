//! Focus chain — owner of the active pane path.
//!
//! Carved out of the god-`Editor` per T-101. Holds the current focus
//! path (a `/pane(/<i>)*` index list) and produces typed
//! `FocusTransition`s only when the path actually changes — so
//! `Pulse::FocusChanged` fires once per real transition, not on
//! no-ops or on tree-shape adjustments that preserve the user's
//! logical focus (those go through `transform`).

use std::ops::Deref;

use devix_protocol::path::Path;
use devix_protocol::pulse::Pulse;

#[derive(Default)]
pub struct FocusChain {
    active: Vec<usize>,
}

impl FocusChain {
    pub fn new() -> Self {
        Self { active: Vec::new() }
    }

    /// The current focus path as `Split.children`-index list. An empty
    /// slice means focus is on the root pane.
    pub fn active(&self) -> &[usize] {
        &self.active
    }

    pub fn as_vec(&self) -> Vec<usize> {
        self.active.clone()
    }

    /// Replace the focus path. Returns a `FocusTransition` iff the path
    /// actually changed; the caller publishes the matching
    /// `Pulse::FocusChanged` exactly when this returns `Some`.
    pub fn replace(&mut self, new: Vec<usize>) -> Option<FocusTransition> {
        if self.active == new {
            return None;
        }
        let from = std::mem::replace(&mut self.active, new);
        Some(FocusTransition {
            from,
            to: self.active.clone(),
        })
    }

    /// Adjust the focus path in place without emitting a transition.
    /// Used after structural tree mutations that shift indices without
    /// moving the user's focus to a different leaf (e.g. inserting a
    /// sibling that pushes existing children one slot over).
    pub fn transform(&mut self, f: impl FnOnce(&mut Vec<usize>)) {
        f(&mut self.active);
    }
}

impl Deref for FocusChain {
    type Target = [usize];
    fn deref(&self) -> &[usize] {
        &self.active
    }
}

/// A real focus change: `from` and `to` are the pre- and post-mutation
/// `Split.children`-index lists. Convert into a `Pulse::FocusChanged`
/// via `into_pulse()`.
pub struct FocusTransition {
    pub from: Vec<usize>,
    pub to: Vec<usize>,
}

impl FocusTransition {
    pub fn into_pulse(self) -> Pulse {
        Pulse::FocusChanged {
            from: Some(pane_path_from_indices(&self.from)),
            to: Some(pane_path_from_indices(&self.to)),
        }
    }
}

fn pane_path_from_indices(indices: &[usize]) -> Path {
    let mut s = String::from("/pane");
    for i in indices {
        s.push('/');
        s.push_str(&i.to_string());
    }
    Path::parse(&s).expect("/pane(/<i>)* is canonical")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_returns_none_on_no_change() {
        let mut chain = FocusChain::new();
        chain.replace(vec![0, 1]);
        assert!(chain.replace(vec![0, 1]).is_none());
    }

    #[test]
    fn replace_returns_transition_on_change() {
        let mut chain = FocusChain::new();
        let t = chain.replace(vec![0]).unwrap();
        assert_eq!(t.from, Vec::<usize>::new());
        assert_eq!(t.to, vec![0]);
        let t = chain.replace(vec![1, 2]).unwrap();
        assert_eq!(t.from, vec![0]);
        assert_eq!(t.to, vec![1, 2]);
    }

    #[test]
    fn transform_does_not_produce_transition() {
        let mut chain = FocusChain::new();
        chain.replace(vec![0]);
        chain.transform(|p| p.insert(0, 0));
        assert_eq!(chain.active(), &[0, 0]);
    }

    #[test]
    fn transition_into_pulse_round_trips_paths() {
        let t = FocusTransition { from: vec![0], to: vec![1, 2] };
        match t.into_pulse() {
            Pulse::FocusChanged { from, to } => {
                assert_eq!(from.unwrap().as_str(), "/pane/0");
                assert_eq!(to.unwrap().as_str(), "/pane/1/2");
            }
            _ => panic!("wrong pulse"),
        }
    }
}
