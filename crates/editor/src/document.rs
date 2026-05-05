//! Document = Buffer + filesystem-watcher attachment + tree-sitter highlighter.
//! Owned by Surface.
//!
//! All buffer mutations should go through `Document::apply_tx` /
//! `Document::undo` / `Document::redo` / `Document::reload_from_disk` rather
//! than reaching into `buffer` directly. Those methods keep the highlighter's
//! tree synchronized with the rope; bypassing them leaves stale highlight
//! spans pointing into the wrong byte ranges.

use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;

use anyhow::Result;
use devix_text::{Buffer, Selection, Transaction};
use devix_syntax::{HighlightSpan, Highlighter, Language, input_edit_for_range};
use notify::{RecursiveMode, Watcher};
use slotmap::new_key_type;

new_key_type! { pub struct DocId; }

pub struct Document {
    pub buffer: Buffer,
    pub watcher: Option<notify::RecommendedWatcher>,
    /// Receives a `()` whenever the watcher detects a change to this doc's path.
    /// Drained on every event-loop tick by `App::drain_disk_events`, which sets
    /// `disk_changed_pending` on the affected Document.
    pub disk_rx: Option<std_mpsc::Receiver<()>>,
    pub disk_changed_pending: bool,
    /// Per-document syntax tree. `None` for plain-text or unknown extensions;
    /// then `highlights` returns an empty vec and the editor renders without
    /// scope styling.
    highlighter: Option<Highlighter>,
}

impl Document {
    pub fn from_buffer(buffer: Buffer) -> Self {
        let highlighter = buffer
            .path()
            .and_then(Language::from_path)
            .and_then(|lang| Highlighter::new(lang).ok());
        let mut d = Self {
            buffer,
            watcher: None,
            disk_rx: None,
            disk_changed_pending: false,
            highlighter,
        };
        d.full_reparse();
        d
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
        let highlighter = Language::from_path(&path).and_then(|lang| Highlighter::new(lang).ok());
        let mut d = Self {
            buffer,
            watcher,
            disk_rx,
            disk_changed_pending: false,
            highlighter,
        };
        d.full_reparse();
        Ok(d)
    }

    pub fn empty() -> Self {
        Self::from_buffer(Buffer::empty())
    }

    /// Apply a buffer transaction and keep the highlighter in sync.
    /// Single-change transactions take the cheap incremental path;
    /// multi-change (multicursor) falls back to a full reparse.
    pub fn apply_tx(&mut self, tx: Transaction) {
        let single_change = tx.changes.len() == 1;
        let edit_data = single_change.then(|| {
            let ch = &tx.changes[0];
            (ch.start, ch.start + ch.remove_len, ch.start + ch.insert.chars().count())
        });
        let before_rope = edit_data.is_some().then(|| self.buffer.rope().clone());

        self.buffer.apply(tx);

        let Some(h) = self.highlighter.as_mut() else { return };
        match (edit_data, before_rope) {
            (Some((s, oe, ne)), Some(before)) => {
                let edit = input_edit_for_range(&before, self.buffer.rope(), s, oe, ne);
                h.edit(&edit);
                h.parse(self.buffer.rope());
            }
            _ => {
                h.invalidate();
                h.parse(self.buffer.rope());
            }
        }
    }

    pub fn undo(&mut self) -> Option<Selection> {
        let sel = self.buffer.undo()?;
        self.full_reparse();
        Some(sel)
    }

    pub fn redo(&mut self) -> Option<Selection> {
        let sel = self.buffer.redo()?;
        self.full_reparse();
        Some(sel)
    }

    pub fn reload_from_disk(&mut self) -> Result<()> {
        self.buffer.reload_from_disk()?;
        self.full_reparse();
        Ok(())
    }

    pub fn highlights(&self, start_byte: usize, end_byte: usize) -> Vec<HighlightSpan> {
        self.highlighter
            .as_ref()
            .map(|h| h.highlights(self.buffer.rope(), start_byte, end_byte))
            .unwrap_or_default()
    }

    pub fn language(&self) -> Option<Language> {
        self.highlighter.as_ref().map(|h| h.language())
    }

    /// Drop and rebuild the syntax tree from current buffer contents. Called
    /// after non-incremental changes (undo/redo, disk reload).
    fn full_reparse(&mut self) {
        if let Some(h) = self.highlighter.as_mut() {
            h.invalidate();
            h.parse(self.buffer.rope());
        }
    }
}

/// Watch `target_path`'s parent directory non-recursively, filtering events to
/// only fire when `target_path` is one of the changed paths. The watcher must
/// be retained (returned to the caller) — dropping it stops the watch.
fn spawn_watcher_for(
    target_path: &Path,
) -> Result<(notify::RecommendedWatcher, std_mpsc::Receiver<()>)> {
    let (tx, rx) = std_mpsc::channel::<()>();
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
    use devix_text::{Selection, replace_selection_tx};

    #[test]
    fn empty_document_has_no_path_no_watcher_no_highlighter() {
        let d = Document::empty();
        assert!(d.buffer.path().is_none());
        assert!(d.watcher.is_none());
        assert!(d.disk_rx.is_none());
        assert!(!d.disk_changed_pending);
        assert!(!d.buffer.dirty());
        assert!(d.language().is_none());
    }

    #[test]
    fn rust_path_attaches_highlighter() {
        let dir = std::env::temp_dir().join(format!("devix-doc-rs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.rs");
        std::fs::write(&p, "fn main() {}").unwrap();
        let d = Document::from_path(p).unwrap();
        assert_eq!(d.language(), Some(Language::Rust));
        let spans = d.highlights(0, d.buffer.rope().len_bytes());
        assert!(!spans.is_empty(), "rust source should produce highlights");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_extension_has_no_highlighter() {
        let dir = std::env::temp_dir().join(format!("devix-doc-txt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.txt");
        std::fs::write(&p, "hello").unwrap();
        let d = Document::from_path(p).unwrap();
        assert!(d.language().is_none());
        assert!(d.highlights(0, 5).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_tx_updates_highlights() {
        let dir = std::env::temp_dir().join(format!("devix-doc-edit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.rs");
        std::fs::write(&p, "fn a() {}").unwrap();
        let mut d = Document::from_path(p).unwrap();
        let before = d.highlights(0, d.buffer.rope().len_bytes());
        let len = d.buffer.len_chars();
        let tx = replace_selection_tx(&d.buffer, &Selection::point(len), "\nfn b() {}");
        d.apply_tx(tx);
        let after = d.highlights(0, d.buffer.rope().len_bytes());
        assert!(
            after.len() > before.len(),
            "adding a second fn should produce more spans (was {}, now {})",
            before.len(),
            after.len(),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
