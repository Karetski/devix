//! System-clipboard abstraction.
//!
//! `core` defines the trait so model-layer crates (`surface`, `commands`)
//! can call into the clipboard without depending on a concrete backend
//! (`arboard`, X11, AppKit, …). The binary picks an impl and passes
//! `&mut dyn Clipboard` through the dispatcher context.

/// Read/write text to the system clipboard. Failures are reported as
/// `false` / `None` — the only sensible responses in an editor where
/// clipboard access is best-effort (no display server, sandbox denial,
/// missing permission).
pub trait Clipboard {
    /// Attempt to write `text` to the clipboard. Returns `true` on
    /// success, `false` if the backend declined.
    fn set_text(&mut self, text: String) -> bool;

    /// Read the current clipboard text, if any.
    fn get_text(&mut self) -> Option<String>;
}

/// Clipboard implementation that always fails. Used in headless
/// environments and tests where no display server is available.
pub struct NoClipboard;

impl Clipboard for NoClipboard {
    fn set_text(&mut self, _: String) -> bool {
        false
    }
    fn get_text(&mut self) -> Option<String> {
        None
    }
}
