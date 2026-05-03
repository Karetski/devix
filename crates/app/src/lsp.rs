//! App-side LSP wiring: spawns the coordinator on a tokio runtime, drains
//! `LspEvent`s into Document/StatusLine state each tick, mirroring
//! [`crate::watcher::drain_disk_events`].
//!
//! Slice 1 surfaces `publishDiagnostics` (→ Document::set_diagnostics) and
//! window/show|logMessage (→ status line). Hover, completion, and the
//! request-shaped LSP traffic land on later slices.

use anyhow::Result;
use devix_lsp::{
    Coordinator, CoordinatorConfig, LanguageConfig, LspCommand, LspEvent, SubprocessSpawner,
    uri_to_path,
};
use devix_workspace::HoverStatus;
use lsp_types::{Location, PositionEncodingKind};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::app::App;

/// Owns the runtime + the inbound LspEvent stream. Held on `App` for the
/// lifetime of the editor; dropping it tears down the coordinator and every
/// child server process (clients are spawned with `kill_on_drop`).
pub struct LspWiring {
    /// Held only for the Drop side effect — dropping the runtime tears down
    /// every spawned task (the coordinator + every LspClient's reader/writer
    /// pair), and `kill_on_drop` reaps the child server processes.
    #[allow(dead_code)]
    pub runtime: Runtime,
    pub events_rx: mpsc::UnboundedReceiver<LspEvent>,
}

/// Build the runtime, spawn the coordinator, and return the change sink +
/// event receiver wrapper. Caller threads `sink` into `Workspace::attach_lsp`.
pub fn setup_lsp() -> Result<(mpsc::UnboundedSender<LspCommand>, PositionEncodingKind, LspWiring)> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("devix-lsp")
        .build()?;

    let (changes_tx, changes_rx) = mpsc::unbounded_channel::<LspCommand>();
    let (events_tx, events_rx) = mpsc::unbounded_channel::<LspEvent>();
    let coord = Coordinator::new(default_config(), SubprocessSpawner);
    runtime.spawn(coord.run(changes_rx, events_tx));

    // Slice 1 advertises utf-8 in initialize and assumes the server agrees.
    // rust-analyzer accepts utf-8; if some other server we add later refuses,
    // its diagnostics positions will be slightly off (but not crashy) until
    // we plumb per-client encoding back through the workspace.
    let encoding = PositionEncodingKind::UTF8;
    Ok((changes_tx, encoding, LspWiring { runtime, events_rx }))
}

fn default_config() -> CoordinatorConfig {
    CoordinatorConfig {
        languages: vec![LanguageConfig {
            id: "rust".into(),
            root_markers: vec!["Cargo.toml".into()],
            command: vec!["rust-analyzer".into()],
        }],
    }
}

/// Drain pending LspEvents and apply them to App state. Pure data updates
/// (diagnostics) mutate Document directly; ShowMessage / LogMessage feed
/// the status line. Marks `app.dirty` if anything changed visible state.
pub fn drain_lsp_events(app: &mut App) {
    // Drain into a local buffer first so the mutable borrow on `app.lsp`
    // ends before per-event handlers re-borrow `app` for hover/goto-def
    // application.
    let mut pending: Vec<LspEvent> = Vec::new();
    if let Some(wiring) = app.lsp.as_mut() {
        while let Ok(ev) = wiring.events_rx.try_recv() {
            pending.push(ev);
        }
    }
    if pending.is_empty() {
        return;
    }
    for ev in pending {
        match ev {
            LspEvent::Diagnostics(p) => {
                let Ok(target_path) = uri_to_path(&p.uri) else { continue };
                let mut found: Option<devix_workspace::DocId> = None;
                for (id, doc) in app.workspace.documents.iter() {
                    let Some(uri) = doc.lsp_uri() else { continue };
                    if let Ok(doc_path) = uri_to_path(uri) {
                        if same_path(&doc_path, &target_path) {
                            found = Some(id);
                            break;
                        }
                    }
                }
                if let Some(id) = found {
                    app.workspace.documents[id].set_diagnostics(p.diagnostics);
                }
            }
            LspEvent::ShowMessage { level, text } => {
                app.status.set(format!("{} {}", message_prefix(level), text));
            }
            LspEvent::LogMessage { level, text } => {
                // Slice 1: surface errors only on the status line; ignore
                // INFO/LOG noise from rust-analyzer indexing chatter.
                if level == lsp_types::MessageType::ERROR {
                    app.status.set(format!("LSP error: {text}"));
                }
            }
            LspEvent::HoverResponse { uri, anchor_char, contents } => {
                apply_hover_response(app, &uri, anchor_char, contents);
            }
            LspEvent::DefinitionResponse { uri, anchor_char, locations } => {
                apply_definition_response(app, &uri, anchor_char, locations);
            }
            // Completion response: applied with the UI commit.
            LspEvent::CompletionResponse { .. } => {}
        }
    }
    app.dirty = true;
}

