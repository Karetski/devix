//! LSP client (JSON-RPC over child stdio). Phase 6.
//!
//! Layering:
//! - [`framing`]   — Content-Length framed reads/writes over an async stream.
//! - [`jsonrpc`]   — request/response/notification JSON shapes.
//! - [`client`]    — `LspClient`: child spawn, reader/writer tasks, request
//!   correlation, initialize handshake with utf-8 PositionEncoding negotiation.
//!
//! Per-(root, language) coordination and the App-side drain land in later
//! commits on top of this surface.

pub mod client;
pub mod framing;
pub mod jsonrpc;

pub use client::{ClientNotification, LspClient};
pub use framing::{FrameReader, write_frame};
pub use jsonrpc::{Notification, Request, RequestId, ResponseError, ServerMessage, ServerMessageKind};
