//! App-side LSP wiring: spawns the coordinator on a tokio runtime, drains
//! `LspEvent`s into Document/StatusLine state each tick, mirroring
//! [`crate::watcher::drain_disk_events`].
//!
//! Slice 1 surfaced `publishDiagnostics` (→ Document::set_diagnostics) and
//! window/show|logMessage (→ status line). Slice 2 plumbs hover and
//! goto-definition responses end-to-end at the wire level; the App-side
//! application of those responses (popup paint, cursor jump) lands in the
//! UI commit.

use anyhow::Result;
use devix_lsp::{
    Coordinator, CoordinatorConfig, LanguageConfig, LspCommand, LspEvent, SubprocessSpawner,
    uri_to_path,
};
use lsp_types::PositionEncodingKind;
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
    let Some(wiring) = app.lsp.as_mut() else { return };
    let mut any = false;
    while let Ok(ev) = wiring.events_rx.try_recv() {
        any = true;
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
            // Hover / goto-def responses arrive here. The App-side
            // application (popup paint, cursor jump) lands with the UI
            // commit; for now they're produced by coord but consumed
            // nowhere — drop them on the floor.
            LspEvent::HoverResponse { .. } | LspEvent::DefinitionResponse { .. } => {}
        }
    }
    if any {
        app.dirty = true;
    }
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
