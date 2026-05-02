//! Editor application: owns terminal-side state and runs the event loop.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::Rect;
use teditor_buffer::{Buffer, Range};
use teditor_config::{Keymap, default_keymap};
use teditor_workspace::{EditorState, StatusLine};

use crate::clipboard;
use crate::events::handle_event;
use crate::render::render;
use crate::watcher::{drain_disk_events, spawn_watcher};

pub struct App {
    pub editor: EditorState,
    pub keymap: Keymap,
    pub status: StatusLine,
    pub quit: bool,
    pub last_editor_area: Rect,
    pub last_gutter_width: u16,
    pub clipboard: Option<arboard::Clipboard>,
    /// Holds the watcher alive; events flow through `disk_rx`.
    pub _watcher: Option<notify::RecommendedWatcher>,
    pub disk_rx: Option<mpsc::Receiver<()>>,
    /// True when an external change has been signaled but we haven't reconciled.
    pub disk_changed_pending: bool,
}

impl App {
    pub fn new(path: Option<PathBuf>) -> Result<Self> {
        let buffer = match path.clone() {
            Some(p) => Buffer::from_path(p)?,
            None => Buffer::empty(),
        };
        let clipboard = clipboard::init();

        let (watcher, rx) = match path.as_deref() {
            Some(p) if p.exists() => spawn_watcher(p)
                .ok()
                .map(|(w, r)| (Some(w), Some(r)))
                .unwrap_or((None, None)),
            _ => (None, None),
        };

        Ok(Self {
            editor: EditorState::new(buffer),
            keymap: default_keymap(),
            status: StatusLine::default(),
            quit: false,
            last_editor_area: Rect::default(),
            last_gutter_width: 0,
            clipboard,
            _watcher: watcher,
            disk_rx: rx,
            disk_changed_pending: false,
        })
    }

    pub fn primary(&self) -> Range {
        self.editor.primary()
    }
}

pub fn run<B: Backend>(terminal: &mut Terminal<B>, path: Option<PathBuf>) -> Result<()> {
    let mut app = App::new(path)?;

    while !app.quit {
        drain_disk_events(&mut app);
        terminal.draw(|frame| render(frame, &mut app))?;

        if event::poll(Duration::from_millis(100))? {
            handle_event(event::read()?, &mut app);
        }
    }
    Ok(())
}

