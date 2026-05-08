//! Command palette and modal-management commands.

use devix_protocol::pulse::ModalKind;

use crate::Action;

use crate::editor::commands::context::Context;
use crate::editor::commands::modal::PalettePane;

pub struct OpenPalette;
impl<'a> Action<Context<'a>> for OpenPalette {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let pane = Box::new(PalettePane::from_registry(ctx.commands));
        ctx.editor.open_modal(pane, ModalKind::Palette);
    }
}

pub struct ClosePalette;
impl<'a> Action<Context<'a>> for ClosePalette {
    fn invoke(&self, ctx: &mut Context<'a>) {
        if super::modal_is::<PalettePane>(ctx) {
            ctx.editor.dismiss_modal();
        }
    }
}

/// Generic "close whatever modal is open" action used by the responder
/// chain after a modal pane signals `ModalOutcome::Dismiss`.
pub struct CloseModal;
impl<'a> Action<Context<'a>> for CloseModal {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.editor.dismiss_modal();
    }
}

pub struct PaletteMove(pub isize);
impl<'a> Action<Context<'a>> for PaletteMove {
    fn invoke(&self, ctx: &mut Context<'a>) {
        if let Some(p) = super::downcast_modal_mut::<PalettePane>(ctx) {
            p.state.move_selection(self.0);
        }
    }
}

pub struct PaletteSetQuery(pub String);
impl<'a> Action<Context<'a>> for PaletteSetQuery {
    fn invoke(&self, ctx: &mut Context<'a>) {
        if let Some(p) = super::downcast_modal_mut::<PalettePane>(ctx) {
            p.state.set_query(self.0.clone());
        }
    }
}

pub struct PaletteAccept;
impl<'a> Action<Context<'a>> for PaletteAccept {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let chosen = ctx
            .editor
            .modal
            .as_ref()
            .and_then(|m| m.as_any())
            .and_then(|a| a.downcast_ref::<PalettePane>())
            .and_then(|p| {
                p.state
                    .selected_command_id()
                    .and_then(|id| ctx.commands.resolve(id))
            });
        ctx.editor.dismiss_modal();
        if let Some(action) = chosen {
            action.invoke(ctx);
        }
    }
}
