//! Translate terminal input into editor mutations on `AppContext`.
//!
//! KeyEvent → `Pane::handle` on the focused leaf → keymap → InsertChar
//! fallback. Mouse events route to the leaf at the click position via
//! the same `Pane::handle` chain, falling back to `ClickAt`/`DragAt` for
//! the editor body.
//!
//! Plugin-specific routing lives nowhere in this file; the responder
//! chain is the sole routing. Plugin sidebars participate by virtue of
//! being installed as `Box<dyn Pane>` content on `LayoutSidebar`.

use std::sync::Arc;

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use devix_core::{
    EditorCommand, ModalOutcome, PalettePane, TabStripHit, chord_from_key, cmd,
};
use devix_core::{HandleCtx, Outcome, Rect};

use crate::context::AppContext;

pub fn handle(ev: Event, ctx: &mut AppContext<'_>) {
    match ev {
        Event::Key(KeyEvent { code, modifiers, kind, .. })
            if kind == KeyEventKind::Press || kind == KeyEventKind::Repeat =>
        {
            handle_key(ev, code, modifiers, ctx);
        }
        Event::Mouse(me) => handle_mouse(ev, me, ctx),
        Event::Resize(_, _) => ctx.request_redraw(),
        _ => {}
    }
}

fn handle_key(ev: Event, code: KeyCode, mods: KeyModifiers, ctx: &mut AppContext<'_>) {
    if ctx.editor.modal.is_some() {
        dispatch_modal_event(ctx, &ev);
        return;
    }

    let pending = ctx
        .editor
        .active_doc()
        .map(|d| d.disk_changed_pending)
        .unwrap_or(false);
    if pending && mods.contains(KeyModifiers::CONTROL) {
        let lower = match code {
            KeyCode::Char(c) => Some(c.to_ascii_lowercase()),
            _ => None,
        };
        match lower {
            Some('r') => {
                ctx.run(&cmd::ReloadFromDisk);
                return;
            }
            Some('k') => {
                ctx.run(&cmd::KeepBufferIgnoreDisk);
                return;
            }
            _ => {}
        }
    }

    if dispatch_to_focused_leaf(ctx, &ev) == Outcome::Consumed {
        ctx.request_redraw();
        return;
    }

    let chord = chord_from_key(code, mods);
    if let Some(action) = ctx.keymap.resolve_chord(chord, ctx.commands) {
        run_arc(ctx, action);
        return;
    }

    if let KeyCode::Char(c) = code {
        if !mods.contains(KeyModifiers::CONTROL) && !mods.contains(KeyModifiers::ALT) {
            ctx.run(&cmd::InsertChar(c));
        }
    }
}

fn dispatch_modal_event(ctx: &mut AppContext<'_>, ev: &Event) {
    {
        let modal = ctx
            .editor
            .modal
            .as_mut()
            .expect("dispatch_modal_event requires a modal");
        let mut hctx = HandleCtx::default();
        let _ = modal.handle(ev, Rect::default(), &mut hctx);
    }

    let outcome = drain_modal_outcome(ctx);
    match outcome {
        ModalOutcome::Dismiss => ctx.run(&cmd::CloseModal),
        ModalOutcome::Accept => {
            if modal_is::<PalettePane>(ctx) {
                ctx.run(&cmd::PaletteAccept);
            } else {
                ctx.run(&cmd::CloseModal);
            }
        }
        ModalOutcome::None => ctx.request_redraw(),
    }
}

fn modal_is<T: 'static>(ctx: &AppContext<'_>) -> bool {
    ctx.editor
        .modal
        .as_ref()
        .and_then(|m| m.as_any())
        .map(|a| a.is::<T>())
        .unwrap_or(false)
}

fn drain_modal_outcome(ctx: &mut AppContext<'_>) -> ModalOutcome {
    let Some(any) = ctx.editor.modal.as_mut().and_then(|m| m.as_any_mut()) else {
        return ModalOutcome::None;
    };
    if let Some(p) = any.downcast_mut::<PalettePane>() {
        return p.drain_outcome();
    }
    ModalOutcome::None
}

