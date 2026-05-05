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
use devix_lsp::{LspCommand, Edit as LspEdit, char_in_rope, path_to_uri, translate_changes};
use devix_syntax::{HighlightSpan, Highlighter, Language, input_edit_for_range};
use lsp_types::{
    Diagnostic, DiagnosticSeverity, PositionEncodingKind, TextDocumentContentChangeEvent, Uri,
};
use notify::{RecursiveMode, Watcher};
use slotmap::new_key_type;
use tokio::sync::mpsc as tokio_mpsc;

new_key_type! { pub struct DocId; }

/// LSP diagnostic, normalized to char positions in the current rope so the
/// renderer doesn't need to know the negotiated encoding.
#[derive(Clone, Debug)]
pub struct DocDiagnostic {
    pub start_line: usize,
    pub start_char_in_line: usize,
    pub end_line: usize,
    pub end_char_in_line: usize,
    pub severity: DiagnosticSeverity,
    pub message: String,
}

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
    /// LSP `textDocument` version. Bumped on every mutation that's reported
    /// to the server (apply_tx / undo / redo / reload). Stays at zero while
    /// the document is unattached.
    lsp_version: i32,
    /// Outbound channel into the LSP coordinator. `Some` once
    /// `attach_lsp` has fired the `DocOpen` notification; `None` otherwise.
    /// Closing the channel (coordinator gone) is treated as detach — sends
    /// fall through silently rather than aborting the editor.
    lsp_sink: Option<tokio_mpsc::UnboundedSender<LspCommand>>,
    lsp_uri: Option<Uri>,
    /// Encoding negotiated with the server for this document's client. Only
    /// consulted while attached; defaults to utf-16 (the LSP fallback) when
    /// not set, matching what `Coordinator` reports if utf-8 negotiation
    /// fails.
    lsp_encoding: PositionEncodingKind,
    diagnostics: Vec<DocDiagnostic>,
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
            lsp_version: 0,
            lsp_sink: None,
            lsp_uri: None,
            lsp_encoding: PositionEncodingKind::UTF16,
            diagnostics: Vec::new(),
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
            lsp_version: 0,
            lsp_sink: None,
            lsp_uri: None,
            lsp_encoding: PositionEncodingKind::UTF16,
            diagnostics: Vec::new(),
        };
        d.full_reparse();
        Ok(d)
    }

    pub fn empty() -> Self {
        Self::from_buffer(Buffer::empty())
    }

    /// Apply a buffer transaction and keep the highlighter + LSP server in
    /// sync. Single-change transactions take the cheap incremental path on
    /// both sides; multi-change (only emitted when multicursor lands) falls
    /// back to a full reparse and a full-text resync.
    pub fn apply_tx(&mut self, tx: Transaction) {
        let single_change = tx.changes.len() == 1;
        let edit_data = single_change.then(|| {
            let ch = &tx.changes[0];
            (ch.start, ch.start + ch.remove_len, ch.start + ch.insert.chars().count())
        });
        // Pre-rope is needed by syntax (single-change incremental edit) and
        // by LSP (translating change ranges from pre-edit positions). One
        // clone covers both consumers; clones are O(1) on ropey.
        let needs_pre_rope = edit_data.is_some() || (self.lsp_attached() && single_change);
        let before_rope = needs_pre_rope.then(|| self.buffer.rope().clone());

        let lsp_events = if self.lsp_attached() && single_change {
            let pre = before_rope.as_ref().expect("pre-rope cloned for single-change");
            let edits: Vec<LspEdit<'_>> = tx
                .changes
                .iter()
                .map(|c| LspEdit {
                    start_char: c.start,
                    end_char: c.start + c.remove_len,
                    text: c.insert.as_str(),
                })
                .collect();
            Some(translate_changes(pre, &edits, &self.lsp_encoding))
        } else {
            None
        };
        let needs_full_resync = self.lsp_attached() && !single_change;

        self.buffer.apply(tx);

        if let Some(events) = lsp_events {
            self.send_lsp_change(events);
        } else if needs_full_resync {
            self.send_lsp_full_resync();
        }

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
        self.send_lsp_full_resync();
        Some(sel)
    }

    pub fn redo(&mut self) -> Option<Selection> {
        let sel = self.buffer.redo()?;
        self.full_reparse();
        self.send_lsp_full_resync();
        Some(sel)
    }

    pub fn reload_from_disk(&mut self) -> Result<()> {
        self.buffer.reload_from_disk()?;
        self.full_reparse();
        self.send_lsp_full_resync();
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

    pub fn lsp_attached(&self) -> bool {
        self.lsp_sink.is_some()
    }

    pub fn lsp_uri(&self) -> Option<&Uri> {
        self.lsp_uri.as_ref()
    }

    /// Attach this document to the LSP coordinator. No-op if the document
    /// has no path or no recognized language. Sends `LspCommand::Open` with
    /// the current full text on success.
    pub fn attach_lsp(
        &mut self,
        sink: tokio_mpsc::UnboundedSender<LspCommand>,
        encoding: PositionEncodingKind,
    ) {
        if self.lsp_attached() {
            return;
        }
        let Some(path) = self.buffer.path() else { return };
        let Some(lang) = self.language() else { return };
        let Ok(uri) = path_to_uri(path) else { return };
        let text = self.buffer.rope().to_string();
        // Reset the version on attach so the server starts at 1 with the
        // first didOpen, matching the LSP spec's monotonic-version contract
        // for a freshly opened document.
        self.lsp_version = 1;
        let send = sink.send(LspCommand::Open {
            uri: uri.clone(),
            language_id: lang.lsp_id().to_string(),
            version: self.lsp_version,
            text,
            root_hint: None,
        });
        if send.is_err() {
            return;
        }
        self.lsp_sink = Some(sink);
        self.lsp_uri = Some(uri);
        self.lsp_encoding = encoding;
    }

    /// Send `LspCommand::Close` and clear the sink. Called from
    /// `Surface::try_remove_orphan_doc` before the Document is dropped.
    pub fn detach_lsp(&mut self) {
        let Some(sink) = self.lsp_sink.take() else { return };
        let Some(uri) = self.lsp_uri.take() else { return };
        let _ = sink.send(LspCommand::Close { uri });
    }

    pub fn diagnostics(&self) -> &[DocDiagnostic] {
        &self.diagnostics
    }

    /// Replace the document's diagnostic list. LSP positions are converted
    /// to char-in-line offsets using the negotiated encoding so the
    /// renderer can paint them directly. Diagnostics whose positions don't
    /// resolve in the current rope are dropped — they're advisory anyway,
    /// and the server will republish on the next keystroke pause.
    pub fn set_diagnostics(&mut self, raw: Vec<Diagnostic>) {
        let rope = self.buffer.rope();
        let enc = &self.lsp_encoding;
        let mut out = Vec::with_capacity(raw.len());
        for d in raw {
            let Some(s_abs) = char_in_rope(rope, d.range.start.line, d.range.start.character, enc) else { continue };
            let Some(e_abs) = char_in_rope(rope, d.range.end.line, d.range.end.character, enc) else { continue };
            let sl = rope.char_to_line(s_abs);
            let el = rope.char_to_line(e_abs);
            let sc = s_abs - rope.line_to_char(sl);
            let ec = e_abs - rope.line_to_char(el);
            out.push(DocDiagnostic {
                start_line: sl,
                start_char_in_line: sc,
                end_line: el,
                end_char_in_line: ec,
                severity: d.severity.unwrap_or(DiagnosticSeverity::INFORMATION),
                message: d.message,
            });
        }
        self.diagnostics = out;
    }

    fn send_lsp_change(&mut self, content_changes: Vec<TextDocumentContentChangeEvent>) {
        let Some(sink) = self.lsp_sink.as_ref() else { return };
        let Some(uri) = self.lsp_uri.clone() else { return };
        self.lsp_version += 1;
        let _ = sink.send(LspCommand::Change {
            uri,
            version: self.lsp_version,
            changes: content_changes,
        });
    }

    fn send_lsp_full_resync(&mut self) {
        let Some(sink) = self.lsp_sink.as_ref() else { return };
        let Some(uri) = self.lsp_uri.clone() else { return };
        self.lsp_version += 1;
        let _ = sink.send(LspCommand::Change {
            uri,
            version: self.lsp_version,
            changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: self.buffer.rope().to_string(),
            }],
        });
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

    fn rust_doc(file_name: &str, contents: &str) -> (Document, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "devix-doc-lsp-{}-{}",
            std::process::id(),
            file_name
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(file_name);
        std::fs::write(&p, contents).unwrap();
        (Document::from_path(p.clone()).unwrap(), dir)
    }

    fn drain<T>(rx: &mut tokio_mpsc::UnboundedReceiver<T>) -> Vec<T> {
        let mut out = Vec::new();
        while let Ok(v) = rx.try_recv() { out.push(v); }
        out
    }

    #[test]
    fn attach_lsp_sends_open_with_full_text() {
        let (mut d, dir) = rust_doc("a.rs", "fn main() {}");
        let (tx, mut rx) = tokio_mpsc::unbounded_channel();
        d.attach_lsp(tx, PositionEncodingKind::UTF8);
        assert!(d.lsp_attached());
        let msgs = drain(&mut rx);
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            LspCommand::Open { language_id, version, text, .. } => {
                assert_eq!(language_id, "rust");
                assert_eq!(*version, 1);
                assert_eq!(text, "fn main() {}");
            }
            other => panic!("expected Open, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn attach_lsp_skips_doc_with_no_language() {
        let dir = std::env::temp_dir().join(format!("devix-lsp-skip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.txt");
        std::fs::write(&p, "hi").unwrap();
        let mut d = Document::from_path(p).unwrap();
        let (tx, mut rx) = tokio_mpsc::unbounded_channel();
        d.attach_lsp(tx, PositionEncodingKind::UTF8);
        assert!(!d.lsp_attached());
        assert!(drain(&mut rx).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_tx_after_attach_sends_incremental_didchange() {
        let (mut d, dir) = rust_doc("b.rs", "fn a() {}");
        let (tx, mut rx) = tokio_mpsc::unbounded_channel();
        d.attach_lsp(tx, PositionEncodingKind::UTF8);
        let _open = drain(&mut rx);

        let len = d.buffer.len_chars();
        let edit = replace_selection_tx(&d.buffer, &Selection::point(len), "X");
        d.apply_tx(edit);

        let msgs = drain(&mut rx);
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            LspCommand::Change { version, changes, .. } => {
                assert_eq!(*version, 2);
                assert_eq!(changes.len(), 1);
                let ev = &changes[0];
                assert_eq!(ev.text, "X");
                let r = ev.range.unwrap();
                assert_eq!(r.start.line, 0);
                assert_eq!(r.end.line, 0);
                // utf-8 character is the byte offset within the line — for ascii
                // "fn a() {}" that's 9.
                assert_eq!(r.start.character, 9);
                assert_eq!(r.end.character, 9);
            }
            other => panic!("expected Change, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_redo_send_full_text_resync() {
        let (mut d, dir) = rust_doc("c.rs", "fn x() {}");
        let (tx, mut rx) = tokio_mpsc::unbounded_channel();
        d.attach_lsp(tx, PositionEncodingKind::UTF8);
        let _open = drain(&mut rx);

        let len = d.buffer.len_chars();
        let edit = replace_selection_tx(&d.buffer, &Selection::point(len), "Y");
        d.apply_tx(edit);
        let _change = drain(&mut rx);

        d.undo().unwrap();
        let msgs = drain(&mut rx);
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            LspCommand::Change { version, changes, .. } => {
                assert_eq!(*version, 3);
                assert_eq!(changes.len(), 1);
                assert!(changes[0].range.is_none(), "full-text resync has no range");
                assert_eq!(changes[0].text, "fn x() {}");
            }
            other => panic!("expected Change, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detach_lsp_sends_close_and_clears_state() {
        let (mut d, dir) = rust_doc("d.rs", "fn d() {}");
        let (tx, mut rx) = tokio_mpsc::unbounded_channel();
        d.attach_lsp(tx, PositionEncodingKind::UTF8);
        let uri_at_attach = d.lsp_uri().cloned();
        let _open = drain(&mut rx);

        d.detach_lsp();
        assert!(!d.lsp_attached());
        let msgs = drain(&mut rx);
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            LspCommand::Close { uri } => {
                assert_eq!(Some(uri.clone()), uri_at_attach);
            }
            other => panic!("expected Close, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_diagnostics_converts_utf8_byte_positions_to_char() {
        let (mut d, dir) = rust_doc("diag.rs", "fn héllo() {}");
        let (tx, _rx) = tokio_mpsc::unbounded_channel();
        d.attach_lsp(tx, PositionEncodingKind::UTF8);
        // 'é' starts at byte 4 (after "fn h"), is 2 bytes wide. Diagnostic
        // covering the 'é' byte range [4, 6) should resolve to char range [4, 5).
        let raw = vec![lsp_types::Diagnostic {
            range: lsp_types::Range {
                start: lsp_types::Position { line: 0, character: 4 },
                end: lsp_types::Position { line: 0, character: 6 },
            },
            severity: Some(lsp_types::DiagnosticSeverity::ERROR),
            message: "noisy é".into(),
            ..Default::default()
        }];
        d.set_diagnostics(raw);
        let diags = d.diagnostics();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].start_line, 0);
        assert_eq!(diags[0].start_char_in_line, 4);
        assert_eq!(diags[0].end_line, 0);
        assert_eq!(diags[0].end_char_in_line, 5);
        assert_eq!(diags[0].severity, lsp_types::DiagnosticSeverity::ERROR);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_diagnostics_drops_out_of_range_lines() {
        let (mut d, dir) = rust_doc("diag2.rs", "fn x() {}");
        let (tx, _rx) = tokio_mpsc::unbounded_channel();
        d.attach_lsp(tx, PositionEncodingKind::UTF8);
        let raw = vec![lsp_types::Diagnostic {
            range: lsp_types::Range {
                start: lsp_types::Position { line: 99, character: 0 },
                end: lsp_types::Position { line: 99, character: 1 },
            },
            severity: Some(lsp_types::DiagnosticSeverity::ERROR),
            message: "stale".into(),
            ..Default::default()
        }];
        d.set_diagnostics(raw);
        assert!(d.diagnostics().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_tx_with_no_sink_does_not_panic_or_send() {
        let (mut d, dir) = rust_doc("e.rs", "fn e() {}");
        // No attach.
        let len = d.buffer.len_chars();
        let edit = replace_selection_tx(&d.buffer, &Selection::point(len), "Z");
        d.apply_tx(edit);
        assert!(!d.lsp_attached());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
