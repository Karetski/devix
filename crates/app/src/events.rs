//! Input events: KeyEvent → Chord → Action via keymap; MouseEvent → Action
//! directly. The disk-pending input gate is enforced here.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind};
use devix_config::chord_from_key;
use devix_workspace::{Action, Context, Overlay, ScrollMode, TabStripHit, Viewport, dispatch};

use crate::app::App;

pub fn handle_event(ev: Event, app: &mut App) {
    match ev {
        Event::Key(KeyEvent { code, modifiers, kind, .. })
            if kind == KeyEventKind::Press || kind == KeyEventKind::Repeat =>
        {
            handle_key(code, modifiers, app);
        }
        Event::Mouse(me) => handle_mouse(me, app),
        Event::Resize(_, _) => app.dirty = true,
        _ => {}
    }
}

pub fn handle_key(code: KeyCode, mods: KeyModifiers, app: &mut App) {
    // Overlay takes input first. The palette consumes its navigation keys
    // (arrows / Enter / Esc / Backspace / printable chars) and only falls
    // through for chords it doesn't care about — letting Ctrl+S still save
    // even with the palette open would be surprising, so palette mode is
    // input-modal: any unhandled key is silently ignored.
    if matches!(app.overlay, Some(Overlay::Palette(_))) {
        handle_palette_key(code, mods, app);
        return;
    }

    let pending = app.workspace.active_doc().map(|d| d.disk_changed_pending).unwrap_or(false);
    if pending && mods.contains(KeyModifiers::CONTROL) {
        let lower = match code {
            KeyCode::Char(c) => Some(c.to_ascii_lowercase()),
            _ => None,
        };
        match lower {
            Some('r') => { run_action(app, Action::ReloadFromDisk); return; }
            Some('k') => { run_action(app, Action::KeepBufferIgnoreDisk); return; }
            _ => {}
        }
    }

    // Completion popup intercepts navigation/accept keys before the
    // keymap. Esc dismisses; Tab/Enter accept; Up/Down navigate. Other
    // keys (printable chars, Backspace) fall through to the editor and
    // re-filter the popup post-dispatch.
    if completion_open(app) {
        match (code, mods) {
            (KeyCode::Esc, _) => { run_action(app, Action::CompletionDismiss); return; }
            (KeyCode::Tab, _) | (KeyCode::Enter, _) => {
                run_action(app, Action::CompletionAccept);
                return;
            }
            (KeyCode::Up, _) => { run_action(app, Action::CompletionMove(-1)); return; }
            (KeyCode::Down, _) => { run_action(app, Action::CompletionMove(1)); return; }
            _ => {}
        }
    }

    let chord = chord_from_key(code, mods);
    if let Some(action) = app.keymap.lookup(chord, &app.commands) {
        run_action(app, action);
        return;
    }

    if let KeyCode::Char(c) = code {
        if !mods.contains(KeyModifiers::CONTROL) && !mods.contains(KeyModifiers::ALT) {
            run_action(app, Action::InsertChar(c));
        }
    }
}

fn completion_open(app: &App) -> bool {
    app.workspace
        .active_view()
        .map(|v| v.completion.is_some())
        .unwrap_or(false)
}

fn handle_palette_key(code: KeyCode, mods: KeyModifiers, app: &mut App) {
    match (code, mods) {
        (KeyCode::Esc, _) => run_action(app, Action::ClosePalette),
        (KeyCode::Enter, _) => run_action(app, Action::PaletteAccept),
        (KeyCode::Up, _) => run_action(app, Action::PaletteMove(-1)),
        (KeyCode::Down, _) => run_action(app, Action::PaletteMove(1)),
        (KeyCode::Backspace, _) => {
            let mut q = current_palette_query(app).to_string();
            q.pop();
            run_action(app, Action::PaletteSetQuery(q));
        }
        (KeyCode::Char(c), m)
            if !m.contains(KeyModifiers::CONTROL) && !m.contains(KeyModifiers::ALT) =>
        {
            let mut q = current_palette_query(app).to_string();
            q.push(c);
            run_action(app, Action::PaletteSetQuery(q));
        }
        _ => {}
    }
}

