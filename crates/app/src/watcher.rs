//! Filesystem-watch reconciliation. Watchers are owned by `Document`; this
//! module drains their receivers each tick and sets `disk_changed_pending`
//! on the affected document.

use devix_workspace::Action;

use crate::app::App;
use crate::events::run_action;

pub fn drain_disk_events(app: &mut App) {
    // Phase 1: collect which DocIds saw events. Holding `&mut documents`
    // across the iteration would conflict with the active-doc reads/writes
    // below, so the drain is split into a read pass + a write pass.
    let mut affected: Vec<devix_workspace::DocId> = Vec::new();
    for (id, doc) in app.workspace.documents.iter() {
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
        .workspace
        .active_view()
        .map(|v| v.doc);

    let mut active_dirty = false;
    for did in &affected {
        let doc = &mut app.workspace.documents[*did];
        if doc.buffer.dirty() {
            doc.disk_changed_pending = true;
            if Some(*did) == active_doc_id { active_dirty = true; }
        } else if Some(*did) == active_doc_id {
            // Active, clean: auto-reload via the standard action flow.
            run_action(app, Action::ReloadFromDisk);
        } else {
            // Background, clean: silently reload now (no prompt to show).
            let res = app.workspace.documents[*did].buffer.reload_from_disk();
            if let Ok(()) = res { /* ok */ }
            // On error, leave the buffer alone; nothing to surface.
        }
    }

    if active_dirty {
        app.status.set("Disk changed (buffer modified) · Ctrl+R reload, Ctrl+K keep");
    }
    app.dirty = true;
}
