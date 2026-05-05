//! Filesystem-watch reconciliation. Watchers are owned by `Document`; this
//! module drains their receivers each tick and sets `disk_changed_pending`
//! on the affected document.

use std::sync::Arc;

use devix_surface::cmd;

use crate::app::App;
use crate::events::run_command;

pub fn drain_disk_events(app: &mut App) {
    // Phase 1: collect which DocIds saw events. Holding `&mut documents`
    // across the iteration would conflict with the active-doc reads/writes
    // below, so the drain is split into a read pass + a write pass.
    let mut affected: Vec<devix_surface::DocId> = Vec::new();
    for (id, doc) in app.surface.documents.iter() {
        let Some(rx) = doc.disk_rx.as_ref() else { continue };
        let mut got = false;
        while rx.try_recv().is_ok() { got = true; }
        if got { affected.push(id); }
    }
    if affected.is_empty() { return; }

    // Determine which (if any) of the affected docs is currently active so we
    // can produce a status-bar prompt for it. Background docs accumulate the
    // pending flag silently.
    let active_doc_id = app
        .surface
        .active_view()
        .map(|v| v.doc);

    for did in &affected {
        let doc = &mut app.surface.documents[*did];
        if doc.buffer.dirty() {
            doc.disk_changed_pending = true;
        } else if Some(*did) == active_doc_id {
            // Active, clean: auto-reload via the standard action flow.
            run_command(app, Arc::new(cmd::ReloadFromDisk));
        } else {
            // Background, clean: silently reload now. Routes through
            // Document so tree-sitter reparses; bypassing this leaves
            // stale highlight spans.
            if app.surface.documents[*did].reload_from_disk().is_ok() {
                let max = app.surface.documents[*did].buffer.len_chars();
                for view in app.surface.views.values_mut() {
                    if view.doc == *did {
                        view.selection.clamp(max);
                    }
                }
            }
        }
    }

    app.request_redraw();
}
