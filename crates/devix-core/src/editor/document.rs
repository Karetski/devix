//! Document = Buffer + filesystem-watcher attachment + tree-sitter highlighter.
//! Owned by Editor's `DocStore`.
//!
//! All buffer mutations should go through `Document::apply_tx` /
//! `Document::undo` / `Document::redo` / `Document::reload_from_disk` rather
//! than reaching into `buffer` directly. Those methods keep the highlighter's
//! tree synchronized with the rope; bypassing them leaves stale highlight
//! spans pointing into the wrong byte ranges.
//!
//! `DocId` is a process-monotonic `u64` per `docs/specs/namespace.md`
//! § *Segment encoding rules → Resource ids*. Slot keys are no longer
//! exposed in paths; ids are minted from a global `AtomicU64` and are
//! stable across the session — `/buf/42` never names two different
//! buffers (a closed-and-reopened buffer mints a fresh id).

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use devix_protocol::path::Path as DevixPath;
use devix_protocol::Lookup;
use devix_text::{Buffer, Selection, Transaction};
use devix_syntax::{HighlightSpan, Highlighter, Language, input_edit_for_range};
use notify::{RecursiveMode, Watcher};

/// Process-monotonic `Document` id. Minted via `DocStore::insert`;
/// never reused across the session (per `namespace.md`'s stability
/// guarantee). Wire form on paths is `/buf/<id>`.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct DocId(u64);

static DOC_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

impl DocId {
    /// Mint a fresh id. Internal — `DocStore::insert` is the
    /// canonical entry point; tests use this directly.
    fn mint() -> Self {
        DocId(DOC_ID_COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Decode the inner u64 (for path encoding).
    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// Construct from a `Path`. Returns `None` if `path` isn't of
    /// the form `/buf/<u64>`.
    pub fn id_from_path(path: &DevixPath) -> Option<Self> {
        let mut segs = path.segments();
        if segs.next()? != "buf" {
            return None;
        }
        let id_seg = segs.next()?;
        if segs.next().is_some() {
            return None;
        }
        id_seg.parse::<u64>().ok().map(DocId)
    }

    /// Encode this id into its canonical path (`/buf/<id>`).
    pub fn to_path(self) -> DevixPath {
        DevixPath::parse(&format!("/buf/{}", self.0)).expect("/buf/<u64> is canonical")
    }
}

pub struct Document {
    pub buffer: Buffer,
    /// Active filesystem watcher. Installed by [`Document::install_disk_watcher`]
    /// after the document has a stable identity in its owning store, since
    /// the notify callback closes over a caller-supplied closure that
    /// typically wants the doc's id.
    pub watcher: Option<notify::RecommendedWatcher>,
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
            disk_changed_pending: false,
            highlighter,
        };
        d.full_reparse();
        d
    }

    /// Open `path`. Does *not* attach a filesystem watcher; the editor
    /// installs one via [`Document::install_disk_watcher`] after the doc
    /// is in the `DocStore` (so the watcher's callback can close over the
    /// stable `DocId`).
    pub fn from_path(path: PathBuf) -> Result<Self> {
        let buffer = Buffer::from_path(&path)?;
        let highlighter = Language::from_path(&path).and_then(|lang| Highlighter::new(lang).ok());
        let mut d = Self {
            buffer,
            watcher: None,
            disk_changed_pending: false,
            highlighter,
        };
        d.full_reparse();
        Ok(d)
    }

    pub fn empty() -> Self {
        Self::from_buffer(Buffer::empty())
    }

    /// Install a filesystem watcher whose `notify` callback invokes
    /// `on_change` on every detected change to this doc's path.
    /// Replaces any previously-installed watcher. Best-effort: if the
    /// watcher fails to spawn (read-only filesystem, permission error,
    /// path has no parent directory), the document is left without a
    /// watcher and `false` is returned.
    pub fn install_disk_watcher(
        &mut self,
        on_change: Box<dyn Fn() + Send + Sync + 'static>,
    ) -> bool {
        let Some(path) = self.buffer.path().map(Path::to_path_buf) else {
            return false;
        };
        match spawn_watcher_for(&path, on_change) {
            Ok(w) => {
                self.watcher = Some(w);
                true
            }
            Err(_) => false,
        }
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

/// Per-session document registry. Implements
/// `Lookup<Resource = Document>` mounted at `/buf/<id>` per
/// `docs/specs/namespace.md` § *Migration table*.
#[derive(Default)]
pub struct DocStore {
    docs: HashMap<DocId, Document>,
}

impl DocStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert `doc`, mint a fresh `DocId`, and return it.
    pub fn insert(&mut self, doc: Document) -> DocId {
        let id = DocId::mint();
        self.docs.insert(id, doc);
        id
    }

    /// Remove a document by id; returns it if present.
    pub fn remove(&mut self, id: DocId) -> Option<Document> {
        self.docs.remove(&id)
    }

    /// Look up a document by id (slotmap-shape compatibility).
    pub fn get(&self, id: DocId) -> Option<&Document> {
        self.docs.get(&id)
    }

    /// Look up a document by id mutably.
    pub fn get_mut(&mut self, id: DocId) -> Option<&mut Document> {
        self.docs.get_mut(&id)
    }

    /// Iterate `(id, &Document)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (DocId, &Document)> {
        self.docs.iter().map(|(id, d)| (*id, d))
    }

