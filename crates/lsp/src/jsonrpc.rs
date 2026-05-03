//! JSON-RPC 2.0 message shapes used by LSP.
//!
//! We don't need a full JSON-RPC implementation — LSP only uses a subset:
//! requests (with id), responses (with id), and notifications (no id).
//! Outgoing requests are typed by `lsp-types`; we serialize their `params`
//! into `serde_json::Value` here so the framing layer doesn't depend on the
//! full LSP type set.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    // LSP allows string ids too; we never emit them but a server might echo one
    // back (rare). Storing as untagged keeps the Deserialize lenient.
    // String variant intentionally omitted for now — add when we observe it.
}

#[derive(Debug, Serialize)]
pub struct Request<'a> {
    pub jsonrpc: &'static str,
    pub id: RequestId,
    pub method: &'a str,
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct Notification<'a> {
    pub jsonrpc: &'static str,
    pub method: &'a str,
    pub params: Value,
}

/// A message read from the server. LSP servers send three kinds:
/// - responses (have `id`, no `method`)
/// - notifications (no `id`, have `method`)
/// - requests *to the client* (have both `id` and `method`) — rare, we ignore
///   most of them in v1 (e.g. window/showMessage we'll log; workspace/configuration
///   we'll respond with empty).
#[derive(Debug, Deserialize)]
pub struct ServerMessage {
    #[allow(dead_code)]
    pub jsonrpc: Option<String>,
    pub id: Option<RequestId>,
    pub method: Option<String>,
    pub params: Option<Value>,
    pub result: Option<Value>,
    pub error: Option<ResponseError>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ResponseError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl ServerMessage {
    pub fn kind(&self) -> ServerMessageKind {
        match (&self.id, &self.method) {
            (Some(_), Some(_)) => ServerMessageKind::ServerRequest,
            (Some(_), None) => ServerMessageKind::Response,
            (None, Some(_)) => ServerMessageKind::Notification,
            (None, None) => ServerMessageKind::Malformed,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ServerMessageKind {
    Response,
    Notification,
    ServerRequest,
    Malformed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_response() {
        let raw = br#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}"#;
        let m: ServerMessage = serde_json::from_slice(raw).unwrap();
        assert_eq!(m.kind(), ServerMessageKind::Response);
        assert_eq!(m.id, Some(RequestId::Number(1)));
        assert!(m.result.is_some());
    }

    #[test]
    fn parse_notification() {
        let raw = br#"{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":"file:///x","diagnostics":[]}}"#;
        let m: ServerMessage = serde_json::from_slice(raw).unwrap();
        assert_eq!(m.kind(), ServerMessageKind::Notification);
        assert_eq!(m.method.as_deref(), Some("textDocument/publishDiagnostics"));
    }

    #[test]
    fn parse_server_request() {
        let raw = br#"{"jsonrpc":"2.0","id":2,"method":"workspace/configuration","params":{"items":[]}}"#;
        let m: ServerMessage = serde_json::from_slice(raw).unwrap();
        assert_eq!(m.kind(), ServerMessageKind::ServerRequest);
    }

    #[test]
    fn parse_error_response() {
        let raw = br#"{"jsonrpc":"2.0","id":3,"error":{"code":-32601,"message":"method not found"}}"#;
        let m: ServerMessage = serde_json::from_slice(raw).unwrap();
        assert_eq!(m.kind(), ServerMessageKind::Response);
        assert_eq!(m.error.unwrap().code, -32601);
    }

    #[test]
    fn serialize_request() {
        let r = Request {
            jsonrpc: "2.0",
            id: RequestId::Number(42),
            method: "initialize",
            params: serde_json::json!({"processId": null}),
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains(r#""id":42"#));
        assert!(s.contains(r#""method":"initialize""#));
    }
}
