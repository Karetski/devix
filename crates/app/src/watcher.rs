//! Filesystem watch + reconciliation.

use std::path::Path;
use std::sync::mpsc;

use anyhow::Result;
use notify::{RecursiveMode, Watcher};
use devix_workspace::Action;

use crate::app::App;
use crate::events::run_action;

pub fn spawn_watcher(path: &Path) -> Result<(notify::RecommendedWatcher, mpsc::Receiver<()>)> {
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            use notify::EventKind::*;
            if matches!(ev.kind, Modify(_) | Create(_) | Remove(_)) {
                let _ = tx.send(());
            }
        }
    })?;
    let watch_target = path.parent().unwrap_or_else(|| Path::new("."));
    watcher.watch(watch_target, RecursiveMode::NonRecursive)?;
    Ok((watcher, rx))
}

pub fn drain_disk_events(app: &mut App) {
    let Some(rx) = app.disk_rx.as_ref() else { return };
    let mut got = false;
    while rx.try_recv().is_ok() {
        got = true;
    }
    if !got {
        return;
    }
    if app.editor.buffer.dirty() {
        app.disk_changed_pending = true;
        app.status
            .set("Disk changed (buffer modified) · Ctrl+R reload, Ctrl+K keep");
    } else {
        run_action(app, Action::ReloadFromDisk);
    }
}
