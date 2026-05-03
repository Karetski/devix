//! `Coordinator` — owns one `LspClient` per (workspace_root, language_id),
//! routes per-document edits in, dispatches request-shaped commands (hover,
//! goto-def) on background tasks, and emits a typed `LspEvent` stream out.
//!
//! Identity: documents are addressed by `lsp_types::Uri` end-to-end. The App
//! is responsible for the `Uri ↔ workspace::DocId` mapping at the boundary,
//! which keeps `devix-lsp` from depending on `devix-workspace`.
//!
//! Lifecycle: `LspClient`s are spawned lazily on first `LspCommand::Open` for
//! a (root, language) pair via the supplied `Spawner` trait. The default
//! `SubprocessSpawner` shells out to the language's configured command;
//! tests inject `FnSpawner` closures that wire up duplex streams instead.
//!
//! Requests use `Arc<LspClient>` so a hover/definition future can be spawned
//! onto the runtime, await the response, and post it back through the same
//! `LspEvent` channel as a `HoverResponse` / `DefinitionResponse`. The App
//! correlates by `anchor_char` (the cursor position at request time),
//! discarding stale answers whose anchor no longer matches.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionParams, CompletionResponse, CompletionTriggerKind,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    GotoDefinitionParams, GotoDefinitionResponse, HoverContents, HoverParams, Location,
    MarkedString, Position, PublishDiagnosticsParams, TextDocumentContentChangeEvent,
    TextDocumentIdentifier, TextDocumentItem, TextDocumentPositionParams, Uri,
    VersionedTextDocumentIdentifier,
};
use tokio::sync::mpsc;

use crate::client::{ClientNotification, LspClient};

#[derive(Clone, Debug)]
pub struct LanguageConfig {
    /// Identity used to route documents (matches the `language_id` an LSP
    /// `textDocument/didOpen` advertises). e.g. `"rust"`.
    pub id: String,
    /// File names that mark a workspace root, walking upward from a doc's
    /// path. First matching ancestor wins. e.g. `["Cargo.toml"]`.
    pub root_markers: Vec<String>,
    /// `[program, args...]`. e.g. `["rust-analyzer"]`.
    pub command: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct CoordinatorConfig {
    pub languages: Vec<LanguageConfig>,
}

impl CoordinatorConfig {
    pub fn language(&self, id: &str) -> Option<&LanguageConfig> {
        self.languages.iter().find(|l| l.id == id)
    }
}

/// Inbound message from the App. Covers document lifecycle (open, change,
/// close) and request-shaped traffic (hover, goto-def). Both flow through
/// the same channel so producers don't need to track multiple sinks.
#[derive(Debug)]
pub enum LspCommand {
    Open {
        uri: Uri,
        language_id: String,
        version: i32,
        text: String,
        /// If `Some`, overrides the marker walk-up (useful for languages with
        /// no marker file, or test fixtures).
        root_hint: Option<PathBuf>,
    },
    Change {
        uri: Uri,
        version: i32,
        changes: Vec<TextDocumentContentChangeEvent>,
    },
    Close { uri: Uri },
    /// Request hover info at `position`. The originating cursor offset is
    /// echoed back through `anchor_char` on the `HoverResponse` so the App
    /// can discard stale answers without an external request-id map.
    Hover {
        uri: Uri,
        position: Position,
        anchor_char: usize,
    },
    GotoDefinition {
        uri: Uri,
        position: Position,
        anchor_char: usize,
    },
    /// Request completion items at `position`. `trigger` mirrors the LSP
    /// `CompletionContext`: `Manual` for an explicit Ctrl+Space, `Char(c)`
    /// when the user typed one of the server's trigger characters.
    Completion {
        uri: Uri,
        position: Position,
        anchor_char: usize,
        trigger: CompletionTrigger,
    },
}

/// What initiated a completion request — passed through to the server's
/// `CompletionContext` so it can rank/prefilter accordingly.
#[derive(Clone, Debug)]
pub enum CompletionTrigger {
    Manual,
    Char(char),
}

/// Outbound event for the App to drain each frame.
#[derive(Debug, Clone)]
pub enum LspEvent {
    Diagnostics(PublishDiagnosticsParams),
    ShowMessage { level: lsp_types::MessageType, text: String },
    LogMessage { level: lsp_types::MessageType, text: String },
    /// Hover result. `contents` is empty when the server returned null or
    /// the request errored — both surface as "no hover info" on the App
    /// side, which is the same outcome.
    HoverResponse {
        uri: Uri,
        anchor_char: usize,
        contents: Vec<String>,
    },
    DefinitionResponse {
        uri: Uri,
        anchor_char: usize,
        locations: Vec<Location>,
    },
    /// Completion result. `items` is the full list as returned by the
    /// server (already flattened from `CompletionResponse::Array | List`).
    /// Empty when the server returned null or the request errored.
    CompletionResponse {
        uri: Uri,
        anchor_char: usize,
        items: Vec<CompletionItem>,
        /// Mirrors LSP's `CompletionList::is_incomplete` — server signaled
        /// the list may need refetching after further keystrokes. Slice 3
        /// just tags it; client-side filter handles refinement until the
        /// cursor moves to a new context, when the App refetches anyway.
        is_incomplete: bool,
    },
}

pub trait Spawner {
    /// Spawn and initialize an `LspClient` for `(root, lang)`.
    /// `notif_tx` is the shared sink every client pushes notifications into;
    /// the coordinator owns the receiver in its run loop.
    fn spawn(
        &mut self,
        lang: &LanguageConfig,
        root: &Path,
        notif_tx: mpsc::UnboundedSender<ClientNotification>,
    ) -> impl std::future::Future<Output = Result<LspClient>> + Send;
}

/// Default production spawner: shells out to the configured command.
pub struct SubprocessSpawner;

impl Spawner for SubprocessSpawner {
    async fn spawn(
        &mut self,
        lang: &LanguageConfig,
        root: &Path,
        notif_tx: mpsc::UnboundedSender<ClientNotification>,
    ) -> Result<LspClient> {
        let (program, args) = lang
            .command
            .split_first()
            .context("LanguageConfig.command is empty")?;
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args).current_dir(root);
        let mut client = LspClient::spawn(cmd, notif_tx).await?;
        let root_uri = path_to_uri(root)?;
        let folders = vec![lsp_types::WorkspaceFolder {
            uri: root_uri.clone(),
            name: root
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }];
        client.initialize(Some(root_uri), folders).await?;
        Ok(client)
    }
}

