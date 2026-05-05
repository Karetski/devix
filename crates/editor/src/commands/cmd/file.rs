//! File / disk commands: save, reload, keep-buffer, open-path, quit.

use devix_panes::Action;

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
        let Some(d) = ctx.editor.active_doc_mut() else { return };
        let _ = d.buffer.save();
    }
}

pub struct KeepBufferIgnoreDisk;
impl<'a> Action<Context<'a>> for KeepBufferIgnoreDisk {
    fn invoke(&self, ctx: &mut Context<'a>) {
        if let Some(d) = ctx.editor.active_doc_mut() {
            d.disk_changed_pending = false;
        }
    }
}

pub struct OpenPath(pub std::path::PathBuf);
impl<'a> Action<Context<'a>> for OpenPath {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let _ = ctx.editor.open_path_replace_current(self.0.clone());
    }
}

pub struct ReloadFromDisk;
impl<'a> Action<Context<'a>> for ReloadFromDisk {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, cid, did)) = ctx.editor.active_ids() else { return };
        let res = ctx.editor.documents[did].reload_from_disk();
        if res.is_ok() {
            let max = ctx.editor.documents[did].buffer.len_chars();
            ctx.editor.documents[did].disk_changed_pending = false;
            ctx.editor.cursors[cid].selection.clamp(max);
        }
    }
}
