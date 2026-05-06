//! Concrete editor app: top-level state and the `ApplicationDelegate`
//! impl that plugs it into the runtime.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use crossterm::event::Event;
use ratatui::Frame;
use devix_editor::{CommandRegistry, Editor, Keymap, build_registry, default_keymap};
use devix_panes::{Clipboard, Theme};

use crate::clipboard;
use crate::events::{handle_event, run_command};
use crate::plugin::{PluginWiring, drain_plugin_events, try_load as try_load_plugin};
use crate::render::render;
use crate::runtime::ApplicationDelegate;
use crate::watcher::drain_disk_events;

pub struct App {
    pub editor: Editor,
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

impl App {
    /// Mark the next frame for redraw. Called from any code path that
    /// changes user-visible state (input, plugin events, disk watcher).
    /// The runtime consumes the flag once per iteration via `take_dirty`.
    pub fn request_redraw(&mut self) {
        self.dirty = true;
    }

    pub fn new(path: Option<PathBuf>, plugin_wakeup: Option<devix_plugin::Wakeup>) -> Result<Self> {
        let editor = Editor::open(path)?;
        let clipboard = clipboard::init();

        let mut commands = build_registry();
        let mut keymap = default_keymap();
        let plugins = try_load_plugin(&mut commands, &mut keymap, plugin_wakeup);

        // Auto-open any sidebar slot the plugin contributed content to,
        // so the user sees plugin output on first frame instead of
        // having to discover Ctrl+B / Ctrl+Alt+B first.
        let mut editor = editor;
        if let Some(wiring) = plugins.as_ref() {
            for slot in wiring.contributed_slots() {
                editor.toggle_sidebar(slot);
            }
        }

        Ok(Self {
            editor,
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

impl ApplicationDelegate for App {
    fn tick(&mut self) {
        drain_disk_events(self);
        drain_plugin_events(self);
        if self.pending_scroll != 0 {
            let delta = std::mem::take(&mut self.pending_scroll);
            run_command(self, Arc::new(devix_editor::cmd::ScrollBy(delta)));
        }
    }

    fn on_input(&mut self, event: Event) {
        handle_event(event, self);
    }

    fn render(&mut self, frame: &mut Frame<'_>) {
        render(frame, self);
    }

    fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    fn should_terminate(&self) -> bool {
        self.quit
    }
}