#[derive(Debug, Clone)]
struct DocState {
    language_id: String,
    root: PathBuf,
}

type ClientKey = (PathBuf, String);

pub struct Coordinator<S: Spawner> {
    config: CoordinatorConfig,
    /// `Arc` so request futures can hold a handle while running on a
    /// background task, independent of coord's mutation of the map.
    clients: HashMap<ClientKey, Arc<LspClient>>,
    docs: HashMap<String, DocState>,
    spawner: S,
}

impl<S: Spawner> Coordinator<S> {
    pub fn new(config: CoordinatorConfig, spawner: S) -> Self {
        Self {
            config,
            clients: HashMap::new(),
            docs: HashMap::new(),
            spawner,
        }
    }

    /// Tied-to-channels pump. Owns:
    /// - inbound `LspCommand` (from App)
    /// - inbound `ClientNotification` (from every spawned client)
    /// - outbound `LspEvent` (to App)
    ///
    /// Exits cleanly when the inbound `commands` channel closes.
    pub async fn run(
        mut self,
        mut commands: mpsc::UnboundedReceiver<LspCommand>,
        events: mpsc::UnboundedSender<LspEvent>,
    ) {
        let (notif_tx, mut notif_rx) = mpsc::unbounded_channel::<ClientNotification>();
        loop {
            tokio::select! {
                biased;
                // Match on the Option directly so a closed `commands`
                // breaks the loop. We can't rely on `else` — the local
                // `notif_tx` keeps `notif_rx` open indefinitely, so the
                // notification branch never disables itself.
                cmd = commands.recv() => {
                    let Some(cmd) = cmd else { break };
                    if let Err(e) = self.handle_command(cmd, &notif_tx, &events).await {
                        let _ = events.send(LspEvent::LogMessage {
                            level: lsp_types::MessageType::ERROR,
                            text: format!("LSP coordinator: {e:#}"),
                        });
                    }
                }
                Some(notif) = notif_rx.recv() => {
                    if let Some(ev) = translate_notification(notif) {
                        let _ = events.send(ev);
                    }
                }
            }
        }
    }

