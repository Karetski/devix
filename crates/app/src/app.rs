//! Editor application: owns terminal-side state and runs the event loop.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{RecvTimeoutError, SyncSender, sync_channel};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event};
use ratatui::Terminal;
use ratatui::backend::Backend;
use devix_surface::{CommandRegistry, Keymap, build_registry, default_keymap};
use devix_core::{Clipboard, Theme};
use devix_surface::Surface;

use crate::clipboard;
use crate::events::{handle_event, run_command};
use crate::plugin::{PluginWiring, drain_plugin_events, try_load as try_load_plugin};
use crate::render::render;
use crate::watcher::drain_disk_events;

pub struct App {
    pub surface: Surface,
    pub commands: CommandRegistry,
    pub keymap: Keymap,
    pub theme: Theme,
    pub quit: bool,
    pub clipboard: Box<dyn Clipboard>,
    dirty: bool,
    pub pending_scroll: isize,
    /// Lua plugin runtime + the contributions it registered. `None` when
    /// no `DEVIX_PLUGIN` was set or loading failed; the editor still runs
    /// without plugin integration in that case.
    pub plugins: Option<PluginWiring>,
}

const MAX_DRAIN_PER_TICK: usize = 256;
/// Maximum time `recv_timeout` blocks while idle. Long enough that
/// disk-watcher polling at the top of the loop doesn't burn CPU; short
/// enough that those subsystems' own latency stays under
/// human-perceptible thresholds. Plugin and input events wake the loop
/// immediately via the unified mpsc, so this isn't a responsiveness
/// floor for them.
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
    /// Mark the next frame for redraw. Called from any code path that
    /// changes user-visible state (input, plugin events, disk watcher).
    /// The render driver consumes the flag once per frame via `take_dirty`.
    pub fn request_redraw(&mut self) {
        self.dirty = true;
    }

    /// Read-and-clear the dirty flag. Owned by the render driver in `run`.
    pub(crate) fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    pub fn new(path: Option<PathBuf>, plugin_wakeup: Option<devix_plugin::Wakeup>) -> Result<Self> {
        let surface = Surface::open(path)?;
        let clipboard = clipboard::init();

        let mut commands = build_registry();
        let mut keymap = default_keymap();
        let plugins = try_load_plugin(&mut commands, &mut keymap, plugin_wakeup);

        // Auto-open any sidebar slot the plugin contributed content to,
        // so the user sees plugin output on first frame instead of
        // having to discover Ctrl+B / Ctrl+Alt+B first.
        let mut surface = surface;
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
            quit: false,
            clipboard,
            dirty: true,
            pending_scroll: 0,
            plugins,
        })
    }
}

pub fn run<B: Backend>(terminal: &mut Terminal<B>, path: Option<PathBuf>) -> Result<()> {
    let (tx, rx) = sync_channel::<MainEvent>(1024);

    let tx_for_plugin = tx.clone();
    let plugin_wakeup: devix_plugin::Wakeup = Arc::new(move || {
        let _ = tx_for_plugin.try_send(MainEvent::PluginActivity);
    });

    let mut app = App::new(path, Some(plugin_wakeup))?;

    spawn_input_thread(tx.clone())?;
    drop(tx);

    while !app.quit {
        drain_disk_events(&mut app);
        drain_plugin_events(&mut app);

        if app.take_dirty() {
            terminal.draw(|frame| render(frame, &mut app))?;
        }

        match rx.recv_timeout(IDLE_TIMEOUT) {
            Ok(MainEvent::Input(ev)) => handle_event(ev, &mut app),
            Ok(MainEvent::PluginActivity) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

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
