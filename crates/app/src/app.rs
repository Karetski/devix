//! Editor application: owns terminal-side state and runs the event loop.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{RecvTimeoutError, SyncSender, sync_channel};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event};
use ratatui::Terminal;
use ratatui::backend::Backend;
use devix_core::Theme;
use devix_surface::{
    CommandRegistry, Keymap, StatusLine, Surface, build_registry, default_keymap,
};

use crate::clipboard;
use crate::events::{handle_event, run_command};
use crate::lsp::{LspWiring, drain_lsp_events, setup_lsp};
use crate::plugin::{PluginWiring, drain_plugin_events, try_load as try_load_plugin};
use crate::render::render;
use crate::watcher::drain_disk_events;

pub struct App {
    pub surface: Surface,
    pub commands: CommandRegistry,
    pub keymap: Keymap,
    pub theme: Theme,
    pub status: StatusLine,
    pub quit: bool,
    pub clipboard: Option<arboard::Clipboard>,
    pub dirty: bool,
    pub pending_scroll: isize,
    /// LSP runtime + inbound event channel. `None` when LSP setup failed at
    /// startup (e.g. the runtime couldn't be built); the editor still runs
    /// without LSP integration in that case.
    pub lsp: Option<LspWiring>,
    /// Lua plugin runtime + the contributions it registered. `None` when
    /// no `DEVIX_PLUGIN` was set or loading failed; the editor still runs
    /// without plugin integration in that case.
    pub plugins: Option<PluginWiring>,
}

const MAX_DRAIN_PER_TICK: usize = 256;
/// Maximum time `recv_timeout` blocks while idle. Long enough that
/// LSP / disk-watcher polling at the top of the loop doesn't burn
/// CPU; short enough that those subsystems' own latency stays under
/// human-perceptible thresholds. Plugin and input events wake the
/// loop immediately via the unified mpsc, so this isn't a
/// responsiveness floor for them.
const IDLE_TIMEOUT: Duration = Duration::from_millis(100);

/// One thing the main loop wakes up to handle. Both terminal input
/// and plugin activity multiplex through this so a single
/// `recv_timeout` covers all wakeup sources — no fixed-cadence
/// polling.
enum MainEvent {
    Input(Event),
    /// Plugin worker thread produced one or more `PluginMsg`s. The
    /// payload itself is drained at the top of the next loop
    /// iteration via `drain_plugin_events`; this enum variant is just
    /// the wakeup signal.
    PluginActivity,
}

impl App {
    /// Construct a new app, optionally with a plugin wakeup hook the
    /// runtime calls every time it pushes a message. `None` means
    /// "no wakeup" — the plugin's outbound channel is still drained
    /// at the top of every main-loop iteration, but the loop won't
    /// unblock specifically for plugin activity. Suitable for tests
    /// that don't run the main loop.
    pub fn new(path: Option<PathBuf>, plugin_wakeup: Option<devix_plugin::Wakeup>) -> Result<Self> {
        let mut surface = Surface::open(path)?;
        let clipboard = clipboard::init();

        // LSP setup is best-effort: if it fails we still launch the editor
        // without server integration rather than refusing to open a file.
        let lsp = match setup_lsp() {
            Ok((sink, encoding, wiring)) => {
                surface.attach_lsp(sink, encoding);
                Some(wiring)
            }
            Err(_) => None,
        };

        let mut commands = build_registry();
        let mut keymap = default_keymap();
        let mut status = StatusLine::default();
        let plugins = try_load_plugin(&mut commands, &mut keymap, &mut status, plugin_wakeup);

        // Auto-open any sidebar slot the plugin contributed content to,
        // so the user sees plugin output on first frame instead of
        // having to discover Ctrl+B / Ctrl+Alt+B first.
        if let Some(wiring) = plugins.as_ref() {
            for slot in wiring.contributed_slots() {
                surface.toggle_sidebar(slot);
            }
        }

        Ok(Self {
            surface,
            commands,
            keymap,
            theme: Theme::default(),
            status,
            quit: false,
            clipboard,
            dirty: true,
            pending_scroll: 0,
            lsp,
            plugins,
        })
    }
}

pub fn run<B: Backend>(terminal: &mut Terminal<B>, path: Option<PathBuf>) -> Result<()> {
    // Unified wakeup channel. The buffer is generous so a momentarily
    // busy plugin (e.g. typing fast in a pane that bumps scroll) never
    // blocks the worker on a full channel; if the editor is so far
    // behind that we ever fill 1024 slots, dropping additional pings
    // is fine — drain_plugin_events catches up at the next iteration.
    let (tx, rx) = sync_channel::<MainEvent>(1024);

    // Plugin → main: a no-op closure when the channel is full so the
    // plugin worker stays non-blocking. Captures a clone of the
    // sender; `try_send` only fails when the buffer is saturated or
    // the receiver hung up.
    let tx_for_plugin = tx.clone();
    let plugin_wakeup: devix_plugin::Wakeup = Arc::new(move || {
        let _ = tx_for_plugin.try_send(MainEvent::PluginActivity);
    });

    let mut app = App::new(path, Some(plugin_wakeup))?;

    // Input thread → main. `event::read()` blocks until the OS
    // delivers a key/mouse event, so this thread sits idle in a
    // syscall until something arrives — no polling.
    spawn_input_thread(tx.clone())?;
    drop(tx); // Only the spawned senders should keep the channel alive now.

    while !app.quit {
        drain_disk_events(&mut app);
        drain_lsp_events(&mut app);
        drain_plugin_events(&mut app);

        if app.dirty {
            terminal.draw(|frame| render(frame, &mut app))?;
            app.dirty = false;
        }

        match rx.recv_timeout(IDLE_TIMEOUT) {
            Ok(MainEvent::Input(ev)) => handle_event(ev, &mut app),
            Ok(MainEvent::PluginActivity) => {
                // No-op: drain happens at the top of the next iter.
            }
            Err(RecvTimeoutError::Timeout) => {
                // Periodic tick — let LSP / disk-watcher integrations
                // poll their state.
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }

        // Drain whatever else is queued without blocking. Coalescing
        // bursts of input here keeps the editor responsive when
        // someone holds an arrow key or pastes a chunk of text.
        let mut drained = 0;
        while drained < MAX_DRAIN_PER_TICK && !app.quit {
            match rx.try_recv() {
                Ok(MainEvent::Input(ev)) => handle_event(ev, &mut app),
                Ok(MainEvent::PluginActivity) => {}
                Err(_) => break,
            }
            drained += 1;
        }

        if app.pending_scroll != 0 {
            let delta = std::mem::take(&mut app.pending_scroll);
            run_command(&mut app, std::sync::Arc::new(devix_surface::cmd::ScrollBy(delta)));
        }
    }
    Ok(())
}

fn spawn_input_thread(tx: SyncSender<MainEvent>) -> Result<()> {
    std::thread::Builder::new()
        .name("devix-input".into())
        .spawn(move || {
            loop {
                match event::read() {
                    Ok(ev) => {
                        if tx.send(MainEvent::Input(ev)).is_err() {
                            return;
                        }
                    }
                    Err(_) => return,
                }
            }
        })?;
    Ok(())
}
