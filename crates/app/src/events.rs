//! Input events: KeyEvent → Chord → Action via keymap; MouseEvent → Action
//! directly. The disk-pending input gate is enforced here.

use std::time::Instant;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind};
use devix_config::chord_from_key;
use devix_workspace::{Action, Context, Viewport, dispatch};

use crate::app::{App, SCROLL_STREAM_GAP};

pub fn handle_event(ev: Event, app: &mut App) {
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

pub fn handle_key(code: KeyCode, mods: KeyModifiers, app: &mut App) {
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

    if let KeyCode::Char(c) = code {
        if !mods.contains(KeyModifiers::CONTROL) && !mods.contains(KeyModifiers::ALT) {
            run_action(app, Action::InsertChar(c));
        }
    }
}

pub fn handle_mouse(me: MouseEvent, app: &mut App) {
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
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            let now = Instant::now();
            // A scroll event with a large gap from the previous one is a fresh
            // user gesture (the OS-emitted inertia stream had ended); a small
            // gap means we're still inside that stream.
            let fresh_gesture = app
                .last_scroll_at
                .map(|t| now.duration_since(t) >= SCROLL_STREAM_GAP)
                .unwrap_or(true);
            app.last_scroll_at = Some(now);

            // Anchored + inertia continuation = noise from a stream the user
            // already cancelled by moving the cursor. Swallow it. No latching
            // here: the next fresh gesture (gap >= threshold) detaches the
            // view and dispatches normally.
            if app.view_anchored && !fresh_gesture {
                return;
            }
            if fresh_gesture {
                app.view_anchored = false;
            }

            let action = if matches!(me.kind, MouseEventKind::ScrollUp) {
                Action::ScrollUp
            } else {
                Action::ScrollDown
            };
            run_action(app, action);
        }
        _ => {}
    }
}

pub fn run_action(app: &mut App, action: Action) {
    let viewport = Viewport {
        x: app.last_editor_area.x,
        y: app.last_editor_area.y,
        width: app.last_editor_area.width,
        height: app.last_editor_area.height,
        gutter_width: app.last_gutter_width,
    };
    let head_before = app.editor.primary().head;
    let mut cx = Context {
        editor: &mut app.editor,
        clipboard: &mut app.clipboard,
        status: &mut app.status,
        quit: &mut app.quit,
        disk_changed_pending: &mut app.disk_changed_pending,
        viewport,
    };
    dispatch(action, &mut cx);
    // Any cursor move (key navigation, click, drag, edit) re-anchors the view.
    // The mode flips back to detached only via a fresh scroll gesture.
    if app.editor.primary().head != head_before {
        app.view_anchored = true;
    }
}
