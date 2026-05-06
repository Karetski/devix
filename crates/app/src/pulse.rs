//! `Pulse` — typed message from a service to the runtime.
//!
//! Each pulse type knows how to deliver itself with `&mut AppContext`.
//! New subsystem-to-runtime messages add a new struct + `impl Pulse`;
//! there is no central enum that grows.

use devix_editor::{DocId, cmd, frame_ids};
use devix_plugin::PluginMsg;

use crate::context::AppContext;

pub trait Pulse: Send + 'static {
    fn deliver(self: Box<Self>, ctx: &mut AppContext<'_>);
    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// Disk watcher reported a change for a document. Three-way handling:
/// dirty buffer → mark pending and prompt; active+clean → reload via the
/// command path; background+clean → silent reload + cursor clamp.
pub struct DiskChanged {
    pub doc: DocId,
}

impl Pulse for DiskChanged {
    fn deliver(self: Box<Self>, ctx: &mut AppContext<'_>) {
        let doc_id = self.doc;
        let active_doc_id = ctx.editor.active_cursor().map(|c| c.doc);
        let dirty = ctx
            .editor
            .documents
            .get(doc_id)
            .map(|d| d.buffer.dirty())
            .unwrap_or(false);

        if dirty {
            if let Some(d) = ctx.editor.documents.get_mut(doc_id) {
                d.disk_changed_pending = true;
            }
            ctx.request_redraw();
        } else if Some(doc_id) == active_doc_id {
            ctx.run(&cmd::ReloadFromDisk);
        } else if let Some(d) = ctx.editor.documents.get_mut(doc_id) {
            if d.reload_from_disk().is_ok() {
                let max = ctx.editor.documents[doc_id].buffer.len_chars();
                for cursor in ctx.editor.cursors.values_mut() {
                    if cursor.doc == doc_id {
                        cursor.selection.clamp(max);
                    }
                }
            }
            ctx.request_redraw();
        }
    }
}

/// Plugin host pushed a message. Mirrors today's `drain_plugin_events`.
pub struct PluginEmitted {
    pub msg: PluginMsg,
}

impl Pulse for PluginEmitted {
    fn deliver(self: Box<Self>, ctx: &mut AppContext<'_>) {
        match self.msg {
            PluginMsg::Status(_) => {}
            PluginMsg::PaneChanged => ctx.request_redraw(),
            PluginMsg::OpenPath(path) => {
                if ctx.editor.active_frame().is_none() {
                    if let Some(fid) = frame_ids(ctx.editor.root.as_ref()).first().copied() {
                        ctx.editor.focus_frame(fid);
                    }
                }
                ctx.run(&cmd::OpenPath(path));
            }
        }
    }
}

/// Mouse-wheel accumulator: the previous design coalesced rapid scroll
/// events into a single `pending_scroll: isize`. With an Effect queue we
/// still want to coalesce — many tiny scroll deltas → one `ScrollBy` —
/// but the queue itself does the bookkeeping.
pub struct ScrollAccumulated {
    pub delta: isize,
}

impl Pulse for ScrollAccumulated {
    fn deliver(self: Box<Self>, ctx: &mut AppContext<'_>) {
        ctx.run(&cmd::ScrollBy(self.delta));
    }
}
