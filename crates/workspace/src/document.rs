//! Document = Buffer + filesystem-watcher attachment, owned by Workspace.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use anyhow::Result;
use devix_buffer::Buffer;
use notify::{RecursiveMode, Watcher};
use slotmap::new_key_type;

new_key_type! { pub struct DocId; }

pub struct Document {
    pub buffer: Buffer,
    pub watcher: Option<notify::RecommendedWatcher>,
    /// Receives a `()` whenever the watcher detects a change to this doc's path.
    /// Drained on every event-loop tick by `App::drain_disk_events`, which sets
    /// `disk_changed_pending` on the affected Document.
    pub disk_rx: Option<mpsc::Receiver<()>>,
    pub disk_changed_pending: bool,
}

impl Document {
    pub fn from_buffer(buffer: Buffer) -> Self {
        Self {
            buffer,
            watcher: None,
            disk_rx: None,
            disk_changed_pending: false,
        }
    }

    /// Open `path`. Best-effort spawns a filesystem watcher for that path; if
    /// spawning fails (e.g. read-only filesystem, permission error), the
    /// document is still returned without a watcher rather than failing the
    /// open.
    pub fn from_path(path: PathBuf) -> Result<Self> {
        let buffer = Buffer::from_path(&path)?;
        let (watcher, disk_rx) = match spawn_watcher_for(&path) {
            Ok((w, rx)) => (Some(w), Some(rx)),
            Err(_) => (None, None),
        };
        Ok(Self {
            buffer,
            watcher,
            disk_rx,
            disk_changed_pending: false,
        })
    }

    pub fn empty() -> Self {
        Self::from_buffer(Buffer::empty())
    }
}

/// Watch `target_path`'s parent directory non-recursively, filtering events to
/// only fire when `target_path` is one of the changed paths. The watcher must
/// be retained (returned to the caller) — dropping it stops the watch.
fn spawn_watcher_for(
    target_path: &Path,
) -> Result<(notify::RecommendedWatcher, mpsc::Receiver<()>)> {
    let (tx, rx) = mpsc::channel::<()>();
    let target = target_path.to_path_buf();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(ev) = res else { return };
        use notify::EventKind::*;
        if !matches!(ev.kind, Modify(_) | Create(_) | Remove(_)) { return; }
        // Only signal if our target path is among the changed paths. Without
        // this filter, a watcher on a shared directory would fire for every
        // sibling file's change, producing spurious "disk changed" prompts.
        if ev.paths.iter().any(|p| same_file(p, &target)) {
            let _ = tx.send(());
        }
    })?;
    let watch_target = target_path.parent().unwrap_or_else(|| Path::new("."));
    watcher.watch(watch_target, RecursiveMode::NonRecursive)?;
    Ok((watcher, rx))
}

/// Best-effort path-equality check. Both sides may or may not be canonical;
/// fall back to lexical equality if canonicalization fails.
fn same_file(a: &Path, b: &Path) -> bool {
    let ca = std::fs::canonicalize(a).ok();
    let cb = std::fs::canonicalize(b).ok();
    match (ca, cb) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_document_has_no_path_and_no_watcher() {
        let d = Document::empty();
        assert!(d.buffer.path().is_none());
        assert!(d.watcher.is_none());
        assert!(d.disk_rx.is_none());
        assert!(!d.disk_changed_pending);
        assert!(!d.buffer.dirty());
    }
}