fn handle_mouse(ev: Event, me: MouseEvent, ctx: &mut AppContext<'_>) {
    if ctx.editor.modal.is_some() {
        if matches!(me.kind, MouseEventKind::Down(MouseButton::Left)) {
            ctx.run(&cmd::CloseModal);
        }
        return;
    }

    match me.kind {
        MouseEventKind::Down(button @ (MouseButton::Left | MouseButton::Right | MouseButton::Middle)) => {
            if button == MouseButton::Left {
                if let Some(hit) = ctx.editor.tab_strip_hit(me.column, me.row) {
                    handle_tab_strip_click(ctx, hit);
                    return;
                }
            }
            ctx.editor.focus_at_screen(me.column, me.row);
            if dispatch_to_focused_leaf(ctx, &ev) == Outcome::Consumed {
                ctx.request_redraw();
                return;
            }
            if button != MouseButton::Left {
                return;
            }
            let extend = me.modifiers.contains(KeyModifiers::SHIFT);
            ctx.run(&cmd::ClickAt {
                col: me.column,
                row: me.row,
                extend,
            });
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            ctx.run(&cmd::DragAt {
                col: me.column,
                row: me.row,
            });
        }
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            if let Some(fid) = ctx.editor.frame_at_strip(me.column, me.row) {
                if ctx.editor.tab_strip_can_scroll(fid) {
                    let delta: isize = if matches!(me.kind, MouseEventKind::ScrollUp) {
                        -2
                    } else {
                        2
                    };
                    ctx.editor.scroll_tab_strip(fid, delta);
                    ctx.request_redraw();
                    return;
                }
            }
            if dispatch_to_leaf_at(ctx, me.column, me.row, &ev) == Outcome::Consumed {
                ctx.request_redraw();
                return;
            }
            let delta: isize = if matches!(me.kind, MouseEventKind::ScrollUp) { -1 } else { 1 };
            // T-63: Effect::Run / EventSink::pulse are retired. Run
            // the scroll command synchronously; consecutive wheel
            // events arriving within the same loop iteration each
            // run `ScrollBy(±1)` directly. Coalescing across ticks
            // is implicit — multiple wheel events in one tick all
            // mutate scroll, then a single render flush at the next
            // dirty cycle paints the final position.
            ctx.run(&cmd::ScrollBy(delta));
        }
        MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
            if let Some(fid) = ctx.editor.frame_at_strip(me.column, me.row) {
                if ctx.editor.tab_strip_can_scroll(fid) {
                    let delta: isize = if matches!(me.kind, MouseEventKind::ScrollLeft) {
                        -2
                    } else {
                        2
                    };
                    ctx.editor.scroll_tab_strip(fid, delta);
                    ctx.request_redraw();
                }
            }
        }
        _ => {}
    }
}

fn handle_tab_strip_click(ctx: &mut AppContext<'_>, hit: TabStripHit) {
    let TabStripHit::Tab { frame, idx } = hit;
    ctx.editor.focus_frame(frame);
    ctx.editor.activate_tab(frame, idx);
    ctx.request_redraw();
}

fn run_arc(ctx: &mut AppContext<'_>, action: Arc<dyn EditorCommand>) {
    ctx.run(action.as_ref());
}

/// Walk the focused-leaf path and invoke `LayoutNode::handle_at` on it.
/// Returns `Ignored` if the focus path resolves to nothing.
fn dispatch_to_focused_leaf(ctx: &mut AppContext<'_>, ev: &Event) -> Outcome {
    let focus = ctx.editor.focus.clone();
    let area = ctx
        .editor
        .panes
        .at_path_with_rect(root_area(ctx), &focus)
        .map(|(rect, _)| rect)
        .unwrap_or_default();
    let Some(leaf) = ctx.editor.panes.at_path_mut(&focus) else {
        return Outcome::Ignored;
    };
    let mut hctx = HandleCtx::default();
    leaf.handle_at(ev, area, &mut hctx)
}

/// Walk the leaf at screen position (`col`, `row`) and dispatch to it.
/// Used for events whose target isn't the focused leaf — mouse-wheel
/// scroll over an unfocused sidebar.
fn dispatch_to_leaf_at(
    ctx: &mut AppContext<'_>,
    col: u16,
    row: u16,
    ev: &Event,
) -> Outcome {
    let area = root_area(ctx);
    let Some((leaf_ref, leaf_area)) = ctx
        .editor
        .panes
        .leaves_with_rects(area)
        .into_iter()
        .find(|(_, r)| col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height)
    else {
        return Outcome::Ignored;
    };
    let Some(path) = ctx.editor.panes.path_to_leaf(leaf_ref) else {
        return Outcome::Ignored;
    };
    let Some(leaf) = ctx.editor.panes.at_path_mut(&path) else {
        return Outcome::Ignored;
    };
    let mut hctx = HandleCtx::default();
    leaf.handle_at(ev, leaf_area, &mut hctx)
}

/// Reconstruct the editor's root rect from cached leaf rects: the root
/// spans the bounding box of every populated entry. Mirrors
/// `Editor::layout`'s input area without re-plumbing it here.
fn root_area(ctx: &AppContext<'_>) -> Rect {
    let mut min_x = u16::MAX;
    let mut min_y = u16::MAX;
    let mut max_x: u16 = 0;
    let mut max_y: u16 = 0;
    let mut any = false;
    for r in ctx
        .editor
        .render_cache
        .frame_rects
        .values()
        .copied()
        .chain(ctx.editor.render_cache.sidebar_rects.values().copied())
    {
        any = true;
        min_x = min_x.min(r.x);
        min_y = min_y.min(r.y);
        max_x = max_x.max(r.x.saturating_add(r.width));
        max_y = max_y.max(r.y.saturating_add(r.height));
    }
    if !any {
        return Rect::default();
    }
    Rect {
        x: min_x,
        y: min_y,
        width: max_x.saturating_sub(min_x),
        height: max_y.saturating_sub(min_y),
    }
}
