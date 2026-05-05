//! File / disk commands: save, reload, keep-buffer, open-path, quit.

use devix_core::Action;

use crate::commands::context::Context;

/// Quit the editor. The simplest possible action: flips the run flag.
pub struct Quit;
impl<'a> Action<Context<'a>> for Quit {
    fn invoke(&self, ctx: &mut Context<'a>) {
        *ctx.quit = true;
    }
}

pub struct Save;
impl<'a> Action<Context<'a>> for Save {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some(d) = ctx.surface.active_doc_mut() else { return };
        let _ = d.buffer.save();
    }
}

pub struct KeepBufferIgnoreDisk;
impl<'a> Action<Context<'a>> for KeepBufferIgnoreDisk {
    fn invoke(&self, ctx: &mut Context<'a>) {
        if let Some(d) = ctx.surface.active_doc_mut() {
            d.disk_changed_pending = false;
        }
    }
}

pub struct OpenPath(pub std::path::PathBuf);
impl<'a> Action<Context<'a>> for OpenPath {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let _ = ctx.surface.open_path_replace_current(self.0.clone());
    }
}

pub struct ReloadFromDisk;
impl<'a> Action<Context<'a>> for ReloadFromDisk {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, vid, did)) = ctx.surface.active_ids() else { return };
        let res = ctx.surface.documents[did].reload_from_disk();
        if res.is_ok() {
            let max = ctx.surface.documents[did].buffer.len_chars();
            ctx.surface.documents[did].disk_changed_pending = false;
            ctx.surface.views[vid].selection.clamp(max);
        }
    }
}