    async fn handle_command(
        &mut self,
        cmd: LspCommand,
        notif_tx: &mpsc::UnboundedSender<ClientNotification>,
        events: &mpsc::UnboundedSender<LspEvent>,
    ) -> Result<()> {
        match cmd {
            LspCommand::Open {
                uri,
                language_id,
                version,
                text,
                root_hint,
            } => {
                let lang = self
                    .config
                    .language(&language_id)
                    .ok_or_else(|| anyhow!("no LanguageConfig for {language_id:?}"))?
                    .clone();
                let path = uri_to_path(&uri)?;
                let root = match root_hint {
                    Some(r) => r,
                    None => resolve_root(&path, &lang.root_markers)
                        .unwrap_or_else(|| path.parent().map(Path::to_path_buf).unwrap_or(path.clone())),
                };
                let key = (root.clone(), lang.id.clone());
                if !self.clients.contains_key(&key) {
                    let client = self.spawner.spawn(&lang, &root, notif_tx.clone()).await?;
                    self.clients.insert(key.clone(), Arc::new(client));
                }
                let client = self.clients.get(&key).unwrap();
                client.notify::<lsp_types::notification::DidOpenTextDocument>(
                    DidOpenTextDocumentParams {
                        text_document: TextDocumentItem {
                            uri: uri.clone(),
                            language_id: lang.id.clone(),
                            version,
                            text,
                        },
                    },
                )?;
                self.docs.insert(
                    uri_key(&uri),
                    DocState { language_id: lang.id, root },
                );
            }
            LspCommand::Change { uri, version, changes } => {
                let client = self.client_for_uri(&uri)?;
                client.notify::<lsp_types::notification::DidChangeTextDocument>(
                    DidChangeTextDocumentParams {
                        text_document: VersionedTextDocumentIdentifier { uri, version },
                        content_changes: changes,
                    },
                )?;
            }
            LspCommand::Close { uri } => {
                let Some(state) = self.docs.remove(&uri_key(&uri)) else {
                    return Ok(());
                };
                let key: ClientKey = (state.root.clone(), state.language_id);
                let Some(client) = self.clients.get(&key) else { return Ok(()) };
                client.notify::<lsp_types::notification::DidCloseTextDocument>(
                    DidCloseTextDocumentParams {
                        text_document: TextDocumentIdentifier { uri },
                    },
                )?;
            }
            LspCommand::Hover { uri, position, anchor_char } => {
                let client = Arc::clone(self.client_for_uri(&uri)?);
                let events = events.clone();
                let uri_resp = uri.clone();
                tokio::spawn(async move {
                    let params = HoverParams {
                        text_document_position_params: TextDocumentPositionParams {
                            text_document: TextDocumentIdentifier { uri },
                            position,
                        },
                        work_done_progress_params: Default::default(),
                    };
                    let contents = match client
                        .request::<lsp_types::request::HoverRequest>(params)
                        .await
                    {
                        Ok(Some(h)) => hover_contents_to_lines(h.contents),
                        _ => Vec::new(),
                    };
                    let _ = events.send(LspEvent::HoverResponse {
                        uri: uri_resp,
                        anchor_char,
                        contents,
                    });
                });
            }
            LspCommand::GotoDefinition { uri, position, anchor_char } => {
                let client = Arc::clone(self.client_for_uri(&uri)?);
                let events = events.clone();
                let uri_resp = uri.clone();
                tokio::spawn(async move {
                    let params = GotoDefinitionParams {
                        text_document_position_params: TextDocumentPositionParams {
                            text_document: TextDocumentIdentifier { uri },
                            position,
                        },
                        work_done_progress_params: Default::default(),
                        partial_result_params: Default::default(),
                    };
                    let locations = match client
                        .request::<lsp_types::request::GotoDefinition>(params)
                        .await
                    {
                        Ok(Some(resp)) => goto_definition_to_locations(resp),
                        _ => Vec::new(),
                    };
                    let _ = events.send(LspEvent::DefinitionResponse {
                        uri: uri_resp,
                        anchor_char,
                        locations,
                    });
                });
            }
            LspCommand::Completion { uri, position, anchor_char, trigger } => {
                let client = Arc::clone(self.client_for_uri(&uri)?);
                let events = events.clone();
                let uri_resp = uri.clone();
                tokio::spawn(async move {
                    let context = match trigger {
                        CompletionTrigger::Manual => CompletionContext {
                            trigger_kind: CompletionTriggerKind::INVOKED,
                            trigger_character: None,
                        },
                        CompletionTrigger::Char(c) => CompletionContext {
                            trigger_kind: CompletionTriggerKind::TRIGGER_CHARACTER,
                            trigger_character: Some(c.to_string()),
                        },
                    };
                    let params = CompletionParams {
                        text_document_position: TextDocumentPositionParams {
                            text_document: TextDocumentIdentifier { uri },
                            position,
                        },
                        work_done_progress_params: Default::default(),
                        partial_result_params: Default::default(),
                        context: Some(context),
                    };
                    let (items, is_incomplete) = match client
                        .request::<lsp_types::request::Completion>(params)
                        .await
                    {
                        Ok(Some(resp)) => completion_response_flatten(resp),
                        _ => (Vec::new(), false),
                    };
                    let _ = events.send(LspEvent::CompletionResponse {
                        uri: uri_resp,
                        anchor_char,
                        items,
                        is_incomplete,
                    });
                });
            }
        }
        Ok(())
    }

