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
use notify::{RecursiveMode, Watcher};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::Paragraph;

use teditor_buffer::{
    Buffer, Range, Selection, Transaction,
    delete_range_tx, replace_selection_tx,
};
use teditor_ui::{EditorView, render_editor};

struct App {
    buffer: Buffer,
    selection: Selection,
    /// Sticky column for vertical motion. Reset on horizontal motion or edit.
    target_col: Option<usize>,
    scroll_top: usize,
    status: Option<String>,
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
        let clipboard = arboard::Clipboard::new().ok();

        let (watcher, rx) = match path.as_deref() {
            Some(p) if p.exists() => spawn_watcher(p).ok().map(|(w, r)| (Some(w), Some(r))).unwrap_or((None, None)),
            _ => (None, None),
        };

        Ok(Self {
            buffer,
            selection: Selection::point(0),
            target_col: None,
            scroll_top: 0,
            status: None,
            quit: false,
            last_editor_area: Rect::default(),
            last_gutter_width: 0,
            clipboard,
            _watcher: watcher,
            disk_rx: rx,
            disk_changed_pending: false,
        })
    }

    fn primary(&self) -> Range { self.selection.primary() }

    fn set_status(&mut self, s: impl Into<String>) {
        self.status = Some(s.into());
    }

    fn clear_status(&mut self) {
        self.status = None;
    }
}

fn spawn_watcher(path: &std::path::Path) -> Result<(notify::RecommendedWatcher, mpsc::Receiver<()>)> {
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            use notify::EventKind::*;
            if matches!(ev.kind, Modify(_) | Create(_) | Remove(_)) {
                let _ = tx.send(());
            }
        }
    })?;
    // Watch the parent directory non-recursively — many editors atomic-rename
    // the file on save, which a direct file watch would miss.
    let watch_target = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    watcher.watch(watch_target, RecursiveMode::NonRecursive)?;
    Ok((watcher, rx))
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

    if app.buffer.dirty() {
        app.disk_changed_pending = true;
        app.set_status("Disk changed (buffer modified) · Ctrl+R reload, Ctrl+K keep");
    } else {
        match app.buffer.reload_from_disk() {
            Ok(()) => {
                let max = app.buffer.len_chars();
                app.selection.clamp(max);
                app.disk_changed_pending = false;
                app.set_status("reloaded from disk");
            }
            Err(e) => {
                app.set_status(format!("reload failed: {e}"));
            }
        }
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

    let cur_line = app.buffer.line_of_char(app.primary().head);
    let visible = editor_area.height as usize;
    if visible > 0 {
        if cur_line < app.scroll_top {
            app.scroll_top = cur_line;
        } else if cur_line >= app.scroll_top + visible {
            app.scroll_top = cur_line + 1 - visible;
        }
    }

    let view = EditorView {
        buffer: &app.buffer,
        selection: &app.selection,
        scroll_top: app.scroll_top,
    };
    let r = render_editor(view, editor_area, frame);
    app.last_gutter_width = r.gutter_width;
    if let Some((x, y)) = r.cursor_screen {
        frame.set_cursor_position((x, y));
    }

    render_status(frame, status_area, app);
}

fn render_status(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let path = app
        .buffer
        .path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "[scratch]".into());
    let dirty = if app.buffer.dirty() { " [+]" } else { "" };
    let head = app.primary().head;
    let line = app.buffer.line_of_char(head) + 1;
    let col = app.buffer.col_of_char(head) + 1;
    let sel_len = app.primary().len();
    let sel = if sel_len > 0 { format!(" ({sel_len} sel)") } else { String::new() };

    let left = format!(" {}{}  {}:{}{}", path, dirty, line, col, sel);
    let right = app
        .status
        .clone()
        .unwrap_or_else(|| "Ctrl+S save · Ctrl+Q quit".to_string());

    let total = area.width as usize;
    let pad = total
        .saturating_sub(left.chars().count() + right.chars().count() + 1);
    let text = format!("{}{}{} ", left, " ".repeat(pad), right);

    let para = Paragraph::new(text).style(Style::default().bg(Color::DarkGray).fg(Color::White));
    frame.render_widget(para, area);
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
// Motion
// ---------------------------------------------------------------------------

fn move_to(app: &mut App, idx: usize, extend: bool, sticky_col: bool) {
    let r = app.primary().put_head(idx, extend);
    *app.selection.primary_mut() = r;
    if !sticky_col {
        app.target_col = None;
    }
}

fn move_vertical(app: &mut App, down: bool, extend: bool) {
    let head = app.primary().head;
    let col = app.target_col.unwrap_or_else(|| app.buffer.col_of_char(head));
    let new = if down {
        app.buffer.move_down(head, Some(col))
    } else {
        app.buffer.move_up(head, Some(col))
    };
    app.target_col = Some(col);
    move_to(app, new, extend, true);
}

// ---------------------------------------------------------------------------
// Edits
// ---------------------------------------------------------------------------

