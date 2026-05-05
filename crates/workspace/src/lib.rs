//! Workspace state: the documents the editor has open.
//!
//! `Document` is the model of one open file (rope buffer + history +
//! filesystem watcher + syntax tree). It used to live in `devix-editor`,
//! but editor's purpose is panes/rendering — keeping the document model
//! in its own crate lets renderless consumers (`devix-commands`,
//! `devix-plugin`, file watchers) depend on it without dragging in
//! `devix-ui` or `ratatui`.

pub mod document;

pub use document::{DocId, Document};
