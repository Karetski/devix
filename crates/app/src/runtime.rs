//! Application runtime: terminal lifecycle, event multiplex, and the
//! lifecycle protocol the binary plugs into.
//!
//! Modeled on UIKit's split between `UIApplication` (the runtime that
//! owns the run loop and event delivery) and `UIApplicationDelegate`
//! (the protocol the embedding app implements to plug into lifecycle
//! moments).
//!
//! - [`Application`] owns the terminal, the input thread, and the loop.
//! - [`ApplicationDelegate`] is what a concrete app implements; see
//!   [`crate::app::App`].
//! - [`Waker`] is the cross-thread wake signal: any thread (plugin
//!   worker, watcher, async task) calls it to wake the loop within an
//!   iteration without polling.

use std::io::stdout;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};

const MAX_DRAIN_PER_TICK: usize = 256;
/// Maximum time the loop blocks while idle. Long enough that anything the
/// delegate polls in `tick` doesn't burn CPU; short enough that those
/// subsystems' own latency stays sub-perceptible. Wakers feed the same
/// channel as input, so an explicit signal still wakes immediately.
const IDLE_TIMEOUT: Duration = Duration::from_millis(100);

/// Cross-thread wake signal. Cheap to clone; calling it pushes a wakeup
/// into the run loop's channel so the next `recv_timeout` returns
/// without waiting out the idle window.
pub type Waker = Arc<dyn Fn() + Send + Sync + 'static>;

/// Lifecycle protocol implemented by the concrete app. The runtime
/// calls these on the main thread, in this order, every iteration.
pub trait ApplicationDelegate {
    /// Pre-render pulse. Drain external sources (filesystem watchers,
    /// plugin events, async results) and apply any deferred state here.
    fn tick(&mut self) {}

    /// Handle one terminal input event (key, mouse, resize, …).
    fn on_input(&mut self, event: Event);

    /// Paint the current frame. Called only when [`take_dirty`] returns
    /// true.
    fn render(&mut self, frame: &mut Frame<'_>);

    /// Read-and-clear the redraw flag. The runtime calls this once per
    /// iteration to decide whether to repaint.
    fn take_dirty(&mut self) -> bool;

    /// Exit predicate. Checked at the top of each iteration; returning
    /// true ends the loop.
    fn should_terminate(&self) -> bool;

    /// Final hook before the terminal is restored. The delegate is
    /// dropped after this returns.
    fn will_terminate(&mut self) {}
}

/// The runtime. Construct it before the delegate so the delegate can
/// capture a [`Waker`] at construction time, then hand ownership to
/// [`Application::run`].
pub struct Application {
    tx: SyncSender<LoopEvent>,
    rx: Receiver<LoopEvent>,
}

impl Application {
    pub fn new() -> Self {
        let (tx, rx) = sync_channel::<LoopEvent>(1024);
        Self { tx, rx }
    }

    /// Cross-thread wake handle. Cheap to clone; safe to hand to plugins,
    /// watchers, or any background task that needs to nudge the loop.
    pub fn waker(&self) -> Waker {
        let tx = self.tx.clone();
        Arc::new(move || {
            let _ = tx.try_send(LoopEvent::Wakeup);
        })
    }

    pub fn run<D: ApplicationDelegate>(self, mut delegate: D) -> Result<()> {
        let _tty = Tty::enter()?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        spawn_input_thread(self.tx.clone())?;
        drop(self.tx);

        let result = run_loop(&mut delegate, &mut terminal, &self.rx);

        delegate.will_terminate();
        let _ = terminal.show_cursor();
        result
    }
}

impl Default for Application {
    fn default() -> Self {
        Self::new()
    }
}

fn run_loop<D, B>(
    delegate: &mut D,
    terminal: &mut Terminal<B>,
    rx: &Receiver<LoopEvent>,
) -> Result<()>
where
    D: ApplicationDelegate,
    B: Backend,
{
    while !delegate.should_terminate() {
        delegate.tick();

        if delegate.take_dirty() {
            terminal.draw(|frame| delegate.render(frame))?;
        }

        match rx.recv_timeout(IDLE_TIMEOUT) {
            Ok(LoopEvent::Input(ev)) => delegate.on_input(ev),
            Ok(LoopEvent::Wakeup) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        let mut drained = 0;
        while drained < MAX_DRAIN_PER_TICK && !delegate.should_terminate() {
            match rx.try_recv() {
                Ok(LoopEvent::Input(ev)) => delegate.on_input(ev),
                Ok(LoopEvent::Wakeup) => {}
                Err(_) => break,
            }
            drained += 1;
        }
    }
    Ok(())
}

/// One thing the run loop wakes up to handle. Input from the terminal
/// thread and explicit `Waker` calls multiplex through this so a single
/// `recv_timeout` covers all wakeup sources.
enum LoopEvent {
    Input(Event),
    Wakeup,
}

fn spawn_input_thread(tx: SyncSender<LoopEvent>) -> Result<()> {
    std::thread::Builder::new()
        .name("devix-input".into())
        .spawn(move || loop {
            match event::read() {
                Ok(ev) => {
                    if tx.send(LoopEvent::Input(ev)).is_err() {
                        return;
                    }
                }
                Err(_) => return,
            }
        })?;
    Ok(())
}

/// Raw-mode terminal lifecycle guard. Enters the alternate screen with
/// mouse capture on construction; restores the terminal on drop and via
/// the panic hook so a crash leaves the user in a usable shell.
struct Tty;

impl Tty {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen, EnableMouseCapture)?;

        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = restore();
            prev(info);
        }));

        Ok(Self)
    }
}

impl Drop for Tty {
    fn drop(&mut self) {
        let _ = restore();
    }
}

fn restore() -> Result<()> {
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    Ok(())
}