    fn client_for_uri(&self, uri: &Uri) -> Result<&Arc<LspClient>> {
        let state = self
            .docs
            .get(&uri_key(uri))
            .ok_or_else(|| anyhow!("LSP command for unknown uri {uri:?}"))?;
        let key: ClientKey = (state.root.clone(), state.language_id.clone());
        self.clients
            .get(&key)
            .ok_or_else(|| anyhow!("no client for {key:?}"))
    }
}

fn translate_notification(n: ClientNotification) -> Option<LspEvent> {
    match n.method.as_str() {
        "textDocument/publishDiagnostics" => {
            let p: PublishDiagnosticsParams = serde_json::from_value(n.params).ok()?;
            Some(LspEvent::Diagnostics(p))
        }
        "window/showMessage" => {
            let p: lsp_types::ShowMessageParams = serde_json::from_value(n.params).ok()?;
            Some(LspEvent::ShowMessage { level: p.typ, text: p.message })
        }
        "window/logMessage" => {
            let p: lsp_types::LogMessageParams = serde_json::from_value(n.params).ok()?;
            Some(LspEvent::LogMessage { level: p.typ, text: p.message })
        }
        // Drop everything else for slice 1: progress, telemetry, registerCapability
        // (which arrives as a server→client *request*, handled by the client's
        // auto-reply path, not as a notification).
        _ => None,
    }
}

/// Flatten `HoverContents` into a list of plain-text lines. Markdown markers
/// pass through unrendered for slice 2; the popup widget treats each entry
/// as a line and the user sees raw markdown. Real markdown rendering can
/// land later without changing this signature.
fn hover_contents_to_lines(c: HoverContents) -> Vec<String> {
    fn marked_to_string(m: MarkedString) -> String {
        match m {
            MarkedString::String(s) => s,
            MarkedString::LanguageString(ls) => ls.value,
        }
    }
    let mut out: Vec<String> = match c {
        HoverContents::Scalar(m) => vec![marked_to_string(m)],
        HoverContents::Array(items) => items.into_iter().map(marked_to_string).collect(),
        HoverContents::Markup(m) => vec![m.value],
    };
    // Split entries on newlines so the popup renders one line per row.
    let mut lines = Vec::with_capacity(out.len());
    for s in out.drain(..) {
        for line in s.split('\n') {
            lines.push(line.trim_end_matches('\r').to_string());
        }
    }
    // Trim trailing blank lines so popups don't render an empty footer.
    while matches!(lines.last(), Some(s) if s.is_empty()) {
        lines.pop();
    }
    lines
}

/// Flatten LSP's `CompletionResponse::Array | List` into a single vec plus
/// the `is_incomplete` bit. Slice 3 ignores the bit beyond surfacing it on
/// the event; we refetch on cursor motion regardless.
fn completion_response_flatten(r: CompletionResponse) -> (Vec<CompletionItem>, bool) {
    match r {
        CompletionResponse::Array(items) => (items, false),
        CompletionResponse::List(list) => (list.items, list.is_incomplete),
    }
}

fn goto_definition_to_locations(r: GotoDefinitionResponse) -> Vec<Location> {
    match r {
        GotoDefinitionResponse::Scalar(loc) => vec![loc],
        GotoDefinitionResponse::Array(locs) => locs,
        GotoDefinitionResponse::Link(links) => links
            .into_iter()
            .map(|l| Location {
                uri: l.target_uri,
                range: l.target_selection_range,
            })
            .collect(),
    }
}

