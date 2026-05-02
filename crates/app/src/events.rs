//! Input events: KeyEvent → Chord → Action via keymap; MouseEvent → Action
//! directly. The disk-pending input gate is enforced here.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind};
use devix_config::chord_from_key;
use devix_workspace::{Action, Context, Viewport, dispatch};

use crate::app::App;

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
        MouseEventKind::ScrollUp => run_action(app, Action::ScrollUp),
        MouseEventKind::ScrollDown => run_action(app, Action::ScrollDown),
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
