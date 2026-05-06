//! Terminal input as a `Service`.
//!
//! Spawns a thread that calls `crossterm::event::poll(POLL_TIMEOUT)`,
//! then `read()` when ready, and pushes `LoopMessage::Input` via the
//! cloned `EventSink`. The poll timeout is what makes shutdown bounded:
//! `stop()` flips an atomic flag the loop checks each iteration; the
//! next poll-timeout returns and the thread exits.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event;

use crate::event_sink::EventSink;
use crate::service::Service;

const POLL_TIMEOUT: Duration = Duration::from_millis(100);

#[derive(Default)]
pub struct InputService {
    join: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

impl Service for InputService {
    fn name(&self) -> &'static str {
        "input"
    }

    fn start(&mut self, sink: EventSink) -> Result<()> {
        let stop = self.stop.clone();
        let join = thread::Builder::new()
            .name("devix-input".into())
            .spawn(move || {
                while !stop.load(Ordering::Acquire) {
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
        self.join = Some(join);
        Ok(())
    }

    fn stop(mut self: Box<Self>, deadline: Duration) {
        self.stop.store(true, Ordering::Release);
        let Some(join) = self.join.take() else { return };
        // Best-effort join with a deadline; the input thread exits within
        // POLL_TIMEOUT once the stop flag is observed.
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
