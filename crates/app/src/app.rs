//! Editor application: owns terminal-side state and runs the event loop.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::Rect;
use devix_buffer::{Buffer, Range};
use devix_config::{Keymap, default_keymap};
use devix_workspace::{Action, EditorState, StatusLine};

use crate::clipboard;
use crate::events::{handle_event, run_action};
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
    /// in view; `false` (detached) → scroll_top floats independently. A scroll
    /// detaches; any other action re-anchors.
    pub view_anchored: bool,
    /// Set when state has changed and the screen needs repainting. Render
    /// clears it. Without this we'd burn CPU rebuilding the frame on every
    /// idle 100ms tick of the event loop.
    pub dirty: bool,
    /// Coalesced scroll delta accumulated across one drain pass. Flushed once
    /// after the drain so a 200-event inertia burst maps to a single
    /// `ScrollBy` dispatch + one render — instead of 200 of each.
    pub pending_scroll: isize,
}

/// Cap on events drained per outer loop iteration. With unbounded drain a
/// pathological event flood (e.g. paste, terminal misbehaving) would starve
/// the renderer; this guarantees a frame at least every N events.
const MAX_DRAIN_PER_TICK: usize = 256;

/// How long to wait for input before looping back. Idle CPU is dominated by
/// this — but only when `dirty` is set we actually pay render cost.
const POLL_TIMEOUT: Duration = Duration::from_millis(100);

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
            dirty: true,
            pending_scroll: 0,
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

        if app.dirty {
            terminal.draw(|frame| render(frame, &mut app))?;
            app.dirty = false;
        }

        if !event::poll(POLL_TIMEOUT)? {
            continue;
        }
        handle_event(event::read()?, &mut app);

        // Drain any further events already queued (e.g. a burst of
        // trackpad-inertia scroll or autorepeat key events). Capped so a
        // pathological flood can't starve the renderer of its next frame.
        let mut drained = 1;
        while drained < MAX_DRAIN_PER_TICK
            && !app.quit
            && event::poll(Duration::ZERO)?
        {
            handle_event(event::read()?, &mut app);
            drained += 1;
        }

        // Flush the coalesced scroll delta as a single dispatch. Doing this
        // here (after drain) instead of per-event means hundreds of inertia
        // ticks collapse to one viewport update — and one render.
        if app.pending_scroll != 0 {
            let delta = std::mem::take(&mut app.pending_scroll);
            run_action(&mut app, Action::ScrollBy(delta));
        }
    }
    Ok(())
}
