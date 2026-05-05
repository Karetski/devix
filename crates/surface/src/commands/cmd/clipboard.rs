//! Clipboard commands: copy / cut / paste.

use devix_core::Action;

use crate::commands::context::Context;

pub struct Copy;
impl<'a> Action<Context<'a>> for Copy {
    fn invoke(&self, ctx: &mut Context<'a>) {
        crate::commands::dispatch::do_copy(ctx);
    }
}

pub struct Cut;
impl<'a> Action<Context<'a>> for Cut {
    fn invoke(&self, ctx: &mut Context<'a>) {
        crate::commands::dispatch::do_cut(ctx);
    }
}

pub struct Paste;
impl<'a> Action<Context<'a>> for Paste {
    fn invoke(&self, ctx: &mut Context<'a>) {
        crate::commands::dispatch::do_paste(ctx);
    }
}
