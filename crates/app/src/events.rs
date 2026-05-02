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
        // Force a repaint so layout-dependent state (gutter width, viewport
        // recentering) is recomputed at the new size.
        Event::Resize(_, _) => app.dirty = true,
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
            // Coalesce: just bump the pending delta. The outer loop dispatches
            // one `ScrollBy(delta)` after draining the whole event burst.
            // A single render then reflects the entire swipe. Inertia tail
            // events from macOS trackpads are not filtered — they may briefly
            // drift the view between keypresses, but the next keypress
            // re-anchors via the branch in `run_action`.
            let delta: isize = if matches!(me.kind, MouseEventKind::ScrollUp) {
                -1
            } else {
                1
            };
            app.pending_scroll = app.pending_scroll.saturating_add(delta);
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
    let is_scroll = matches!(action, Action::ScrollBy(_));

    let mut cx = Context {
        editor: &mut app.editor,
        clipboard: &mut app.clipboard,
        status: &mut app.status,
        quit: &mut app.quit,
        disk_changed_pending: &mut app.disk_changed_pending,
        viewport,
    };
    dispatch(action, &mut cx);

    // A scroll detaches the view; any other action re-anchors it, even if
    // the cursor didn't move (e.g. Ctrl+Up when already at the top): the
    // keypress expresses intent to navigate, so the view should follow the
    // cursor again. This is the same bet micro makes — no inertia debounce,
    // we just trust the next keystroke to re-anchor.
    app.view_anchored = !is_scroll;

    app.dirty = true;
}
