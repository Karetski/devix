//! Filesystem watch. `drain_disk_events` is added in sub-task 8f after `app`
//! and `events` modules exist.

use std::path::Path;
use std::sync::mpsc;

use anyhow::Result;
use notify::{RecursiveMode, Watcher};

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
