//! System-clipboard binding for the binary.
//!
//! `core` defines the `Clipboard` trait so the model layer (`editor`,
//! `commands`) can call into the clipboard without depending on a concrete
//! backend. The binary picks `arboard` and bridges it here.

use devix_panes::{Clipboard, NoClipboard};

struct ArboardClipboard(arboard::Clipboard);

impl Clipboard for ArboardClipboard {
    fn set_text(&mut self, text: String) -> bool {
        self.0.set_text(text).is_ok()
    }
    fn get_text(&mut self) -> Option<String> {
        self.0.get_text().ok()
    }
}

/// Initialize the system clipboard backend. Falls back to `NoClipboard`
/// when no display server is available — copy/cut/paste still dispatch
/// but become no-ops (matching the prior `Option<Clipboard>` behavior).
pub fn init() -> Box<dyn Clipboard> {
    match arboard::Clipboard::new() {
        Ok(cb) => Box::new(ArboardClipboard(cb)),
        Err(_) => Box::new(NoClipboard),
    }
}
