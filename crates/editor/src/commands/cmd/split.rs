//! Split / focus / sidebar commands.
//!
//! `SplitVertical` / `SplitHorizontal` are named for the *dividing line* the
//! user expects to see; the editor's `Axis` describes the layout direction
//! children are arranged in. So a "vertical split" produces children laid
//! out horizontally — flip in the impls.

use devix_core::Action;
use devix_core::layout::{Axis, Direction, SidebarSlot};

use crate::commands::context::Context;

pub struct SplitVertical;
impl<'a> Action<Context<'a>> for SplitVertical {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.editor.split_active(Axis::Horizontal);
    }
}

pub struct SplitHorizontal;
impl<'a> Action<Context<'a>> for SplitHorizontal {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.editor.split_active(Axis::Vertical);
    }
}

pub struct CloseFrame;
impl<'a> Action<Context<'a>> for CloseFrame {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.editor.close_active_frame();
    }
}

pub struct ToggleSidebar(pub SidebarSlot);
impl<'a> Action<Context<'a>> for ToggleSidebar {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.editor.toggle_sidebar(self.0);
    }
}

pub struct FocusDir(pub Direction);
impl<'a> Action<Context<'a>> for FocusDir {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.editor.focus_dir(self.0);
    }
}
