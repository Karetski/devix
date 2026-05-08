//! Pane handle types — the Lua-side userdata returned from
//! `devix.register_pane` plus the editor-side `LuaPane` /
//! `PluginPane` wrappers.
//!
//! Three concerns live here:
//!
//! 1. [`LuaPaneHandle`]: the userdata Lua holds. Carries shared state
//!    (lines, scroll, has-callback flags) and the channels needed to
//!    mutate it from inside Lua callbacks.
//! 2. [`LuaPane`]: the editor-side `Pane` impl that paints the shared
//!    line content into the sidebar slot and forwards key / click /
//!    wheel input back to the worker thread.
//! 3. [`PluginPane`]: a snapshot of the shared state, returned by
//!    `PluginRuntime::pane_for`, so the editor can build a `LuaPane`
//!    without reaching into `Contributions` directly.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use crossterm::event::{KeyEvent, MouseButton};
use mlua::{Function, RegistryKey, UserData, UserDataMethods, Value};

use devix_protocol::view::View;

use crate::geom::Rect;
use crate::pane::{HandleCtx, Outcome, Pane, RenderCtx};
use crate::Event;

use super::view_lua::view_from_lua_table;
use super::{InputSender, PluginInput, PluginMsg, next_handle, parse_lines_value, send_input};

/// Per-pane callback handles. Keys into [`PluginHost::callbacks`].
#[derive(Default)]
pub(crate) struct PaneCallbackKeys {
    pub on_key: Option<u64>,
    pub on_click: Option<u64>,
}

#[derive(Copy, Clone, Debug)]
pub(crate) enum PaneCallbackKind {
    OnKey,
    OnClick,
}

/// Userdata handed back to Lua from `devix.register_pane`. Holds the
/// shared lines / flag state plus the channels needed to mutate it
/// from inside Lua callbacks (`set_lines`, `on_key`, `on_click`,
/// `set_view`).
#[derive(Clone)]
pub struct LuaPaneHandle {
    pane_id: u64,
    lines: Arc<Mutex<Vec<String>>>,
    scroll: Arc<AtomicU16>,
    visible_rows: Arc<AtomicU16>,
    has_on_key: Arc<AtomicBool>,
    has_on_click: Arc<AtomicBool>,
    view: Arc<Mutex<Option<View>>>,
    callbacks: Arc<Mutex<HashMap<u64, RegistryKey>>>,
    pane_callbacks: Arc<Mutex<HashMap<u64, PaneCallbackKeys>>>,
    next_handle: Arc<Mutex<u64>>,
    outbox: Arc<Mutex<Vec<PluginMsg>>>,
}

impl LuaPaneHandle {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        pane_id: u64,
        lines: Arc<Mutex<Vec<String>>>,
        scroll: Arc<AtomicU16>,
        visible_rows: Arc<AtomicU16>,
        has_on_key: Arc<AtomicBool>,
        has_on_click: Arc<AtomicBool>,
        view: Arc<Mutex<Option<View>>>,
        callbacks: Arc<Mutex<HashMap<u64, RegistryKey>>>,
        pane_callbacks: Arc<Mutex<HashMap<u64, PaneCallbackKeys>>>,
        next_handle: Arc<Mutex<u64>>,
        outbox: Arc<Mutex<Vec<PluginMsg>>>,
    ) -> Self {
        Self {
            pane_id,
            lines,
            scroll,
            visible_rows,
            has_on_key,
            has_on_click,
            view,
            callbacks,
            pane_callbacks,
            next_handle,
            outbox,
        }
    }

    fn replace_lines(&self, new: Vec<String>) {
        if let Ok(mut l) = self.lines.lock() {
            *l = new;
        }
        self.notify_pane_changed();
    }

    fn notify_pane_changed(&self) {
        if let Ok(mut o) = self.outbox.lock() {
            o.push(PluginMsg::PaneChanged);
        }
    }

    pub(crate) fn set_callback(
        &self,
        kind: PaneCallbackKind,
        key: RegistryKey,
    ) -> Result<(), mlua::Error> {
        let handle = next_handle(&self.next_handle)?;
        self.callbacks
            .lock()
            .map_err(|e| mlua::Error::external(anyhow!("{e}")))?
            .insert(handle, key);
        let mut map = self
            .pane_callbacks
            .lock()
            .map_err(|e| mlua::Error::external(anyhow!("{e}")))?;
        let entry = map.entry(self.pane_id).or_default();
        match kind {
            PaneCallbackKind::OnKey => {
                entry.on_key = Some(handle);
                self.has_on_key.store(true, Ordering::Release);
            }
            PaneCallbackKind::OnClick => {
                entry.on_click = Some(handle);
                self.has_on_click.store(true, Ordering::Release);
            }
        }
        Ok(())
    }
}