fn replace_selection(app: &mut App, text: &str) {
    let tx = replace_selection_tx(&app.buffer, &app.selection, text);
    apply_tx(app, tx);
}

fn delete_primary_or(app: &mut App, builder: impl FnOnce(&Buffer, usize) -> Option<(usize, usize)>) {
    let prim = app.primary();
    if !prim.is_empty() {
        let tx = delete_range_tx(&app.buffer, &app.selection, prim.start(), prim.end());
        apply_tx(app, tx);
        return;
    }
    let Some((start, end)) = builder(&app.buffer, prim.head) else { return };
    if start == end { return; }
    let tx = delete_range_tx(&app.buffer, &app.selection, start, end);
    apply_tx(app, tx);
}

fn apply_tx(app: &mut App, tx: Transaction) {
    let after = tx.selection_after.clone();
    app.buffer.apply(tx);
    app.selection = after;
    app.target_col = None;
    app.clear_status();
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

fn handle_key(code: KeyCode, mods: KeyModifiers, app: &mut App) {
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let shift = mods.contains(KeyModifiers::SHIFT);
    let alt = mods.contains(KeyModifiers::ALT);

    // External-change reconciliation prompt.
    if app.disk_changed_pending && ctrl {
        match code {
            KeyCode::Char('r') | KeyCode::Char('R') => {
                match app.buffer.reload_from_disk() {
                    Ok(()) => {
                        let max = app.buffer.len_chars();
                        app.selection.clamp(max);
                        app.disk_changed_pending = false;
                        app.set_status("reloaded from disk");
                    }
                    Err(e) => app.set_status(format!("reload failed: {e}")),
                }
                return;
            }
            KeyCode::Char('k') | KeyCode::Char('K') => {
                app.disk_changed_pending = false;
                app.set_status("kept buffer; disk change ignored");
                return;
            }
            _ => {}
        }
    }

    // Ctrl shortcuts that do not also use Alt.
    if ctrl && !alt {
        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                app.quit = true;
                return;
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                let msg = match app.buffer.save() {
                    Ok(()) => "saved".into(),
                    Err(e) => format!("save failed: {e}"),
                };
                app.set_status(msg);
                return;
            }
            KeyCode::Char('z') | KeyCode::Char('Z') if !shift => {
                if let Some(sel) = app.buffer.undo() {
                    app.selection = sel;
                    app.target_col = None;
                    app.clear_status();
                } else {
                    app.set_status("nothing to undo");
                }
                return;
            }
            KeyCode::Char('z') | KeyCode::Char('Z') if shift => {
                if let Some(sel) = app.buffer.redo() {
                    app.selection = sel;
                    app.target_col = None;
                    app.clear_status();
                } else {
                    app.set_status("nothing to redo");
                }
                return;
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(sel) = app.buffer.redo() {
                    app.selection = sel;
                    app.target_col = None;
                    app.clear_status();
                } else {
                    app.set_status("nothing to redo");
                }
                return;
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                let end = app.buffer.len_chars();
                app.selection = Selection::single(Range::new(0, end));
                app.target_col = None;
                return;
            }
            KeyCode::Char('c') | KeyCode::Char('C') => { do_copy(app); return; }
            KeyCode::Char('x') | KeyCode::Char('X') => { do_cut(app); return; }
            KeyCode::Char('v') | KeyCode::Char('V') => { do_paste(app); return; }
            _ => {}
        }
    }

    // Motion. Shift extends selection; without Shift collapses+moves.
    let extend = shift;
    match (code, ctrl, alt) {
        (KeyCode::Left, true, false) => {
            let to = app.buffer.line_start_of(app.primary().head);
            move_to(app, to, extend, false);
            return;
        }
        (KeyCode::Right, true, false) => {
            let to = app.buffer.line_end_of(app.primary().head);
            move_to(app, to, extend, false);
            return;
        }
        (KeyCode::Up, true, false) => {
            let to = app.buffer.doc_start();
            move_to(app, to, extend, false);
            return;
        }
        (KeyCode::Down, true, false) => {
            let to = app.buffer.doc_end();
            move_to(app, to, extend, false);
            return;
        }
        (KeyCode::Left, false, true) => {
            let to = app.buffer.word_left(app.primary().head);
            move_to(app, to, extend, false);
            return;
        }
        (KeyCode::Right, false, true) => {
            let to = app.buffer.word_right(app.primary().head);
            move_to(app, to, extend, false);
            return;
        }
        (KeyCode::Left, false, false) => {
            let to = app.buffer.move_left(app.primary().head);
            move_to(app, to, extend, false);
            return;
        }
        (KeyCode::Right, false, false) => {
            let to = app.buffer.move_right(app.primary().head);
            move_to(app, to, extend, false);
            return;
        }
        (KeyCode::Up, false, false) => { move_vertical(app, false, extend); return; }
        (KeyCode::Down, false, false) => { move_vertical(app, true, extend); return; }
        (KeyCode::Home, _, _) => {
            let to = app.buffer.line_start_of(app.primary().head);
            move_to(app, to, extend, false);
            return;
        }
        (KeyCode::End, _, _) => {
            let to = app.buffer.line_end_of(app.primary().head);
            move_to(app, to, extend, false);
            return;
        }
        (KeyCode::PageUp, _, _) => {
            let step = page_step(app);
            for _ in 0..step { move_vertical(app, false, extend); }
            return;
        }
        (KeyCode::PageDown, _, _) => {
            let step = page_step(app);
            for _ in 0..step { move_vertical(app, true, extend); }
            return;
        }
        _ => {}
    }

    // Edits.
    match code {
        KeyCode::Backspace => {
            delete_primary_or(app, |buf, head| {
                if head == 0 { return None; }
                let start = if alt { buf.word_left(head) } else { head - 1 };
                Some((start, head))
            });
        }
        KeyCode::Delete => {
            delete_primary_or(app, |buf, head| {
                let len = buf.len_chars();
                if head >= len { return None; }
                let end = if alt { buf.word_right(head) } else { head + 1 };
                Some((head, end))
            });
        }
        KeyCode::Enter => replace_selection(app, "\n"),
        KeyCode::Tab => replace_selection(app, "    "),
        KeyCode::Char(c) if !ctrl => {
            let mut s = [0u8; 4];
            replace_selection(app, c.encode_utf8(&mut s));
        }
        _ => {}
    }
}

