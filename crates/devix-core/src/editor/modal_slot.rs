//! Modal slot — owner of the at-most-one active modal Pane.
//!
//! Carved out of the god-`Editor` per T-103. Holds the (Pane,
//! ModalKind) pair, enforces the single-modal-at-a-time invariant
//! (opening a new modal while one's active drops the previous), and
//! exposes the typed `ModalKind` so `Editor::open_modal` /
//! `dismiss_modal` can publish `Pulse::ModalOpened` /
//! `Pulse::ModalDismissed` on transitions.

use devix_protocol::pulse::ModalKind;

use crate::Pane;

#[derive(Default)]
pub struct ModalSlot {
    current: Option<Box<dyn Pane>>,
    kind: Option<ModalKind>,
}

impl ModalSlot {
    pub fn new() -> Self {
        Self { current: None, kind: None }
    }

    pub fn is_some(&self) -> bool {
        self.current.is_some()
    }

    pub fn is_none(&self) -> bool {
        self.current.is_none()
    }

    pub fn kind(&self) -> Option<ModalKind> {
        self.kind
    }

    /// Borrow the active modal as a trait object, if any.
    pub fn as_ref(&self) -> Option<&dyn Pane> {
        self.current.as_deref()
    }

    /// Mutable access to the active modal (used by the responder chain
    /// to dispatch input events into it).
    pub fn as_mut(&mut self) -> Option<&mut Box<dyn Pane>> {
        self.current.as_mut()
    }

    /// Install `pane` of `kind` as the active modal. Returns the kind
    /// of the previous occupant if there was one (so the caller can
    /// emit a `ModalDismissed` for it before the corresponding
    /// `ModalOpened`).
    pub fn open(&mut self, pane: Box<dyn Pane>, kind: ModalKind) -> Option<ModalKind> {
        self.current = Some(pane);
        self.kind.replace(kind)
    }

    /// Drop the active modal, if any. Returns its kind so the caller
    /// can emit `ModalDismissed`.
    pub fn dismiss(&mut self) -> Option<ModalKind> {
        self.current = None;
        self.kind.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::commands::modal::PalettePane;
    use crate::editor::commands::CommandRegistry;

    #[test]
    fn open_replaces_previous_and_returns_old_kind() {
        let registry = CommandRegistry::default();
        let mut slot = ModalSlot::new();
        assert!(slot.is_none());
        let prev = slot.open(
            Box::new(PalettePane::from_registry(&registry)),
            ModalKind::Palette,
        );
        assert!(prev.is_none());
        assert!(slot.is_some());
        assert_eq!(slot.kind(), Some(ModalKind::Palette));

        let prev = slot.open(
            Box::new(PalettePane::from_registry(&registry)),
            ModalKind::Picker,
        );
        assert_eq!(prev, Some(ModalKind::Palette));
        assert_eq!(slot.kind(), Some(ModalKind::Picker));
    }

    #[test]
    fn dismiss_returns_kind_and_clears() {
        let registry = CommandRegistry::default();
        let mut slot = ModalSlot::new();
        slot.open(
            Box::new(PalettePane::from_registry(&registry)),
            ModalKind::Palette,
        );
        let kind = slot.dismiss();
        assert_eq!(kind, Some(ModalKind::Palette));
        assert!(slot.is_none());
        assert!(slot.dismiss().is_none(), "second dismiss is a no-op");
    }
}
