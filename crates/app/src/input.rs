//! Terminal input thread.
//!
//! `crossterm::event::read()` has no push API and no interrupt, so this
//! is the one place in the runtime that genuinely owns a poll thread.
//! `InputThread::spawn` starts it; `shutdown` flips an atomic flag the
//! poll loop checks each iteration and joins, with a deadline so we
//! don't hang on a wedged terminal.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event;

use crate::event_sink::EventSink;

const POLL_TIMEOUT: Duration = Duration::from_millis(100);

pub(crate) struct InputThread {
    join: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

impl InputThread {
    pub(crate) fn spawn(sink: EventSink) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let join = thread::Builder::new()
            .name("devix-input".into())
            .spawn(move || {
                while !stop_clone.load(Ordering::Acquire) {
                    match event::poll(POLL_TIMEOUT) {
                        Ok(true) => match event::read() {
                            Ok(ev) => {
                                if sink.input(ev).is_err() {
                                    return;
                                }
                            }
                            Err(_) => return,
                        },
                        Ok(false) => continue,
                        Err(_) => return,
                    }
                }
            })
            .context("spawning devix-input thread")?;
        Ok(Self { join: Some(join), stop })
    }

    /// Signal the thread to stop and join with a deadline. The thread
    /// observes `stop` within `POLL_TIMEOUT` once the flag is set.
    pub(crate) fn shutdown(mut self, deadline: Duration) {
        self.stop.store(true, Ordering::Release);
        let Some(join) = self.join.take() else { return };
        let start = Instant::now();
        while !join.is_finished() {
            if start.elapsed() >= deadline {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = join.join();
    }
}
