//! Built-in services bundled with the binary.
//!
//! Only `InputService` actually owns a thread: terminal input has no
//! push API, so we read in a background thread and pulse via
//! `EventSink::input`. Disk events arrive directly from `notify`'s own
//! thread (via the editor's `disk_sink` callback) and plugin messages
//! arrive directly from the plugin worker thread (via the runtime's
//! `MsgSink`); neither needs a service of its own to poll.

pub mod input;
pub mod plugin;
