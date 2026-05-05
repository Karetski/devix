//! Editor application: owns terminal-side state and runs the event loop.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossterm::event;
use ratatui::Terminal;
use ratatui::backend::Backend;
use devix_core::Theme;
use devix_workspace::{
    CommandRegistry, Keymap, StatusLine, Workspace, build_registry, default_keymap,
};

use crate::clipboard;
use crate::events::{handle_event, run_command};
use crate::lsp::{LspWiring, drain_lsp_events, setup_lsp};
use crate::render::render;
use crate::watcher::drain_disk_events;

pub struct App {
    pub workspace: Workspace,
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
}

const MAX_DRAIN_PER_TICK: usize = 256;
const POLL_TIMEOUT: Duration = Duration::from_millis(100);

impl App {
    pub fn new(path: Option<PathBuf>) -> Result<Self> {
        let mut workspace = Workspace::open(path)?;
        let clipboard = clipboard::init();

        // LSP setup is best-effort: if it fails we still launch the editor
        // without server integration rather than refusing to open a file.
        let lsp = match setup_lsp() {
            Ok((sink, encoding, wiring)) => {
                workspace.attach_lsp(sink, encoding);
                Some(wiring)
            }
            Err(_) => None,
        };

        Ok(Self {
            workspace,
            commands: build_registry(),
            keymap: default_keymap(),
            theme: Theme::default(),
            status: StatusLine::default(),
            quit: false,
            clipboard,
            dirty: true,
            pending_scroll: 0,
            lsp,
        })
    }
}

pub fn run<B: Backend>(terminal: &mut Terminal<B>, path: Option<PathBuf>) -> Result<()> {
    let mut app = App::new(path)?;

    while !app.quit {
        drain_disk_events(&mut app);
        drain_lsp_events(&mut app);

        if app.dirty {
            terminal.draw(|frame| render(frame, &mut app))?;
            app.dirty = false;
        }

        if !event::poll(POLL_TIMEOUT)? { continue; }
        handle_event(event::read()?, &mut app);

        let mut drained = 1;
        while drained < MAX_DRAIN_PER_TICK
            && !app.quit
            && event::poll(Duration::ZERO)?
        {
            handle_event(event::read()?, &mut app);
            drained += 1;
        }

        if app.pending_scroll != 0 {
            let delta = std::mem::take(&mut app.pending_scroll);
            run_command(&mut app, std::sync::Arc::new(devix_workspace::cmd::ScrollBy(delta)));
        }
    }
    Ok(())
}