impl UserData for LuaPaneHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("set_lines", |_, this, value: Value| {
            let lines = parse_lines_value(value)?;
            this.replace_lines(lines);
            Ok(())
        });
        methods.add_method("set_view", |_, this, value: Value| {
            let view_opt = match value {
                Value::Nil => None,
                Value::Table(t) => Some(view_from_lua_table(&t)?),
                other => {
                    return Err(mlua::Error::external(anyhow!(
                        "expected view as table or nil, got {:?}",
                        other,
                    )));
                }
            };
            if let Ok(mut slot) = this.view.lock() {
                *slot = view_opt;
            }
            this.notify_pane_changed();
            Ok(())
        });
        methods.add_method("on_key", |lua, this, cb: Function| {
            let key = lua.create_registry_value(cb)?;
            this.set_callback(PaneCallbackKind::OnKey, key)?;
            Ok(())
        });
        methods.add_method("on_click", |lua, this, cb: Function| {
            let key = lua.create_registry_value(cb)?;
            this.set_callback(PaneCallbackKind::OnClick, key)?;
            Ok(())
        });
        // Scroll control: the renderer applies `scroll` as a top-line
        // offset, and the mouse-wheel forwarder bumps it directly.
        // Plugins only need to call these to keep selection in view.
        methods.add_method("scroll_to", |_, this, top: u32| {
            let clamped: u16 = top.min(u16::MAX as u32) as u16;
            this.scroll.store(clamped, Ordering::Release);
            this.notify_pane_changed();
            Ok(())
        });
        methods.add_method("scroll", |_, this, ()| {
            Ok(this.scroll.load(Ordering::Acquire))
        });
        methods.add_method("visible_rows", |_, this, ()| {
            Ok(this.visible_rows.load(Ordering::Acquire))
        });
    }
}

/// Sidebar pane painted from a Lua-driven list of lines (or, if
/// the plugin pushed View IR via `pane:set_view`, from that View
/// directly). Reads the shared state on every render and forwards
/// events to the plugin thread when a callback is registered.
pub struct LuaPane {
    pub lines: Arc<Mutex<Vec<String>>>,
    pane_id: u64,
    scroll: Arc<AtomicU16>,
    visible_rows: Arc<AtomicU16>,
    has_on_key: Arc<AtomicBool>,
    has_on_click: Arc<AtomicBool>,
    pub(crate) view: Arc<Mutex<Option<View>>>,
    pub(crate) input_tx: InputSender,
}

impl LuaPane {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        pane_id: u64,
        lines: Arc<Mutex<Vec<String>>>,
        scroll: Arc<AtomicU16>,
        visible_rows: Arc<AtomicU16>,
        has_on_key: Arc<AtomicBool>,
        has_on_click: Arc<AtomicBool>,
        view: Arc<Mutex<Option<View>>>,
        input_tx: InputSender,
    ) -> Self {
        Self {
            pane_id,
            lines,
            scroll,
            visible_rows,
            has_on_key,
            has_on_click,
            view,
            input_tx,
        }
    }

    pub fn pane_id(&self) -> u64 {
        self.pane_id
    }

    pub fn has_on_key(&self) -> bool {
        self.has_on_key.load(Ordering::Acquire)
    }

    pub fn has_on_click(&self) -> bool {
        self.has_on_click.load(Ordering::Acquire)
    }

    /// Current top-line scroll offset.
    pub fn scroll(&self) -> u16 {
        self.scroll.load(Ordering::Acquire)
    }

    /// Adjust the scroll offset by `delta` rows (negative scrolls up).
    /// Clamped to `[0, max_top]`. Returns the new offset.
    pub fn scroll_by(&self, delta: i32, line_count: u16, height: u16) -> u16 {
        let max_top = line_count.saturating_sub(1);
        let max_visible_top = line_count.saturating_sub(height.max(1));
        let cap = max_visible_top.min(max_top);
        let cur = self.scroll() as i32;
        let next = (cur + delta).clamp(0, cap as i32) as u16;
        self.scroll.store(next, Ordering::Release);
        next
    }

    /// Forward a key event to the plugin's `on_key` callback, if any.
    /// Returns `true` when the event was sent.
    pub fn forward_key(&self, event: KeyEvent) -> bool {
        if !self.has_on_key() {
            return false;
        }
        send_input(
            &self.input_tx,
            PluginInput::Key {
                pane_id: self.pane_id,
                event,
            },
        )
    }

    /// Forward a click event. Same fall-through semantics as
    /// [`Self::forward_key`].
    pub fn forward_click(&self, x: u16, y: u16, button: MouseButton) -> bool {
        if !self.has_on_click() {
            return false;
        }
        send_input(
            &self.input_tx,
            PluginInput::Click {
                pane_id: self.pane_id,
                x,
                y,
                button,
            },
        )
    }
}