fn page_step(app: &App) -> usize {
    app.last_editor_area.height.saturating_sub(1).max(1) as usize
}

// ---------------------------------------------------------------------------
// Clipboard
// ---------------------------------------------------------------------------

fn current_line_span(buf: &Buffer, head: usize) -> (usize, usize) {
    let line = buf.line_of_char(head);
    let start = buf.line_start(line);
    let end_no_nl = start + buf.line_len_chars(line);
    let end = if line + 1 < buf.line_count() {
        buf.line_start(line + 1)
    } else {
        end_no_nl
    };
    (start, end)
}

fn do_copy(app: &mut App) {
    let prim = app.primary();
    let (start, end, msg) = if prim.is_empty() {
        let (s, e) = current_line_span(&app.buffer, prim.head);
        (s, e, "copied line")
    } else {
        (prim.start(), prim.end(), "copied")
    };
    if start == end { return; }
    let text = app.buffer.slice_to_string(start, end);
    if let Some(cb) = app.clipboard.as_mut() {
        if cb.set_text(text).is_err() {
            app.set_status("clipboard error");
            return;
        }
    } else {
        app.set_status("no system clipboard");
        return;
    }
    app.set_status(msg);
}

fn do_cut(app: &mut App) {
    let prim = app.primary();
    let (start, end, line_cut) = if prim.is_empty() {
        let (s, e) = current_line_span(&app.buffer, prim.head);
        (s, e, true)
    } else {
        (prim.start(), prim.end(), false)
    };
    if start == end { return; }
    let text = app.buffer.slice_to_string(start, end);
    if let Some(cb) = app.clipboard.as_mut() {
        if cb.set_text(text).is_err() {
            app.set_status("clipboard error");
            return;
        }
    } else {
        app.set_status("no system clipboard");
        return;
    }
    let tx = delete_range_tx(&app.buffer, &app.selection, start, end);
    apply_tx(app, tx);
    if line_cut { app.set_status("cut line"); } else { app.set_status("cut"); }
}

fn do_paste(app: &mut App) {
    let text = match app.clipboard.as_mut().and_then(|cb| cb.get_text().ok()) {
        Some(t) => t,
        None => { app.set_status("clipboard empty"); return; }
    };
    if text.is_empty() { return; }
    replace_selection(app, &text);
    app.set_status("pasted");
}

// ---------------------------------------------------------------------------
// Mouse
// ---------------------------------------------------------------------------

fn click_to_char_idx(app: &App, col: u16, row: u16) -> Option<usize> {
    let area = app.last_editor_area;
    let gutter = app.last_gutter_width;
    if row < area.y || row >= area.y + area.height { return None; }
    let text_x = area.x + gutter;
    let click_col = col.saturating_sub(text_x) as usize;
    let row_in_view = (row - area.y) as usize;
    let line = (app.scroll_top + row_in_view).min(app.buffer.line_count().saturating_sub(1));
    let col = click_col.min(app.buffer.line_len_chars(line));
    Some(app.buffer.line_start(line) + col)
}

fn handle_mouse(me: MouseEvent, app: &mut App) {
    match me.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(idx) = click_to_char_idx(app, me.column, me.row) {
                let extend = me.modifiers.contains(KeyModifiers::SHIFT);
                move_to(app, idx, extend, false);
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(idx) = click_to_char_idx(app, me.column, me.row) {
                move_to(app, idx, true, false);
            }
        }
        MouseEventKind::ScrollUp => {
            app.scroll_top = app.scroll_top.saturating_sub(3);
        }
        MouseEventKind::ScrollDown => {
            let max = app.buffer.line_count().saturating_sub(1);
            app.scroll_top = (app.scroll_top + 3).min(max);
        }
        _ => {}
    }
}
