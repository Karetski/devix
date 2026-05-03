//! LSP client (JSON-RPC over child stdio). Phase 6.
//!
//! Layering:
//! - [`framing`]   — Content-Length framed reads/writes over an async stream.
//! - [`jsonrpc`]   — request/response/notification JSON shapes.
//!
//! `LspClient`, the per-(root, language) coordinator, and the App-side
//! drain land in subsequent commits on top of this surface.

pub mod framing;
pub mod jsonrpc;

pub use framing::{FrameReader, write_frame};
pub use jsonrpc::{Notification, Request, RequestId, ResponseError, ServerMessage, ServerMessageKind};