impl Pane for LuaPane {
    fn render(&self, area: Rect, ctx: &mut RenderCtx<'_, '_>) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        // View IR path (T-111): if the plugin pushed a view via
        // `pane:set_view`, paint that instead of the line fallback.
        let view_snapshot: Option<View> = self
            .view
            .lock()
            .ok()
            .and_then(|g| g.clone());
        if let Some(view) = view_snapshot {
            super::view_lua::paint_minimal(&view, area, ctx.frame);
            self.visible_rows.store(area.height, std::sync::atomic::Ordering::Release);
            return;
        }
        let snapshot: Vec<String> = self
            .lines
            .lock()
            .map(|l| l.clone())
            .unwrap_or_default();
        let line_count = snapshot.len();
        let max_top = line_count
            .saturating_sub(area.height.max(1) as usize)
            .min(u16::MAX as usize) as u16;
        let raw_scroll = self.scroll.load(Ordering::Acquire);
        let scroll = raw_scroll.min(max_top);
        if scroll != raw_scroll {
            self.scroll.store(scroll, Ordering::Release);
        }
        // Direct cell-write paint so this crate does not depend on
        // `ratatui::widgets`. Walk the visible window of `snapshot`,
        // truncate each line to `area.width`, and stamp it via
        // `Buffer::set_stringn` (the safe variant that respects width).
        let buf = ctx.frame.buffer_mut();
        let top = scroll as usize;
        let visible = area.height as usize;
        for row in 0..visible {
            let line_idx = top + row;
            if line_idx >= line_count {
                break;
            }
            let y = area.y + row as u16;
            let line = &snapshot[line_idx];
            buf.set_stringn(
                area.x,
                y,
                line,
                area.width as usize,
                ratatui::style::Style::default(),
            );
        }
        // Publish the painted body height back to Lua so plugins can
        // keep selection visible (`pane:visible_rows()`).
        self.visible_rows.store(area.height, Ordering::Release);
    }

    fn handle(&mut self, ev: &Event, _: Rect, _: &mut HandleCtx<'_>) -> Outcome {
        use crossterm::event::MouseEventKind;
        match ev {
            Event::Key(k) if self.has_on_key() => {
                let _ = send_input(
                    &self.input_tx,
                    PluginInput::Key {
                        pane_id: self.pane_id,
                        event: *k,
                    },
                );
                Outcome::Consumed
            }
            Event::Mouse(m) => match m.kind {
                MouseEventKind::Down(button) if self.has_on_click() => {
                    let _ = send_input(
                        &self.input_tx,
                        PluginInput::Click {
                            pane_id: self.pane_id,
                            x: m.column,
                            y: m.row,
                            button,
                        },
                    );
                    Outcome::Consumed
                }
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                    let line_count = self
                        .lines
                        .lock()
                        .map(|l| l.len())
                        .unwrap_or(0)
                        .min(u16::MAX as usize) as u16;
                    let height = self.visible_rows.load(Ordering::Acquire);
                    let delta: i32 = if matches!(m.kind, MouseEventKind::ScrollUp) {
                        -2
                    } else {
                        2
                    };
                    self.scroll_by(delta, line_count, height);
                    Outcome::Consumed
                }
                _ => Outcome::Ignored,
            },
            _ => Outcome::Ignored,
        }
    }

    fn is_focusable(&self) -> bool {
        true
    }
}

/// Snapshot of a plugin pane's shared state. Returned by
/// [`super::PluginRuntime::pane_for`] so the editor can build a
/// [`LuaPane`] without reaching into `Contributions` directly.
#[derive(Clone)]
pub struct PluginPane {
    pub pane_id: u64,
    pub lines: Arc<Mutex<Vec<String>>>,
    pub scroll: Arc<AtomicU16>,
    pub visible_rows: Arc<AtomicU16>,
    pub has_on_key: Arc<AtomicBool>,
    pub has_on_click: Arc<AtomicBool>,
    pub view: Arc<Mutex<Option<View>>>,
    pub input_tx: InputSender,
}

impl PluginPane {
    pub fn into_pane(self) -> LuaPane {
        LuaPane {
            lines: self.lines,
            pane_id: self.pane_id,
            scroll: self.scroll,
            visible_rows: self.visible_rows,
            has_on_key: self.has_on_key,
            has_on_click: self.has_on_click,
            view: self.view,
            input_tx: self.input_tx,
        }
    }

    /// Snapshot of the current line content. Convenience for tests
    /// and the few sites that want a `Vec<String>` instead of the
    /// shared `Arc<Mutex<…>>`.
    pub fn lines_snapshot(&self) -> Vec<String> {
        self.lines.lock().map(|l| l.clone()).unwrap_or_default()
    }
}
