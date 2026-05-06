//! Plugin host as a `Service`.
//!
//! Pure runtime-ownership: the plugin's tokio worker thread (started
//! inside `PluginRuntime::load_with_sink`) pushes `PluginEmitted` pulses
//! directly via the `MsgSink` it was given at load time. This service
//! exists only to keep the runtime alive for the application's
//! lifetime; on `stop()`, dropping the runtime closes its channels and
//! its worker thread exits.

use std::time::Duration;

use anyhow::Result;
use devix_plugin::PluginRuntime;

use crate::event_sink::EventSink;
use crate::service::Service;

pub struct PluginService {
    runtime: Option<PluginRuntime>,
}

impl PluginService {
    pub fn new(runtime: PluginRuntime) -> Self {
        Self { runtime: Some(runtime) }
    }
}

impl Service for PluginService {
    fn name(&self) -> &'static str {
        "plugin"
    }

    fn start(&mut self, _sink: EventSink) -> Result<()> {
        // The runtime's `MsgSink` was wired at load time; nothing to do
        // here. Holding the runtime in `self.runtime` keeps its worker
        // thread alive until `stop()` drops it.
        Ok(())
    }

    fn stop(self: Box<Self>, _deadline: Duration) {
        // Dropping `runtime` closes its `invoke_tx` / `input_tx`, which
        // ends the worker's `tokio::select!` loop. The join handle the
        // runtime owns is detached; the OS reaps the thread.
        drop(self.runtime);
    }
}