fn current_palette_query(app: &App) -> &str {
    match &app.overlay {
        Some(Overlay::Palette(p)) => p.query(),
        _ => "",
    }
}

pub fn handle_mouse(me: MouseEvent, app: &mut App) {
    // Overlay swallows mouse so clicks never reach the editor or tab strip.
    // Left-click dismisses the palette (matches most editors' click-out UX);
    // mouse-driven row selection is a future polish item.
    if matches!(app.overlay, Some(Overlay::Palette(_))) {
        if matches!(me.kind, MouseEventKind::Down(MouseButton::Left)) {
            run_action(app, Action::ClosePalette);
        }
        return;
    }

    match me.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // Tab-strip clicks are not editor clicks: don't fall through to
            // ClickAt or we'd reposition the caret on a phantom row.
            if let Some(hit) = app.workspace.tab_strip_hit(me.column, me.row) {
                handle_tab_strip_click(app, hit);
                return;
            }
            app.workspace.focus_at_screen(me.column, me.row);
            let extend = me.modifiers.contains(KeyModifiers::SHIFT);
            run_action(app, Action::ClickAt {
                col: me.column, row: me.row, extend,
            });
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            run_action(app, Action::DragAt {
                col: me.column, row: me.row,
            });
        }
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            // Wheel over a *scrollable* tab strip scrolls the strip
            // horizontally (vertical-wheel-as-horizontal-scroll, like browsers
            // do). When the strip already fits, fall through so the wheel
            // reaches the editor instead of being silently swallowed.
            if let Some(fid) = app.workspace.frame_at_strip(me.column, me.row) {
                if app.workspace.tab_strip_can_scroll(fid) {
                    let delta: isize = if matches!(me.kind, MouseEventKind::ScrollUp) { -2 } else { 2 };
                    app.workspace.scroll_tab_strip(fid, delta);
                    app.dirty = true;
                    return;
                }
            }
            let delta: isize = if matches!(me.kind, MouseEventKind::ScrollUp) { -1 } else { 1 };
            app.pending_scroll = app.pending_scroll.saturating_add(delta);
        }
        MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
            // Real horizontal scroll (trackpad swipe). Only meaningful on the
            // tab strip; the editor is not horizontally scrollable today.
            if let Some(fid) = app.workspace.frame_at_strip(me.column, me.row) {
                if app.workspace.tab_strip_can_scroll(fid) {
                    let delta: isize = if matches!(me.kind, MouseEventKind::ScrollLeft) { -2 } else { 2 };
                    app.workspace.scroll_tab_strip(fid, delta);
                    app.dirty = true;
                }
            }
        }
        _ => {}
    }
}

fn handle_tab_strip_click(app: &mut App, hit: TabStripHit) {
    let TabStripHit::Tab { frame, idx } = hit;
    app.workspace.focus_frame(frame);
    app.workspace.activate_tab(frame, idx);
    if let Some(v) = app.workspace.active_view_mut() {
        v.scroll_mode = ScrollMode::Anchored;
    }
    app.dirty = true;
}

pub fn run_action(app: &mut App, action: Action) {
    // Resolve the viewport against the *currently focused* frame at dispatch
    // time. Mouse handlers update focus before calling this, so reading from
    // the render cache here picks up the new frame's body rect — using cached
    // last-render values would translate clicks against the previously-active
    // frame.
    let rect = app
        .workspace
        .active_frame()
        .and_then(|fid| app.workspace.render_cache.frame_rects.get(fid).copied())
        .unwrap_or_default();
    let gutter_width = app
        .workspace
        .active_doc()
        .map(|d| (d.buffer.line_count().to_string().len() as u16) + 2)
        .unwrap_or(0);
    let viewport = Viewport {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        gutter_width,
    };
    let is_scroll = matches!(action, Action::ScrollBy(_));

    let mut cx = Context {
        workspace: &mut app.workspace,
        clipboard: &mut app.clipboard,
        status: &mut app.status,
        quit: &mut app.quit,
        viewport,
        commands: &app.commands,
        overlay: &mut app.overlay,
    };
    dispatch(action, &mut cx);

    if let Some(v) = app.workspace.active_view_mut() {
        v.scroll_mode = if is_scroll { ScrollMode::Free } else { ScrollMode::Anchored };
    }
    app.dirty = true;
}