/// `file://` percent-encoding is fiddly; for slice 1 we use the URI's
/// `to_string()` as a HashMap key. This is round-trip stable for paths we
/// produced ourselves via `path_to_uri`. Not normalized against arbitrary
/// inputs — sufficient because every URI in the coordinator's docs map was
/// produced by us.
fn uri_key(uri: &Uri) -> String {
    uri.as_str().to_string()
}

pub fn path_to_uri(path: &Path) -> Result<Uri> {
    use std::str::FromStr;
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let s = abs.to_string_lossy();
    let normalized = if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{}", s.replace('\\', "/"))
    };
    Uri::from_str(&normalized).with_context(|| format!("building Uri from {normalized:?}"))
}

pub fn uri_to_path(uri: &Uri) -> Result<PathBuf> {
    let s = uri.as_str();
    let stripped = s
        .strip_prefix("file://")
        .ok_or_else(|| anyhow!("non-file URI: {s}"))?;
    // POSIX: file:///foo → /foo. Windows: file:///C:/foo → C:/foo.
    let stripped = stripped.strip_prefix('/').unwrap_or(stripped);
    if stripped.starts_with(|c: char| c.is_ascii_alphabetic())
        && stripped.chars().nth(1) == Some(':')
    {
        Ok(PathBuf::from(stripped))
    } else {
        Ok(PathBuf::from(format!("/{stripped}")))
    }
}

