//! System-clipboard initialization. Errors are swallowed: when no display
//! server is available, copy/cut/paste operate without a clipboard backend
//! (the Action handlers set status accordingly).

pub fn init() -> Option<arboard::Clipboard> {
    arboard::Clipboard::new().ok()
}
