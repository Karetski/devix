//! `LspClient` — async front-end for one language server.
//!
//! Architecture:
//!
//! ```text
//!  outbound_tx ─► writer task ──stdin──► server
//!                                          │
//!                                        stdout
//!                                          │
//!                  ┌─ pending map ◄──── reader task
//!                  └─ notifications mpsc ◄┘
//! ```
//!
//! - Public methods (`request`, `notify`, `initialize`) push pre-encoded
//!   JSON bytes onto an unbounded mpsc consumed by the writer task. Writes
//!   never block the caller.
//! - The reader task demuxes inbound `ServerMessage`s. Responses fulfil the
//!   matching `oneshot::Sender` parked in the pending map. Notifications
//!   forward to the coordinator-supplied `notifications` channel. Server →
//!   client requests are answered with `method not found` for now (slice ≥2
//!   may handle `workspace/configuration` etc.).
//! - Drop kills the child via `kill_on_drop`. The reader exits on stdout EOF;
//!   the writer exits when `outbound_tx` is dropped (which happens on
//!   `LspClient` drop, since the writer holds no clones).

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicI64, Ordering};

use anyhow::{Context, Result, anyhow};
use lsp_types::{
    ClientCapabilities, ClientInfo, GeneralClientCapabilities, InitializeParams, InitializeResult,
    InitializedParams, PositionEncodingKind, PublishDiagnosticsClientCapabilities,
    ServerCapabilities, ServerInfo, TextDocumentClientCapabilities,
    TextDocumentSyncClientCapabilities, Uri, WorkspaceClientCapabilities, WorkspaceFolder,
};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

use crate::framing::{FrameReader, write_frame};
use crate::jsonrpc::{
    Notification, Request, RequestId, ResponseError, ServerMessage, ServerMessageKind,
};

type Pending = Arc<Mutex<HashMap<RequestId, oneshot::Sender<Result<Value, ResponseError>>>>>;

/// Notification (or unmatched server-request) forwarded out of the client.
/// Coordinator wraps these with the originating client identity before
/// forwarding to the App's drain queue.
#[derive(Debug, Clone)]
pub struct ClientNotification {
    pub method: String,
    pub params: Value,
}

pub struct LspClient {
    next_id: AtomicI64,
    pending: Pending,
    outbound_tx: mpsc::UnboundedSender<Vec<u8>>,
    capabilities: Option<ServerCapabilities>,
    server_info: Option<ServerInfo>,
    position_encoding: PositionEncodingKind,
    /// Held to keep the child reaped on drop. `Option` so `with_streams`
    /// (test path) constructs without a process.
    _child: Option<Child>,
    /// Joining these tasks isn't strictly necessary — they exit when the
    /// pipes close — but holding the handles keeps Drop semantics explicit.
    _reader: tokio::task::JoinHandle<()>,
    _writer: tokio::task::JoinHandle<()>,
}

