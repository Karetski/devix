//! File / disk commands: save, reload, keep-buffer, open-path, quit.

use crate::Action;

use crate::editor::commands::context::Context;

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
        let Some((_, _, did)) = ctx.editor.active_ids() else { return };
        let d = match ctx.editor.documents.get_mut(did) {
            Some(d) => d,
            None => return,
        };
        if d.buffer.save().is_err() {
            return;
        }
        // F-5 follow-up 2026-05-12: announce the successful save so
        // subscribers (e.g., autoformatters, status-bar plugins)
        // can react. `BufferSaved.path` is the document path; the
        // disk path is included so subscribers don't need to
        // re-resolve it.
        let fs_path = d.buffer.path().map(|p| p.to_path_buf());
        if let Some(fs_path) = fs_path {
            ctx.editor
                .bus
                .publish(devix_protocol::pulse::Pulse::BufferSaved {
                    path: did.to_path(),
                    fs_path,
                });
        }
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
