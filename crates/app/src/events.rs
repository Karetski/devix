//! Input events: KeyEvent → Chord → command via keymap; MouseEvent →
//! command directly. The disk-pending input gate is enforced here.

use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind};
use devix_commands::{
    Context, EditorCommand, ModalOutcome, PalettePane, Viewport, chord_from_key, cmd,
};
use devix_core::HandleCtx;
use devix_surface::TabStripHit;

use crate::app::App;

pub fn handle_event(ev: Event, app: &mut App) {
    match ev {
        Event::Key(KeyEvent { code, modifiers, kind, .. })
            if kind == KeyEventKind::Press || kind == KeyEventKind::Repeat =>
        {
            handle_key(ev, code, modifiers, app);
        }
        Event::Mouse(me) => handle_mouse(me, app),
        Event::Resize(_, _) => app.request_redraw(),
        _ => {}
    }
}

pub fn handle_key(ev: Event, code: KeyCode, mods: KeyModifiers, app: &mut App) {
    if app.surface.modal.is_some() {
        dispatch_modal_event(app, &ev);
        return;
    }

    let pending = app.surface.active_doc().map(|d| d.disk_changed_pending).unwrap_or(false);
    if pending && mods.contains(KeyModifiers::CONTROL) {
        let lower = match code {
            KeyCode::Char(c) => Some(c.to_ascii_lowercase()),
            _ => None,
        };
        match lower {
            Some('r') => { run_command(app, Arc::new(cmd::ReloadFromDisk)); return; }
            Some('k') => { run_command(app, Arc::new(cmd::KeepBufferIgnoreDisk)); return; }
            _ => {}
        }
    }

    // When focus sits on a plugin-contributed sidebar pane, give the
    // plugin first crack at every key.
    if let Some(slot) = crate::plugin::focused_plugin_slot(app) {
        if let Event::Key(key_ev) = ev {
            if crate::plugin::forward_key_to_plugin(app, slot, key_ev) {
                app.request_redraw();
                return;
            }
        }
    }

    let chord = chord_from_key(code, mods);
    if let Some(action) = app.keymap.lookup(chord, &app.commands) {
        run_command(app, action);
        return;
    }

    if let KeyCode::Char(c) = code {
        if !mods.contains(KeyModifiers::CONTROL) && !mods.contains(KeyModifiers::ALT) {
            run_command(app, Arc::new(cmd::InsertChar(c)));
        }
    }
}

fn dispatch_modal_event(app: &mut App, ev: &Event) {
    {
        let modal = app
            .surface
            .modal
            .as_mut()
            .expect("dispatch_modal_event requires a modal");
        let mut hctx = HandleCtx::default();
        let _ = modal.handle(ev, devix_core::Rect::default(), &mut hctx);
    }

    let outcome = drain_modal_outcome(app);
    match outcome {
        ModalOutcome::Dismiss => run_command(app, Arc::new(cmd::CloseModal)),
        ModalOutcome::Accept => {
            let action: Arc<dyn EditorCommand> = if modal_is::<PalettePane>(app) {
                Arc::new(cmd::PaletteAccept)
            } else {
                Arc::new(cmd::CloseModal)
            };
            run_command(app, action);
        }
        ModalOutcome::None => {
            app.request_redraw();
        }
    }
}

fn modal_is<T: 'static>(app: &App) -> bool {
    app.surface
        .modal
        .as_ref()
        .and_then(|m| m.as_any())
        .map(|a| a.is::<T>())
        .unwrap_or(false)
}

fn drain_modal_outcome(app: &mut App) -> ModalOutcome {
    let Some(any) = app
        .surface
        .modal
        .as_mut()
        .and_then(|m| m.as_any_mut())
    else {
        return ModalOutcome::None;
    };
    if let Some(p) = any.downcast_mut::<PalettePane>() {
        return p.drain_outcome();
    }
    ModalOutcome::None
}