impl LspClient {
    /// Spawn `command` as a child process and connect to it.
    /// `notifications` receives every server-originated notification.
    pub async fn spawn(
        mut command: Command,
        notifications: mpsc::UnboundedSender<ClientNotification>,
    ) -> Result<Self> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().context("spawning LSP server")?;
        let stdin = child.stdin.take().context("LSP server stdin missing")?;
        let stdout = child.stdout.take().context("LSP server stdout missing")?;
        let mut me = Self::with_streams(stdout, stdin, notifications);
        me._child = Some(child);
        Ok(me)
    }

    /// Construct from arbitrary async read/write streams. Test path; in
    /// production use `spawn`.
    pub fn with_streams<R, W>(
        rd: R,
        wr: W,
        notifications: mpsc::UnboundedSender<ClientNotification>,
    ) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();

        let mut wr = wr;
        let writer_handle = tokio::spawn(async move {
            while let Some(bytes) = outbound_rx.recv().await {
                if write_frame(&mut wr, &bytes).await.is_err() {
                    break;
                }
            }
        });

        let pending_for_reader = Arc::clone(&pending);
        // Reader needs a writer handle so it can answer server→client
        // requests with method-not-found. Cloning is fine; mpsc senders are
        // cheap and the reader's clone dies with the reader task.
        let outbound_for_reader = outbound_tx.clone();
        let reader_handle = tokio::spawn(async move {
            let mut reader = FrameReader::new(rd);
            loop {
                let frame = match reader.read_frame().await {
                    Ok(Some(bytes)) => bytes,
                    Ok(None) | Err(_) => break,
                };
                let msg: ServerMessage = match serde_json::from_slice(&frame) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                match msg.kind() {
                    ServerMessageKind::Response => {
                        let Some(id) = msg.id else { continue };
                        let waiter = pending_for_reader.lock().unwrap().remove(&id);
                        if let Some(tx) = waiter {
                            let outcome = match (msg.result, msg.error) {
                                (_, Some(e)) => Err(e),
                                (Some(v), None) => Ok(v),
                                (None, None) => Ok(Value::Null),
                            };
                            let _ = tx.send(outcome);
                        }
                    }
                    ServerMessageKind::Notification => {
                        if let Some(method) = msg.method {
                            let _ = notifications.send(ClientNotification {
                                method,
                                params: msg.params.unwrap_or(Value::Null),
                            });
                        }
                    }
                    ServerMessageKind::ServerRequest => {
                        if let Some(id) = msg.id {
                            let resp = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "error": {
                                    "code": -32601,
                                    "message": "method not found",
                                },
                            });
                            if let Ok(bytes) = serde_json::to_vec(&resp) {
                                let _ = outbound_for_reader.send(bytes);
                            }
                        }
                    }
                    ServerMessageKind::Malformed => {}
                }
            }
        });

        Self {
            next_id: AtomicI64::new(1),
            pending,
            outbound_tx,
            capabilities: None,
            server_info: None,
            position_encoding: PositionEncodingKind::UTF16,
            _child: None,
            _reader: reader_handle,
            _writer: writer_handle,
        }
    }

    /// Issue a typed request. Resolves when the server's response arrives.
    pub async fn request<R: lsp_types::request::Request>(
        &self,
        params: R::Params,
    ) -> Result<R::Result> {
        let id = RequestId::Number(self.next_id.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);

        let req = Request {
            jsonrpc: "2.0",
            id,
            method: R::METHOD,
            params: serde_json::to_value(params).context("serializing request params")?,
        };
        let bytes = serde_json::to_vec(&req).context("serializing request envelope")?;
        self.outbound_tx
            .send(bytes)
            .map_err(|_| anyhow!("LSP writer task closed"))?;

        let outcome = rx
            .await
            .map_err(|_| anyhow!("LSP reader closed before response"))?;
        match outcome {
            Ok(v) => serde_json::from_value(v).context("deserializing response result"),
            Err(e) => Err(anyhow!("LSP server error {}: {}", e.code, e.message)),
        }
    }

    /// Fire a typed notification (no response expected).
    pub fn notify<N: lsp_types::notification::Notification>(
        &self,
        params: N::Params,
    ) -> Result<()> {
        let n = Notification {
            jsonrpc: "2.0",
            method: N::METHOD,
            params: serde_json::to_value(params).context("serializing notification params")?,
        };
        let bytes = serde_json::to_vec(&n).context("serializing notification envelope")?;
        self.outbound_tx
            .send(bytes)
            .map_err(|_| anyhow!("LSP writer task closed"))?;
        Ok(())
    }

    /// Initialize handshake: advertises capabilities (including utf-8
    /// PositionEncoding preference), stores the response, and sends the
    /// `initialized` notification.
    pub async fn initialize(
        &mut self,
        root_uri: Option<Uri>,
        workspace_folders: Vec<WorkspaceFolder>,
    ) -> Result<()> {
        // `root_uri` is deprecated in favor of `workspace_folders` since 3.6,
        // but older or single-root-only servers still consult it. Send both
        // when we have a root.
        #[allow(deprecated)]
        let params = InitializeParams {
            process_id: Some(std::process::id()),
            capabilities: build_client_capabilities(),
            workspace_folders: (!workspace_folders.is_empty()).then_some(workspace_folders),
            root_uri,
            client_info: Some(ClientInfo {
                name: "devix".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            ..Default::default()
        };
        let result: InitializeResult = self.request::<lsp_types::request::Initialize>(params).await?;
        self.position_encoding = result
            .capabilities
            .position_encoding
            .clone()
            .unwrap_or(PositionEncodingKind::UTF16);
        self.capabilities = Some(result.capabilities);
        self.server_info = result.server_info;
        self.notify::<lsp_types::notification::Initialized>(InitializedParams {})?;
        Ok(())
    }

    pub fn capabilities(&self) -> Option<&ServerCapabilities> {
        self.capabilities.as_ref()
    }

    pub fn server_info(&self) -> Option<&ServerInfo> {
        self.server_info.as_ref()
    }

    pub fn position_encoding(&self) -> &PositionEncodingKind {
        &self.position_encoding
    }
}

fn build_client_capabilities() -> ClientCapabilities {
    ClientCapabilities {
        general: Some(GeneralClientCapabilities {
            position_encodings: Some(vec![
                PositionEncodingKind::UTF8,
                PositionEncodingKind::UTF16,
            ]),
            ..Default::default()
        }),
        text_document: Some(TextDocumentClientCapabilities {
            synchronization: Some(TextDocumentSyncClientCapabilities {
                dynamic_registration: Some(false),
                will_save: Some(false),
                will_save_wait_until: Some(false),
                did_save: Some(false),
            }),
            publish_diagnostics: Some(PublishDiagnosticsClientCapabilities {
                related_information: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        }),
        workspace: Some(WorkspaceClientCapabilities {
            workspace_folders: Some(false),
            configuration: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use tokio::io::{AsyncRead, AsyncWrite, duplex, split};

    /// Stand up a connected client + a fake server. Returns the client and
    /// a `FakeServer` handle the test drives manually.
    fn pair() -> (LspClient, FakeServer, mpsc::UnboundedReceiver<ClientNotification>) {
        let (client_side, server_side) = duplex(8192);
        let (client_rd, client_wr) = split(client_side);
        let (server_rd, server_wr) = split(server_side);
        let (notif_tx, notif_rx) = mpsc::unbounded_channel();
        let client = LspClient::with_streams(client_rd, client_wr, notif_tx);
        let server = FakeServer {
            reader: FrameReader::new(server_rd),
            writer: Box::new(server_wr),
        };
        (client, server, notif_rx)
    }

    struct FakeServer {
        reader: FrameReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
        writer: Box<dyn AsyncWrite + Unpin + Send>,
    }

    impl FakeServer {
        async fn recv(&mut self) -> ServerMessage {
            let frame = self
                .reader
                .read_frame()
                .await
                .expect("frame read")
                .expect("frame present");
            serde_json::from_slice(&frame).expect("valid JSON")
        }

        async fn respond(&mut self, id: RequestId, result: Value) {
            let resp = serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result});
            let bytes = serde_json::to_vec(&resp).unwrap();
            write_frame(&mut self.writer, &bytes).await.unwrap();
        }

        async fn respond_err(&mut self, id: RequestId, code: i64, message: &str) {
            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": code, "message": message},
            });
            let bytes = serde_json::to_vec(&resp).unwrap();
            write_frame(&mut self.writer, &bytes).await.unwrap();
        }

        async fn notify(&mut self, method: &str, params: Value) {
            let n = serde_json::json!({"jsonrpc": "2.0", "method": method, "params": params});
            let bytes = serde_json::to_vec(&n).unwrap();
            write_frame(&mut self.writer, &bytes).await.unwrap();
        }

        async fn server_request(&mut self, id: i64, method: &str, params: Value) {
            let r = serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
            let bytes = serde_json::to_vec(&r).unwrap();
            write_frame(&mut self.writer, &bytes).await.unwrap();
        }
    }

    // Suppress unused-trait-bound warnings on the helper.
    fn _assert_async<R: AsyncRead + Unpin + Send>(_: R) {}

    #[tokio::test]
    async fn request_response_roundtrip() {
        let (client, mut server, _notif) = pair();
        let req_fut = tokio::spawn(async move {
            client
                .request::<lsp_types::request::Shutdown>(())
                .await
                .unwrap();
        });
        let msg = server.recv().await;
        assert_eq!(msg.method.as_deref(), Some("shutdown"));
        let id = msg.id.expect("request has id");
        server.respond(id, Value::Null).await;
        req_fut.await.unwrap();
    }

    #[tokio::test]
    async fn notifications_forward_to_channel() {
        let (_client, mut server, mut notif) = pair();
        server
            .notify(
                "textDocument/publishDiagnostics",
                serde_json::json!({"uri": "file:///x", "diagnostics": []}),
            )
            .await;
        let n = notif.recv().await.expect("notification");
        assert_eq!(n.method, "textDocument/publishDiagnostics");
        assert_eq!(n.params["uri"], "file:///x");
    }

    #[tokio::test]
    async fn out_of_order_responses_demux_correctly() {
        let (client, mut server, _notif) = pair();
        let client = Arc::new(client);
        let c1 = Arc::clone(&client);
        let c2 = Arc::clone(&client);
        let h1 = tokio::spawn(async move {
            c1.request::<lsp_types::request::Shutdown>(()).await.unwrap();
        });
        let h2 = tokio::spawn(async move {
            c2.request::<lsp_types::request::Shutdown>(()).await.unwrap();
        });
        let m1 = server.recv().await;
        let m2 = server.recv().await;
        let id1 = m1.id.unwrap();
        let id2 = m2.id.unwrap();
        // Respond to the second request first.
        server.respond(id2, Value::Null).await;
        server.respond(id1, Value::Null).await;
        h1.await.unwrap();
        h2.await.unwrap();
    }

    #[tokio::test]
    async fn server_error_response_propagates() {
        let (client, mut server, _notif) = pair();
        let req_fut = tokio::spawn(async move {
            client
                .request::<lsp_types::request::Shutdown>(())
                .await
                .expect_err("server returned error")
        });
        let msg = server.recv().await;
        server
            .respond_err(msg.id.unwrap(), -32000, "server unhappy")
            .await;
        let err = req_fut.await.unwrap();
        let s = format!("{err}");
        assert!(s.contains("-32000"));
        assert!(s.contains("server unhappy"));
    }

    #[tokio::test]
    async fn server_request_gets_method_not_found() {
        let (_client, mut server, _notif) = pair();
        // Server sends a request; client's reader auto-responds with -32601.
        server
            .server_request(7, "client/registerCapability", serde_json::json!({}))
            .await;
        let reply = server.recv().await;
        assert_eq!(reply.id, Some(RequestId::Number(7)));
        let err = reply.error.expect("error present");
        assert_eq!(err.code, -32601);
    }

    #[tokio::test]
    async fn initialize_negotiates_utf8_when_offered() {
        let (mut client, mut server, _notif) = pair();
        let init_fut = tokio::spawn(async move {
            let uri = Uri::from_str("file:///workspace").ok();
            client.initialize(uri, vec![]).await.unwrap();
            client
        });
        let msg = server.recv().await;
        assert_eq!(msg.method.as_deref(), Some("initialize"));
        // Verify the client advertised utf-8 first.
        let pos_encs = &msg.params.as_ref().unwrap()["capabilities"]["general"]["positionEncodings"];
        assert_eq!(pos_encs[0], "utf-8");
        // Server picks utf-8 in its reply.
        server
            .respond(
                msg.id.unwrap(),
                serde_json::json!({
                    "capabilities": {
                        "positionEncoding": "utf-8",
                    },
                    "serverInfo": {"name": "fake-server", "version": "0.0"},
                }),
            )
            .await;
        // Client should follow up with the `initialized` notification.
        let initialized = server.recv().await;
        assert_eq!(initialized.method.as_deref(), Some("initialized"));
        let client = init_fut.await.unwrap();
        assert_eq!(client.position_encoding(), &PositionEncodingKind::UTF8);
        assert_eq!(client.server_info().unwrap().name, "fake-server");
    }

    #[tokio::test]
    async fn initialize_falls_back_to_utf16_when_not_negotiated() {
        let (mut client, mut server, _notif) = pair();
        let init_fut = tokio::spawn(async move {
            client.initialize(None, vec![]).await.unwrap();
            client
        });
        let msg = server.recv().await;
        // Server omits positionEncoding entirely.
        server
            .respond(
                msg.id.unwrap(),
                serde_json::json!({"capabilities": {}}),
            )
            .await;
        let _initialized = server.recv().await;
        let client = init_fut.await.unwrap();
        assert_eq!(client.position_encoding(), &PositionEncodingKind::UTF16);
    }
}
