//! Input events: KeyEvent → Chord → command via keymap; MouseEvent →
//! command directly. The disk-pending input gate is enforced here.

use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind};
use devix_core::HandleCtx;
use devix_surface::{
    Context, EditorCommand, ModalOutcome, PalettePane, SettingsPane, SymbolPickerPane,
    TabStripHit, Viewport, chord_from_key, cmd,
};

use crate::app::App;

pub fn handle_event(ev: Event, app: &mut App) {
    match ev {
        Event::Key(KeyEvent { code, modifiers, kind, .. })
            if kind == KeyEventKind::Press || kind == KeyEventKind::Repeat =>
        {
            handle_key(ev, code, modifiers, app);
        }
        Event::Mouse(me) => handle_mouse(me, app),
        Event::Resize(_, _) => app.dirty = true,
        _ => {}
    }
}

pub fn handle_key(ev: Event, code: KeyCode, mods: KeyModifiers, app: &mut App) {
    // Modal at the head of the responder chain. The modal Pane's `handle`
    // does navigation/typing internally; close / accept / LSP-refetch
    // come back as flags drained via `ModalOutcome`. Any keys it doesn't
    // claim are silently swallowed — modal mode is input-modal: letting
    // Ctrl+S still save with the palette open would be surprising.
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

    // Completion popup intercepts navigation/accept keys before the
    // keymap. Esc dismisses; Tab/Enter accept; Up/Down navigate. Other
    // keys (printable chars, Backspace) fall through to the editor and
    // re-filter the popup post-dispatch.
    if completion_open(app) {
        match (code, mods) {
            (KeyCode::Esc, _) => { run_command(app, Arc::new(cmd::CompletionDismiss)); return; }
            (KeyCode::Tab, _) | (KeyCode::Enter, _) => {
                run_command(app, Arc::new(cmd::CompletionAccept));
                return;
            }
            (KeyCode::Up, _) => { run_command(app, Arc::new(cmd::CompletionMove(-1))); return; }
            (KeyCode::Down, _) => { run_command(app, Arc::new(cmd::CompletionMove(1))); return; }
            _ => {}
        }
    }

    // When focus sits on a plugin-contributed sidebar pane, give the
    // plugin first crack at every key. If the plugin registered an
    // `on_key` callback the event is forwarded and treated as
    // consumed; otherwise we fall through to the keymap so navigation
    // chords still work even with no plugin handler.
    if let Some(slot) = crate::plugin::focused_plugin_slot(app) {
        if let Event::Key(key_ev) = ev {
            if crate::plugin::forward_key_to_plugin(app, slot, key_ev) {
                app.dirty = true;
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

fn completion_open(app: &App) -> bool {
    app.surface
        .active_view()
        .map(|v| v.completion.is_some())
        .unwrap_or(false)
}

/// Hand `ev` to the modal Pane via `Pane::handle`, then drain any
/// side-effect outcome (close / accept / refetch) the modal signaled.
/// The drain step is the one place the host has to know about specific
/// modal types — palette accepts dispatch the chosen command; symbols
/// accepts jump to the picked location; refetch re-queries the LSP.
fn dispatch_modal_event(app: &mut App, ev: &Event) {
    {
        let modal = app
            .surface
            .modal
            .as_mut()
            .expect("dispatch_modal_event requires a modal");
        let mut hctx = HandleCtx::default();
        let _ = modal.handle(ev, ratatui::layout::Rect::default(), &mut hctx);
    }

    let outcome = drain_modal_outcome(app);
    match outcome {
        ModalOutcome::Dismiss => run_command(app, Arc::new(cmd::CloseModal)),
        ModalOutcome::Accept => {
            // Type-specific accept: palette resolves+invokes the chosen
            // command; symbols jumps to the picked location.
            let action: Arc<dyn EditorCommand> = if modal_is::<PalettePane>(app) {
                Arc::new(cmd::PaletteAccept)
            } else if modal_is::<SymbolPickerPane>(app) {
                Arc::new(cmd::SymbolsAccept)
            } else {
                Arc::new(cmd::CloseModal)
            };
            run_command(app, action);
        }
        ModalOutcome::Refetch => run_command(app, Arc::new(cmd::RefetchWorkspaceSymbols)),
        ModalOutcome::None => {
            app.dirty = true;
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
    if let Some(s) = any.downcast_mut::<SymbolPickerPane>() {
        return s.drain_outcome();
    }
    if let Some(s) = any.downcast_mut::<SettingsPane>() {
        return s.drain_outcome();
    }
    ModalOutcome::None
}

pub fn handle_mouse(me: MouseEvent, app: &mut App) {
    // Modal swallows mouse so clicks never reach the editor or tab strip.
    // Left-click dismisses (matches most editors' click-out UX); modal-
    // specific mouse handling is a future polish item.
    if app.surface.modal.is_some() {
        if matches!(me.kind, MouseEventKind::Down(MouseButton::Left)) {
            run_command(app, Arc::new(cmd::CloseModal));
        }
        return;
    }

    match me.kind {
        MouseEventKind::Down(button @ (MouseButton::Left | MouseButton::Right | MouseButton::Middle)) => {
            // Tab-strip clicks are not editor clicks: don't fall through to
            // ClickAt or we'd reposition the caret on a phantom row.
            if button == MouseButton::Left {
                if let Some(hit) = app.surface.tab_strip_hit(me.column, me.row) {
                    handle_tab_strip_click(app, hit);
                    return;
                }
            }
            app.surface.focus_at_screen(me.column, me.row);
            // Plugin sidebar panes get the click before the editor's
            // ClickAt — clicking inside a plugin pane shouldn't move the
            // editor caret. Coordinates are converted to pane-relative
            // (0,0 = top-left of the inner content area).
            if let Some(slot) = crate::plugin::focused_plugin_slot(app) {
                if let Some((rx, ry)) = sidebar_inner_relative(app, slot, me.column, me.row) {
                    if crate::plugin::forward_click_to_plugin(app, slot, rx, ry, button) {
                        app.dirty = true;
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
            // Wheel over a *scrollable* tab strip scrolls the strip
            // horizontally (vertical-wheel-as-horizontal-scroll, like browsers
            // do). When the strip already fits, fall through so the wheel
            // reaches the editor instead of being silently swallowed.
            if let Some(fid) = app.surface.frame_at_strip(me.column, me.row) {
                if app.surface.tab_strip_can_scroll(fid) {
                    let delta: isize = if matches!(me.kind, MouseEventKind::ScrollUp) { -2 } else { 2 };
                    app.surface.scroll_tab_strip(fid, delta);
                    app.dirty = true;
                    return;
                }
            }
            // Wheel over a plugin sidebar scrolls *its* line buffer,
            // not the editor body. Without this the user has no way to
            // see content past the bottom of a long pane.
            if let Some(slot) = crate::plugin::plugin_slot_at(app, me.column, me.row) {
                let delta: i32 = if matches!(me.kind, MouseEventKind::ScrollUp) { -2 } else { 2 };
                if crate::plugin::scroll_plugin_pane(app, slot, delta) {
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
            if let Some(fid) = app.surface.frame_at_strip(me.column, me.row) {
                if app.surface.tab_strip_can_scroll(fid) {
                    let delta: isize = if matches!(me.kind, MouseEventKind::ScrollLeft) { -2 } else { 2 };
                    app.surface.scroll_tab_strip(fid, delta);
                    app.dirty = true;
                }
            }
        }
        _ => {}
    }
}

/// Translate a screen `(col, row)` click into pane-relative coordinates
/// inside `slot`'s rendered content area. Returns `None` if the click
/// missed the slot or lay on the chrome border. Reads the cached
/// sidebar rect populated by the previous render pass.
fn sidebar_inner_relative(
    app: &App,
    slot: devix_surface::SidebarSlot,
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
    app.dirty = true;
}

pub fn run_command(app: &mut App, action: Arc<dyn EditorCommand>) {
    // Resolve the viewport against the *currently focused* frame at dispatch
    // time. Mouse handlers update focus before calling this, so reading from
    // the render cache here picks up the new frame's body rect.
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
        clipboard: &mut app.clipboard,
        status: &mut app.status,
        quit: &mut app.quit,
        viewport,
        commands: &app.commands,
    };
    // Phase 5: every command flows through `core::Action::invoke`. The old
    // `dispatch(Action, ...)` enum match is no longer in the input path.
    action.invoke(&mut cx);

    app.dirty = true;
}
