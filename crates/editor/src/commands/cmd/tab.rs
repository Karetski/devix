//! Tab management: new / next / prev / close.

use devix_panes::Action;

use crate::commands::context::Context;

pub struct NewTab;
impl<'a> Action<Context<'a>> for NewTab {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.editor.new_tab();
    }
}

pub struct NextTab;
impl<'a> Action<Context<'a>> for NextTab {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.editor.next_tab();
    }
}

pub struct PrevTab;
impl<'a> Action<Context<'a>> for PrevTab {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.editor.prev_tab();
    }
}

pub struct CloseTab;
impl<'a> Action<Context<'a>> for CloseTab {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.editor.close_active_tab(false);
    }
}

pub struct ForceCloseTab;
impl<'a> Action<Context<'a>> for ForceCloseTab {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.editor.close_active_tab(true);
    }
}