pub fn handle_mouse(me: MouseEvent, app: &mut App) {
    if app.surface.modal.is_some() {
        if matches!(me.kind, MouseEventKind::Down(MouseButton::Left)) {
            run_command(app, Arc::new(cmd::CloseModal));
        }
        return;
    }

    match me.kind {
        MouseEventKind::Down(button @ (MouseButton::Left | MouseButton::Right | MouseButton::Middle)) => {
            if button == MouseButton::Left {
                if let Some(hit) = app.surface.tab_strip_hit(me.column, me.row) {
                    handle_tab_strip_click(app, hit);
                    return;
                }
            }
            app.surface.focus_at_screen(me.column, me.row);
            if let Some(slot) = crate::plugin::focused_plugin_slot(app) {
                if let Some((rx, ry)) = sidebar_inner_relative(app, slot, me.column, me.row) {
                    if crate::plugin::forward_click_to_plugin(app, slot, rx, ry, button) {
                        app.request_redraw();
                        return;
                    }
                }
            }
            if button != MouseButton::Left {
                return;
            }
            let extend = me.modifiers.contains(KeyModifiers::SHIFT);
            run_command(app, Arc::new(cmd::ClickAt {
                col: me.column, row: me.row, extend,
            }));
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            run_command(app, Arc::new(cmd::DragAt {
                col: me.column, row: me.row,
            }));
        }
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            if let Some(fid) = app.surface.frame_at_strip(me.column, me.row) {
                if app.surface.tab_strip_can_scroll(fid) {
                    let delta: isize = if matches!(me.kind, MouseEventKind::ScrollUp) { -2 } else { 2 };
                    app.surface.scroll_tab_strip(fid, delta);
                    app.request_redraw();
                    return;
                }
            }
            if let Some(slot) = crate::plugin::plugin_slot_at(app, me.column, me.row) {
                let delta: i32 = if matches!(me.kind, MouseEventKind::ScrollUp) { -2 } else { 2 };
                if crate::plugin::scroll_plugin_pane(app, slot, delta) {
                    app.request_redraw();
                    return;
                }
            }
            let delta: isize = if matches!(me.kind, MouseEventKind::ScrollUp) { -1 } else { 1 };
            app.pending_scroll = app.pending_scroll.saturating_add(delta);
        }
        MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
            if let Some(fid) = app.surface.frame_at_strip(me.column, me.row) {
                if app.surface.tab_strip_can_scroll(fid) {
                    let delta: isize = if matches!(me.kind, MouseEventKind::ScrollLeft) { -2 } else { 2 };
                    app.surface.scroll_tab_strip(fid, delta);
                    app.request_redraw();
                }
            }
        }
        _ => {}
    }
}

fn sidebar_inner_relative(
    app: &App,
    slot: devix_core::SidebarSlot,
    col: u16,
    row: u16,
) -> Option<(u16, u16)> {
    let rect = app.surface.render_cache.sidebar_rects.get(&slot).copied()?;
    let inner_x = rect.x.saturating_add(1);
    let inner_y = rect.y.saturating_add(1);
    let inner_w = rect.width.saturating_sub(2);
    let inner_h = rect.height.saturating_sub(2);
    if col < inner_x || row < inner_y || col >= inner_x + inner_w || row >= inner_y + inner_h {
        return None;
    }
    Some((col - inner_x, row - inner_y))
}

fn handle_tab_strip_click(app: &mut App, hit: TabStripHit) {
    let TabStripHit::Tab { frame, idx } = hit;
    app.surface.focus_frame(frame);
    app.surface.activate_tab(frame, idx);
    app.request_redraw();
}

pub fn run_command(app: &mut App, action: Arc<dyn EditorCommand>) {
    let rect = app
        .surface
        .active_frame()
        .and_then(|fid| app.surface.render_cache.frame_rects.get(&fid).copied())
        .unwrap_or_default();
    let gutter_width = app
        .surface
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

    let mut cx = Context {
        surface: &mut app.surface,
        clipboard: app.clipboard.as_mut(),
        quit: &mut app.quit,
        viewport,
        commands: &app.commands,
    };
    action.invoke(&mut cx);

    app.request_redraw();
}