/// Match the response against an open view of `uri` whose `hover.anchor_char`
/// equals `anchor_char`. Stale responses (cursor moved, popup dismissed) are
/// dropped silently — the request was already work the server performed,
/// nothing to undo.
fn apply_hover_response(app: &mut App, uri: &lsp_types::Uri, anchor_char: usize, contents: Vec<String>) {
    let Ok(target_path) = uri_to_path(uri) else { return };
    let mut target_did: Option<devix_workspace::DocId> = None;
    for (id, doc) in app.workspace.documents.iter() {
        let Some(doc_uri) = doc.lsp_uri() else { continue };
        let Ok(doc_path) = uri_to_path(doc_uri) else { continue };
        if same_path(&doc_path, &target_path) {
            target_did = Some(id);
            break;
        }
    }
    let Some(did) = target_did else { return };
    for view in app.workspace.views.values_mut() {
        if view.doc != did {
            continue;
        }
        let Some(hover) = view.hover.as_mut() else { continue };
        if hover.anchor_char != anchor_char {
            continue;
        }
        hover.status = if contents.is_empty() {
            HoverStatus::Empty
        } else {
            HoverStatus::Ready(contents.clone())
        };
    }
}

/// Jump to the first location returned by goto-definition. If the file is
/// already open in any view, switch to it; otherwise replace the current
/// tab. Locations beyond the first are ignored for slice 2 — the
/// "implementations" list UX lands later.
fn apply_definition_response(
    app: &mut App,
    _uri: &lsp_types::Uri,
    anchor_char: usize,
    locations: Vec<Location>,
) {
    // anchor_char here is the originating cursor position. We don't need it
    // to dispatch goto-def — the user pressed F12 once and moved to wait;
    // unlike hover there's no per-position state to invalidate. Kept on the
    // event for symmetry and so future "ignore if user already navigated"
    // gates can plug in without a wire change.
    let _ = anchor_char;
    let Some(loc) = locations.into_iter().next() else {
        app.status.set("no definition found");
        return;
    };
    let Ok(target_path) = uri_to_path(&loc.uri) else { return };

    // Convert the LSP Position to a char index against the (eventually) open
    // document so we can position the cursor. For an already-open doc, use
    // its rope; for a fresh open, the call falls through after open_path_*.
    let want_line = loc.range.start.line as usize;
    let want_char_in_line = loc.range.start.character as usize;

    // If `target_path` is open in any view of any frame, prefer that — it
    // avoids replacing the user's current tab.
    let mut hit: Option<devix_workspace::ViewId> = None;
    'outer: for (vid, view) in app.workspace.views.iter() {
        let doc = &app.workspace.documents[view.doc];
        let Some(doc_path) = doc.buffer.path() else { continue };
        if same_path(doc_path, &target_path) {
            hit = Some(vid);
            break 'outer;
        }
    }
    if let Some(vid) = hit {
        if let Some(fid) = frame_owning_view(&app.workspace, vid) {
            app.workspace.focus_frame(fid);
            // Best-effort: select the matching tab so the focused view is
            // the one we hit.
            if let Some(idx) = app.workspace.frames[fid].tabs.iter().position(|&v| v == vid) {
                app.workspace.activate_tab(fid, idx);
            }
        }
        position_cursor_at(app, vid, want_line, want_char_in_line);
        return;
    }

    if let Err(e) = app.workspace.open_path_replace_current(target_path) {
        app.status.set(format!("goto-def open failed: {e}"));
        return;
    }
    if let Some((_, vid, _)) = app.workspace.active_ids() {
        position_cursor_at(app, vid, want_line, want_char_in_line);
    }
}

fn frame_owning_view(ws: &devix_workspace::Workspace, vid: devix_workspace::ViewId) -> Option<devix_workspace::FrameId> {
    for (fid, frame) in ws.frames.iter() {
        if frame.tabs.contains(&vid) {
            return Some(fid);
        }
    }
    None
}

fn position_cursor_at(app: &mut App, vid: devix_workspace::ViewId, line: usize, char_in_line: usize) {
    let did = app.workspace.views[vid].doc;
    let buf = &app.workspace.documents[did].buffer;
    let line = line.min(buf.line_count().saturating_sub(1));
    let line_len = buf.line_len_chars(line);
    let col = char_in_line.min(line_len);
    let idx = buf.line_start(line) + col;
    let v = &mut app.workspace.views[vid];
    v.move_to(idx, false, false);
    v.scroll_mode = devix_workspace::ScrollMode::Anchored;
}

fn message_prefix(t: lsp_types::MessageType) -> &'static str {
    use lsp_types::MessageType;
    match t {
        MessageType::ERROR => "LSP error:",
        MessageType::WARNING => "LSP:",
        MessageType::INFO => "LSP:",
        MessageType::LOG => "LSP:",
        _ => "LSP:",
    }
}

fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    let ca = std::fs::canonicalize(a).ok();
    let cb = std::fs::canonicalize(b).ok();
    match (ca, cb) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}