    /// Iterate `(id, &mut Document)` pairs.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (DocId, &mut Document)> {
        self.docs.iter_mut().map(|(id, d)| (*id, d))
    }

    /// Iterate documents (no ids).
    pub fn values(&self) -> impl Iterator<Item = &Document> {
        self.docs.values()
    }

    /// Iterate documents mutably (no ids).
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Document> {
        self.docs.values_mut()
    }

    /// Iterate ids only.
    pub fn keys(&self) -> impl Iterator<Item = DocId> + '_ {
        self.docs.keys().copied()
    }

    /// Whether `id` is in the store.
    pub fn contains_key(&self, id: DocId) -> bool {
        self.docs.contains_key(&id)
    }

    /// Number of live documents.
    pub fn len(&self) -> usize {
        self.docs.len()
    }

    /// True when no documents are live.
    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }
}

impl std::ops::Index<DocId> for DocStore {
    type Output = Document;
    fn index(&self, id: DocId) -> &Document {
        self.docs
            .get(&id)
            .unwrap_or_else(|| panic!("DocStore: unknown DocId {:?}", id))
    }
}

impl std::ops::IndexMut<DocId> for DocStore {
    fn index_mut(&mut self, id: DocId) -> &mut Document {
        self.docs
            .get_mut(&id)
            .unwrap_or_else(|| panic!("DocStore: unknown DocId {:?}", id))
    }
}

impl Lookup for DocStore {
    type Resource = Document;

    fn lookup(&self, path: &DevixPath) -> Option<&Document> {
        DocId::id_from_path(path).and_then(|id| self.get(id))
    }

    fn lookup_mut(&mut self, path: &DevixPath) -> Option<&mut Document> {
        DocId::id_from_path(path).and_then(|id| self.get_mut(id))
    }

    fn paths(&self) -> Box<dyn Iterator<Item = DevixPath> + '_> {
        Box::new(self.docs.keys().map(|id| id.to_path()))
    }
}

/// Watch `target_path`'s parent directory non-recursively, filtering
/// events to only fire when `target_path` is one of the changed paths,
/// and invoking `on_change` directly from the notify callback. The
/// returned watcher must be retained — dropping it stops the watch.
fn spawn_watcher_for(
    target_path: &Path,
    on_change: Box<dyn Fn() + Send + Sync + 'static>,
) -> Result<notify::RecommendedWatcher> {
    let target = target_path.to_path_buf();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(ev) = res else { return };
        use notify::EventKind::*;
        if !matches!(ev.kind, Modify(_) | Create(_) | Remove(_)) { return; }
        // Only signal if our target path is among the changed paths.
        // Without this filter, a watcher on a shared directory would
        // fire for every sibling file's change, producing spurious
        // "disk changed" prompts.
        if ev.paths.iter().any(|p| same_file(p, &target)) {
            on_change();
        }
    })?;
    let watch_target = target_path.parent().unwrap_or_else(|| Path::new("."));
    watcher.watch(watch_target, RecursiveMode::NonRecursive)?;
    Ok(watcher)
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

    #[test]
    fn doc_id_to_path_round_trips() {
        let id = DocId::mint();
        let path = id.to_path();
        assert!(path.as_str().starts_with("/buf/"));
        let back = DocId::id_from_path(&path).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn doc_id_from_path_rejects_other_roots() {
        let p = DevixPath::parse("/cur/3").unwrap();
        assert!(DocId::id_from_path(&p).is_none());
        let p = DevixPath::parse("/buf/abc").unwrap();
        assert!(DocId::id_from_path(&p).is_none());
        let p = DevixPath::parse("/buf").unwrap();
        assert!(DocId::id_from_path(&p).is_none());
        let p = DevixPath::parse("/buf/42/extra").unwrap();
        assert!(DocId::id_from_path(&p).is_none());
    }

    #[test]
    fn doc_store_implements_lookup_round_trip() {
        let mut store = DocStore::new();
        let id = store.insert(Document::empty());
        let path = id.to_path();
        assert!(store.lookup(&path).is_some());
        assert!(store.lookup_mut(&path).is_some());
        let paths: Vec<DevixPath> = store.paths().collect();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], path);
    }

    #[test]
    fn doc_store_close_and_reopen_mints_fresh_id() {
        let mut store = DocStore::new();
        let id_a = store.insert(Document::empty());
        store.remove(id_a).unwrap();
        let id_b = store.insert(Document::empty());
        // Process-monotonic counter — id_b is strictly greater.
        assert_ne!(id_a, id_b);
        assert!(id_b.as_u64() > id_a.as_u64());
        // Original id no longer resolves.
        assert!(store.get(id_a).is_none());
        // Reopened path is the new id's path, not the closed-buffer's.
        assert!(store.lookup(&id_a.to_path()).is_none());
        assert!(store.lookup(&id_b.to_path()).is_some());
    }
}
