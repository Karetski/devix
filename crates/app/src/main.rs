use std::io::stdout;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::{Constraint, Direction, Layout, Rect};

use teditor_buffer::{Buffer, Range};
use teditor_config::{Keymap, chord_from_key, default_keymap};
use teditor_ui::{EditorView, StatusInfo, render_editor, render_status as render_status_widget};
use teditor_workspace::{Action, Context, EditorState, StatusLine, Viewport, dispatch};

mod clipboard;
mod watcher;

use watcher::spawn_watcher;

struct App {
    editor: EditorState,
    keymap: Keymap,
    status: StatusLine,
    quit: bool,
    last_editor_area: Rect,
    last_gutter_width: u16,
    clipboard: Option<arboard::Clipboard>,
    /// Holds the watcher alive; events flow through `disk_rx`.
    _watcher: Option<notify::RecommendedWatcher>,
    disk_rx: Option<mpsc::Receiver<()>>,
    /// True when an external change has been signaled but we haven't reconciled.
    disk_changed_pending: bool,
}

impl App {
    fn new(path: Option<PathBuf>) -> Result<Self> {
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

    fn primary(&self) -> Range { self.editor.primary() }

    fn set_status(&mut self, s: impl Into<String>) {
        self.status.set(s);
    }
}

fn main() -> Result<()> {
    let path = std::env::args().nth(1).map(PathBuf::from);

    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(out))?;

    let result = run(&mut terminal, path);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    result
}

fn run<B: Backend>(terminal: &mut Terminal<B>, path: Option<PathBuf>) -> Result<()> {
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

fn drain_disk_events(app: &mut App) {
    let Some(rx) = app.disk_rx.as_ref() else { return };
    let mut got = false;
    while rx.try_recv().is_ok() {
        got = true;
    }
    if !got { return; }

    if app.editor.buffer.dirty() {
        app.disk_changed_pending = true;
        app.set_status("Disk changed (buffer modified) · Ctrl+R reload, Ctrl+K keep");
    } else {
        run_action(app, Action::ReloadFromDisk);
    }
}

fn render(frame: &mut ratatui::Frame<'_>, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());
    let editor_area = chunks[0];
    let status_area = chunks[1];
    app.last_editor_area = editor_area;

    let cur_line = app.editor.buffer.line_of_char(app.primary().head);
    let visible = editor_area.height as usize;
    if visible > 0 {
        if cur_line < app.editor.scroll_top {
            app.editor.scroll_top = cur_line;
        } else if cur_line >= app.editor.scroll_top + visible {
            app.editor.scroll_top = cur_line + 1 - visible;
        }
    }

    let view = EditorView {
        buffer: &app.editor.buffer,
        selection: &app.editor.selection,
        scroll_top: app.editor.scroll_top,
    };
    let r = render_editor(view, editor_area, frame);
    app.last_gutter_width = r.gutter_width;
    if let Some((x, y)) = r.cursor_screen {
        frame.set_cursor_position((x, y));
    }

    render_status(frame, status_area, app);
}

fn render_status(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let path_str = app.editor.buffer.path().map(|p| p.display().to_string());
    let head = app.primary().head;
    let info = StatusInfo {
        path: path_str.as_deref(),
        dirty: app.editor.buffer.dirty(),
        line: app.editor.buffer.line_of_char(head) + 1,
        col: app.editor.buffer.col_of_char(head) + 1,
        sel_len: app.primary().len(),
        message: app.status.get(),
    };
    render_status_widget(&info, area, frame);
}

fn handle_event(ev: Event, app: &mut App) {
    match ev {
        Event::Key(KeyEvent { code, modifiers, kind, .. })
            if kind == KeyEventKind::Press || kind == KeyEventKind::Repeat =>
        {
            handle_key(code, modifiers, app);
        }
        Event::Mouse(me) => handle_mouse(me, app),
        Event::Resize(_, _) => {}
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Input dispatch
// ---------------------------------------------------------------------------

fn handle_key(code: KeyCode, mods: KeyModifiers, app: &mut App) {
    // Disk-pending input gate: special-case Ctrl+R / Ctrl+K. Other chords
    // pass through to the keymap normally (typing while pending is allowed).
    if app.disk_changed_pending && mods.contains(KeyModifiers::CONTROL) {
        let lower = match code {
            KeyCode::Char(c) => Some(c.to_ascii_lowercase()),
            _ => None,
        };
        match lower {
            Some('r') => {
                run_action(app, Action::ReloadFromDisk);
                return;
            }
            Some('k') => {
                run_action(app, Action::KeepBufferIgnoreDisk);
                return;
            }
            _ => {}
        }
    }

    let chord = chord_from_key(code, mods);
    if let Some(action) = app.keymap.lookup(chord) {
        run_action(app, action);
        return;
    }

    // Fallback: plain typing.
    if let KeyCode::Char(c) = code {
        if !mods.contains(KeyModifiers::CONTROL) && !mods.contains(KeyModifiers::ALT) {
            run_action(app, Action::InsertChar(c));
        }
    }
}

fn handle_mouse(me: MouseEvent, app: &mut App) {
    match me.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let extend = me.modifiers.contains(KeyModifiers::SHIFT);
            run_action(app, Action::ClickAt {
                col: me.column,
                row: me.row,
                extend,
            });
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            run_action(app, Action::DragAt {
                col: me.column,
                row: me.row,
            });
        }
        MouseEventKind::ScrollUp => run_action(app, Action::ScrollUp),
        MouseEventKind::ScrollDown => run_action(app, Action::ScrollDown),
        _ => {}
    }
}

fn run_action(app: &mut App, action: Action) {
    let viewport = Viewport {
        x: app.last_editor_area.x,
        y: app.last_editor_area.y,
        width: app.last_editor_area.width,
        height: app.last_editor_area.height,
        gutter_width: app.last_gutter_width,
    };
    let mut cx = Context {
        editor: &mut app.editor,
        clipboard: &mut app.clipboard,
        status: &mut app.status,
        quit: &mut app.quit,
        disk_changed_pending: &mut app.disk_changed_pending,
        viewport,
    };
    dispatch(action, &mut cx);
}
