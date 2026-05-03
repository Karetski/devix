//! LSP client (JSON-RPC over child stdio). Phase 6.
//!
//! Layering:
//! - [`framing`]   — Content-Length framed reads/writes over an async stream.
//! - [`jsonrpc`]   — request/response/notification JSON shapes.
//! - [`client`]    — `LspClient`: child spawn, reader/writer tasks, request
//!   correlation, initialize handshake with utf-8 PositionEncoding negotiation.
//! - [`coord`]     — `Coordinator`: one client per (workspace_root, language),
//!   `DocChange` in / `LspEvent` out, lazy spawn via the `Spawner` trait.
//!
//! The App-side drain (Document sink wiring + diagnostics render) lives in
//! `devix` (the binary crate) on top of this surface.

pub mod client;
pub mod coord;
pub mod framing;
pub mod jsonrpc;

pub use client::{ClientNotification, LspClient};
pub use coord::{
    Coordinator, CoordinatorConfig, DocChange, LanguageConfig, LspEvent, Spawner,
    SubprocessSpawner, path_to_uri, uri_to_path,
};
pub use framing::{FrameReader, write_frame};
pub use jsonrpc::{Notification, Request, RequestId, ResponseError, ServerMessage, ServerMessageKind};
