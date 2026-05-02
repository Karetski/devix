//! Editor application: owns terminal-side state and runs the event loop.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossterm::event;
use ratatui::Terminal;
use ratatui::backend::Backend;
use devix_config::{Keymap, default_keymap};
use devix_workspace::{Action, StatusLine, Workspace};

use crate::clipboard;
use crate::events::{handle_event, run_action};
use crate::render::render;
use crate::watcher::drain_disk_events;

pub struct App {
    pub workspace: Workspace,
    pub keymap: Keymap,
    pub status: StatusLine,
    pub quit: bool,
    pub clipboard: Option<arboard::Clipboard>,
    pub dirty: bool,
    pub pending_scroll: isize,
}

const MAX_DRAIN_PER_TICK: usize = 256;
const POLL_TIMEOUT: Duration = Duration::from_millis(100);

impl App {
    pub fn new(path: Option<PathBuf>) -> Result<Self> {
        let workspace = Workspace::open(path)?;
        let clipboard = clipboard::init();

        Ok(Self {
            workspace,
            keymap: default_keymap(),
            status: StatusLine::default(),
            quit: false,
            clipboard,
            dirty: true,
            pending_scroll: 0,
        })
    }
}

pub fn run<B: Backend>(terminal: &mut Terminal<B>, path: Option<PathBuf>) -> Result<()> {
    let mut app = App::new(path)?;

    while !app.quit {
        drain_disk_events(&mut app);

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
            run_action(&mut app, Action::ScrollBy(delta));
        }
    }
    Ok(())
}