fn resolve_root(path: &Path, markers: &[String]) -> Option<PathBuf> {
    let mut cur = path.parent()?;
    loop {
        for m in markers {
            if cur.join(m).exists() {
                return Some(cur.to_path_buf());
            }
        }
        cur = cur.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framing::{FrameReader, write_frame};
    use crate::jsonrpc::ServerMessage;
    use std::str::FromStr;
    use tokio::io::{AsyncWrite, split};

    /// Spawner that builds a fresh `LspClient` on each call and ships the
    /// corresponding `FakeServer` half out through `servers_tx`. Tests pull
    /// FakeServers off the matching receiver to drive the wire from the
    /// server side. This is the only correct way to wire mock clients —
    /// the client must use the spawner-supplied `notif_tx` so its inbound
    /// notifications reach the coordinator's `notif_rx`.
    struct MockSpawner {
        servers_tx: mpsc::UnboundedSender<FakeServer>,
    }

    impl Spawner for MockSpawner {
        async fn spawn(
            &mut self,
            _lang: &LanguageConfig,
            _root: &Path,
            notif_tx: mpsc::UnboundedSender<ClientNotification>,
        ) -> Result<LspClient> {
            let (client, server) = build_pair(notif_tx);
            let _ = self.servers_tx.send(server);
            Ok(client)
        }
    }

    struct FakeServer {
        reader: FrameReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
        writer: Box<dyn AsyncWrite + Unpin + Send>,
    }

    impl FakeServer {
        async fn recv(&mut self) -> ServerMessage {
            let bytes = self
                .reader
                .read_frame()
                .await
                .expect("frame read")
                .expect("frame present");
            serde_json::from_slice(&bytes).expect("valid JSON")
        }

        async fn notify(&mut self, method: &str, params: serde_json::Value) {
            let n = serde_json::json!({"jsonrpc": "2.0", "method": method, "params": params});
            let bytes = serde_json::to_vec(&n).unwrap();
            write_frame(&mut self.writer, &bytes).await.unwrap();
        }

        async fn respond(&mut self, id: crate::jsonrpc::RequestId, result: serde_json::Value) {
            let resp = serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result});
            let bytes = serde_json::to_vec(&resp).unwrap();
            write_frame(&mut self.writer, &bytes).await.unwrap();
        }
    }

    fn build_pair(notif_tx: mpsc::UnboundedSender<ClientNotification>) -> (LspClient, FakeServer) {
        let (client_side, server_side) = tokio::io::duplex(8192);
        let (client_rd, client_wr) = split(client_side);
        let (server_rd, server_wr) = split(server_side);
        let client = LspClient::with_streams(client_rd, client_wr, notif_tx);
        let server = FakeServer {
            reader: FrameReader::new(server_rd),
            writer: Box::new(server_wr),
        };
        (client, server)
    }

    fn make_coord() -> (Coordinator<MockSpawner>, mpsc::UnboundedReceiver<FakeServer>) {
        let (servers_tx, servers_rx) = mpsc::unbounded_channel::<FakeServer>();
        let coord = Coordinator::new(rust_config(), MockSpawner { servers_tx });
        (coord, servers_rx)
    }

    fn rust_config() -> CoordinatorConfig {
        CoordinatorConfig {
            languages: vec![LanguageConfig {
                id: "rust".into(),
                root_markers: vec!["Cargo.toml".into()],
                command: vec!["true".into()],
            }],
        }
    }

    #[tokio::test]
    async fn open_change_close_roundtrip_to_one_client() {
        let (coord, mut servers_rx) = make_coord();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (events_tx, _events_rx) = mpsc::unbounded_channel();
        let run_handle = tokio::spawn(coord.run(cmd_rx, events_tx));

        let uri = Uri::from_str("file:///proj/src/main.rs").unwrap();
        cmd_tx
            .send(LspCommand::Open {
                uri: uri.clone(),
                language_id: "rust".into(),
                version: 1,
                text: "fn main() {}".into(),
                root_hint: Some(PathBuf::from("/proj")),
            })
            .unwrap();
        let mut server = servers_rx.recv().await.unwrap();

        let m = server.recv().await;
        assert_eq!(m.method.as_deref(), Some("textDocument/didOpen"));
        assert_eq!(m.params.as_ref().unwrap()["textDocument"]["uri"], uri.as_str());
        assert_eq!(m.params.as_ref().unwrap()["textDocument"]["text"], "fn main() {}");

        cmd_tx
            .send(LspCommand::Change {
                uri: uri.clone(),
                version: 2,
                changes: vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: "fn main() { println!(\"hi\"); }".into(),
                }],
            })
            .unwrap();

        let m = server.recv().await;
        assert_eq!(m.method.as_deref(), Some("textDocument/didChange"));
        assert_eq!(m.params.as_ref().unwrap()["textDocument"]["version"], 2);

        cmd_tx
            .send(LspCommand::Close { uri: uri.clone() })
            .unwrap();

        let m = server.recv().await;
        assert_eq!(m.method.as_deref(), Some("textDocument/didClose"));

        drop(cmd_tx);
        run_handle.await.unwrap();
    }

    #[tokio::test]
    async fn diagnostics_from_server_translate_to_lsp_event() {
        let (coord, mut servers_rx) = make_coord();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let run_handle = tokio::spawn(coord.run(cmd_rx, events_tx));

        let uri = Uri::from_str("file:///proj/lib.rs").unwrap();
        cmd_tx
            .send(LspCommand::Open {
                uri: uri.clone(),
                language_id: "rust".into(),
                version: 1,
                text: String::new(),
                root_hint: Some(PathBuf::from("/proj")),
            })
            .unwrap();
        let mut server = servers_rx.recv().await.unwrap();
        let _ = server.recv().await; // didOpen

        server
            .notify(
                "textDocument/publishDiagnostics",
                serde_json::json!({
                    "uri": uri.as_str(),
                    "diagnostics": [{
                        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                        "severity": 1,
                        "message": "boom",
                    }],
                }),
            )
            .await;

        let ev = events_rx.recv().await.expect("event");
        match ev {
            LspEvent::Diagnostics(p) => {
                assert_eq!(p.uri, uri);
                assert_eq!(p.diagnostics.len(), 1);
                assert_eq!(p.diagnostics[0].message, "boom");
            }
            other => panic!("expected Diagnostics, got {other:?}"),
        }

        drop(cmd_tx);
        run_handle.await.unwrap();
    }

    #[tokio::test]
    async fn second_open_in_same_root_reuses_client() {
        let (coord, mut servers_rx) = make_coord();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (events_tx, _events_rx) = mpsc::unbounded_channel();
        let run_handle = tokio::spawn(coord.run(cmd_rx, events_tx));

        for path in ["file:///proj/a.rs", "file:///proj/b.rs"] {
            cmd_tx
                .send(LspCommand::Open {
                    uri: Uri::from_str(path).unwrap(),
                    language_id: "rust".into(),
                    version: 1,
                    text: String::new(),
                    root_hint: Some(PathBuf::from("/proj")),
                })
                .unwrap();
        }

        let mut server = servers_rx.recv().await.unwrap();
        // No second server arrives — same (root, language) reuses the client.
        let m1 = server.recv().await;
        let m2 = server.recv().await;
        assert_eq!(m1.method.as_deref(), Some("textDocument/didOpen"));
        assert_eq!(m2.method.as_deref(), Some("textDocument/didOpen"));
        assert_ne!(m1.params.as_ref().unwrap()["textDocument"]["uri"],
                   m2.params.as_ref().unwrap()["textDocument"]["uri"]);
        assert!(servers_rx.try_recv().is_err(), "second open should reuse the existing client");

        drop(cmd_tx);
        run_handle.await.unwrap();
    }

    #[tokio::test]
    async fn unknown_language_emits_error_log() {
        let (coord, _servers_rx) = make_coord();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let run_handle = tokio::spawn(coord.run(cmd_rx, events_tx));

        cmd_tx
            .send(LspCommand::Open {
                uri: Uri::from_str("file:///proj/file.unknownlang").unwrap(),
                language_id: "unknownlang".into(),
                version: 1,
                text: String::new(),
                root_hint: Some(PathBuf::from("/proj")),
            })
            .unwrap();

        let ev = events_rx.recv().await.expect("event");
        match ev {
            LspEvent::LogMessage { level, text } => {
                assert_eq!(level, lsp_types::MessageType::ERROR);
                assert!(text.contains("unknownlang"), "got: {text}");
            }
            other => panic!("expected LogMessage, got {other:?}"),
        }

        drop(cmd_tx);
        run_handle.await.unwrap();
    }

    #[tokio::test]
    async fn hover_request_pumps_response_back_through_events() {
        let (coord, mut servers_rx) = make_coord();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let run_handle = tokio::spawn(coord.run(cmd_rx, events_tx));

        let uri = Uri::from_str("file:///proj/h.rs").unwrap();
        cmd_tx
            .send(LspCommand::Open {
                uri: uri.clone(),
                language_id: "rust".into(),
                version: 1,
                text: "fn x() {}".into(),
                root_hint: Some(PathBuf::from("/proj")),
            })
            .unwrap();
        let mut server = servers_rx.recv().await.unwrap();
        let _ = server.recv().await; // didOpen

        cmd_tx
            .send(LspCommand::Hover {
                uri: uri.clone(),
                position: Position { line: 0, character: 3 },
                anchor_char: 3,
            })
            .unwrap();

        let req = server.recv().await;
        assert_eq!(req.method.as_deref(), Some("textDocument/hover"));
        let id = req.id.expect("hover request has id");
        server
            .respond(
                id,
                serde_json::json!({
                    "contents": {"kind": "plaintext", "value": "fn x()\nreturns ()"},
                }),
            )
            .await;

        let ev = events_rx.recv().await.expect("event");
        match ev {
            LspEvent::HoverResponse { uri: u, anchor_char, contents } => {
                assert_eq!(u, uri);
                assert_eq!(anchor_char, 3);
                assert_eq!(contents, vec!["fn x()".to_string(), "returns ()".to_string()]);
            }
            other => panic!("expected HoverResponse, got {other:?}"),
        }

        drop(cmd_tx);
        run_handle.await.unwrap();
    }

    #[tokio::test]
    async fn goto_definition_request_pumps_locations_back() {
        let (coord, mut servers_rx) = make_coord();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let run_handle = tokio::spawn(coord.run(cmd_rx, events_tx));

        let uri = Uri::from_str("file:///proj/g.rs").unwrap();
        cmd_tx
            .send(LspCommand::Open {
                uri: uri.clone(),
                language_id: "rust".into(),
                version: 1,
                text: "fn x() {}\nfn y() { x(); }".into(),
                root_hint: Some(PathBuf::from("/proj")),
            })
            .unwrap();
        let mut server = servers_rx.recv().await.unwrap();
        let _ = server.recv().await;

        cmd_tx
            .send(LspCommand::GotoDefinition {
                uri: uri.clone(),
                position: Position { line: 1, character: 9 },
                anchor_char: 19,
            })
            .unwrap();

        let req = server.recv().await;
        assert_eq!(req.method.as_deref(), Some("textDocument/definition"));
        let id = req.id.expect("definition request has id");
        server
            .respond(
                id,
                serde_json::json!({
                    "uri": uri.as_str(),
                    "range": {
                        "start": {"line": 0, "character": 3},
                        "end":   {"line": 0, "character": 4},
                    },
                }),
            )
            .await;

        let ev = events_rx.recv().await.expect("event");
        match ev {
            LspEvent::DefinitionResponse { uri: u, anchor_char, locations } => {
                assert_eq!(u, uri);
                assert_eq!(anchor_char, 19);
                assert_eq!(locations.len(), 1);
                assert_eq!(locations[0].uri, uri);
                assert_eq!(locations[0].range.start, Position { line: 0, character: 3 });
            }
            other => panic!("expected DefinitionResponse, got {other:?}"),
        }

        drop(cmd_tx);
        run_handle.await.unwrap();
    }

    #[tokio::test]
    async fn completion_request_pumps_items_back() {
        let (coord, mut servers_rx) = make_coord();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let run_handle = tokio::spawn(coord.run(cmd_rx, events_tx));

        let uri = Uri::from_str("file:///proj/c.rs").unwrap();
        cmd_tx
            .send(LspCommand::Open {
                uri: uri.clone(),
                language_id: "rust".into(),
                version: 1,
                text: "fn main() { let s = String::ne".into(),
                root_hint: Some(PathBuf::from("/proj")),
            })
            .unwrap();
        let mut server = servers_rx.recv().await.unwrap();
        let _ = server.recv().await;

        cmd_tx
            .send(LspCommand::Completion {
                uri: uri.clone(),
                position: Position { line: 0, character: 30 },
                anchor_char: 30,
                trigger: CompletionTrigger::Char(':'),
            })
            .unwrap();

        let req = server.recv().await;
        assert_eq!(req.method.as_deref(), Some("textDocument/completion"));
        // Verify trigger context made the trip.
        let ctx = &req.params.as_ref().unwrap()["context"];
        assert_eq!(ctx["triggerKind"], 2); // TRIGGER_CHARACTER
        assert_eq!(ctx["triggerCharacter"], ":");
        let id = req.id.expect("completion request has id");
        server
            .respond(
                id,
                serde_json::json!({
                    "isIncomplete": true,
                    "items": [
                        { "label": "new", "detail": "fn() -> String", "kind": 3 },
                        { "label": "next", "kind": 6 },
                    ],
                }),
            )
            .await;

        let ev = events_rx.recv().await.expect("event");
        match ev {
            LspEvent::CompletionResponse { uri: u, anchor_char, items, is_incomplete } => {
                assert_eq!(u, uri);
                assert_eq!(anchor_char, 30);
                assert!(is_incomplete);
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].label, "new");
                assert_eq!(items[0].detail.as_deref(), Some("fn() -> String"));
                assert_eq!(items[1].label, "next");
            }
            other => panic!("expected CompletionResponse, got {other:?}"),
        }

        drop(cmd_tx);
        run_handle.await.unwrap();
    }

    #[test]
    fn resolve_root_walks_up_to_marker() {
        let dir = std::env::temp_dir().join(format!("devix-root-{}", std::process::id()));
        let nested = dir.join("crates").join("foo").join("src");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "").unwrap();
        let file = nested.join("lib.rs");
        std::fs::write(&file, "").unwrap();
        // resolve_root walks parent links lexically — no canonicalization,
        // so it returns the same prefix as the input path. Comparing via
        // ends_with avoids the macOS /tmp ↔ /private/tmp dance.
        let root = resolve_root(&file, &["Cargo.toml".to_string()]).expect("found root");
        assert_eq!(root, dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_root_returns_none_with_no_marker() {
        let dir = std::env::temp_dir().join(format!("devix-noroot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("loose.txt");
        std::fs::write(&file, "").unwrap();
        assert!(resolve_root(&file, &["Cargo.toml".to_string()]).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hover_contents_markup_splits_on_newlines() {
        let m = HoverContents::Markup(lsp_types::MarkupContent {
            kind: lsp_types::MarkupKind::Markdown,
            value: "fn x()\n\n```rust\nfn x() {}\n```".into(),
        });
        let lines = hover_contents_to_lines(m);
        assert_eq!(lines, vec![
            "fn x()".to_string(),
            String::new(),
            "```rust".to_string(),
            "fn x() {}".to_string(),
            "```".to_string(),
        ]);
    }
}
