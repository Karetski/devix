//! Editor application: owns terminal-side state and runs the event loop.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::Rect;
use devix_buffer::{Buffer, Range};
use devix_config::{Keymap, default_keymap};
use devix_workspace::{EditorState, StatusLine};

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
    /// Viewport-follow mode. `true` (anchored) → the renderer keeps the cursor
    /// in view; `false` (detached) → scroll_top floats independently. The mode
    /// flips on two well-defined events: any cursor move re-anchors, a fresh
    /// scroll gesture (gap > threshold from last scroll) detaches.
    pub view_anchored: bool,
    /// Wall-clock time of the most recently received scroll event. Used to
    /// distinguish a fresh user scroll from a trackpad-inertia continuation.
    pub last_scroll_at: Option<Instant>,
}

/// Maximum gap between two scroll events that still counts as the same
/// trackpad-inertia stream. macOS emits inertia at ~60Hz; 150ms is well above
/// that but well below any plausible new-gesture cadence.
pub const SCROLL_STREAM_GAP: Duration = Duration::from_millis(150);

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
            view_anchored: true,
            last_scroll_at: None,
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
            // Drain any further events already queued (e.g. a burst of
            // trackpad-inertia scroll events) before drawing the next frame.
            // Otherwise each event waits a full render cycle, which on large
            // buffers turns a fast swipe into seconds of catch-up scroll.
            while event::poll(Duration::from_millis(0))? {
                handle_event(event::read()?, &mut app);
                if app.quit { break; }
            }
        }
    }
    Ok(())
}

