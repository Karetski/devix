//! Document = Buffer + filesystem-watcher attachment, owned by Workspace.

use std::path::PathBuf;

use anyhow::Result;
use devix_buffer::Buffer;
use slotmap::new_key_type;

new_key_type! { pub struct DocId; }

pub struct Document {
    pub buffer: Buffer,
    pub watcher: Option<notify::RecommendedWatcher>,
    pub disk_changed_pending: bool,
}

impl Document {
    pub fn from_buffer(buffer: Buffer) -> Self {
        Self { buffer, watcher: None, disk_changed_pending: false }
    }

    pub fn from_path(path: PathBuf) -> Result<Self> {
        Ok(Self::from_buffer(Buffer::from_path(path)?))
    }

    pub fn empty() -> Self {
        Self::from_buffer(Buffer::empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_document_has_no_path_and_no_watcher() {
        let d = Document::empty();
        assert!(d.buffer.path().is_none());
        assert!(d.watcher.is_none());
        assert!(!d.disk_changed_pending);
        assert!(!d.buffer.dirty());
    }
}
