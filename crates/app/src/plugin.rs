//! App-side plugin wiring.
//!
//! Mirrors `crate::lsp` in spirit: own a long-lived runtime handle, drain
//! its inbound channel each tick, and surface events on `App` state.
//!
//! The plugin runtime itself (Lua VM, callbacks) lives in `devix-plugin`
//! on a dedicated thread. This module is just the editor-side glue:
//!
//! - Discover a plugin file from `DEVIX_PLUGIN`.
//! - Register each contributed [`CommandSpec`] into the editor's
//!   [`CommandRegistry`] as a [`devix_plugin::LuaAction`], and bind any
//!   chord the plugin asked for into the [`Keymap`].
//! - Stash sidebar contributions on `App.plugins` so the renderer can
//!   pull them out when building [`SidebarSlotPane`] content.
//! - Drain status messages each tick into the status line.
//!
//! Plugin failures (parse error, missing file, panic in callback) are
//! best-effort: surfaced on the status line, never fatal.

use std::collections::HashSet;

use std::sync::Arc;

use devix_plugin::{
    CommandSpec, LuaPane, PluginMsg, PluginPane, PluginRuntime, default_plugin_path,
    parse_chord,
};
use devix_surface::{Command, CommandId, CommandRegistry, Keymap, SidebarSlot, cmd};

use crate::app::App;
use crate::events::run_command;

/// Wraps the plugin runtime plus the static command-id storage backing
/// each registered Lua command. `CommandId` holds a `&'static str`, so
/// every plugin id has to be leaked once at registration time; we keep
/// the leaked strings here as a single owned bag for symmetry with
/// other long-lived editor state.
pub struct PluginWiring {
    pub runtime: PluginRuntime,
    /// Leaked `Box<str>` backing the `CommandId(&'static str)` values
    /// installed for plugin commands. Held only so a future
    /// `unregister` can free them; today we never unregister.
    #[allow(dead_code)]
    pub leaked_strings: Vec<&'static str>,
}

impl PluginWiring {
    pub fn install(
        runtime: PluginRuntime,
        commands: &mut CommandRegistry,
        keymap: &mut Keymap,
        status: &mut devix_surface::StatusLine,
    ) -> Self {
        let mut leaked = Vec::new();
        let sender = runtime.invoke_sender();
        for spec in &runtime.contributions().commands {
            let id_static: &'static str = leak_str(&spec.id);
            let label_static: &'static str = leak_str(&spec.label);
            leaked.push(id_static);
            leaked.push(label_static);

            let id = CommandId(id_static);
            let action = devix_plugin::make_command_action(spec, sender.clone());
            commands.register(Command {
                id,
                label: label_static,
                category: Some("Plugin"),
                action,
            });
            bind_chord_if_any(spec, id, keymap, status);
        }
        Self {
            runtime,
            leaked_strings: leaked,
        }
    }

    /// Snapshot of the lines a plugin contributed for `slot`, if any.
    /// Returned shape matches what the render pass needs for the
    /// `SidebarSlotPane.content` field.
    pub fn pane_for(&self, slot: SidebarSlot) -> Option<PluginPane> {
        self.runtime.pane_for(slot)
    }

    /// Distinct sidebar slots this plugin contributed to. Used by the
    /// App to auto-open them on startup so users don't have to discover
    /// the toggle chord to see plugin content.
    pub fn contributed_slots(&self) -> HashSet<SidebarSlot> {
        self.runtime
            .contributions()
            .panes
            .iter()
            .map(|p| p.slot)
            .collect()
    }
}

fn bind_chord_if_any(
    spec: &CommandSpec,
    id: CommandId,
    keymap: &mut Keymap,
    status: &mut devix_surface::StatusLine,
) {
    let Some(raw) = spec.chord.as_deref() else { return };
    match parse_chord(raw) {
        Some(chord) => keymap.bind_command(chord, id),
        None => status.set(format!(
            "plugin {}: cannot parse chord {raw:?}; command stays palette-only",
            spec.id,
        )),
    }
}

fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// Try to load the plugin pointed at by `DEVIX_PLUGIN`. Failures are
/// reported on the status line; the editor still launches.
pub fn try_load(
    commands: &mut CommandRegistry,
    keymap: &mut Keymap,
    status: &mut devix_surface::StatusLine,
    wakeup: Option<devix_plugin::Wakeup>,
) -> Option<PluginWiring> {
    let path = default_plugin_path()?;
    match PluginRuntime::load_with_wakeup(&path, wakeup) {
        Ok(rt) => {
            let cmds = rt.contributions().commands.len();
            let panes = rt.contributions().panes.len();
            let label = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
            status.set(format!(
                "plugin loaded: {label} ({cmds} action{}, {panes} pane{})",
                if cmds == 1 { "" } else { "s" },
                if panes == 1 { "" } else { "s" },
            ));
            Some(PluginWiring::install(rt, commands, keymap, status))
        }
        Err(e) => {
            status.set(format!("plugin load failed ({}): {e}", path.display()));
            None
        }
    }
}

/// Drain any plugin-emitted messages and apply them to App state.
/// Mirrors `drain_lsp_events` / `drain_disk_events`. Marks `app.dirty`
/// when state visible to the renderer changes.
pub fn drain_plugin_events(app: &mut App) {
    let msgs = match app.plugins.as_mut() {
        Some(wiring) => wiring.runtime.drain_messages(),
        None => return,
    };
    if msgs.is_empty() {
        return;
    }
    for m in msgs {
        match m {
            PluginMsg::Status(text) => app.status.set(text),
            PluginMsg::PaneChanged => {
                // Render reads the live `Arc<Mutex<Vec<String>>>`;
                // setting dirty just wakes the loop.
            }
            PluginMsg::OpenPath(path) => {
                // `OpenPath` targets the active *frame*. When the user
                // opens a file from a plugin sidebar, focus is on the
                // sidebar leaf and `active_frame()` returns None, which
                // would make the open fail silently. Bounce focus to
                // the first available editor frame first so the file
                // lands somewhere visible.
                if app.surface.active_frame().is_none() {
                    if let Some(fid) =
                        devix_surface::frame_ids(app.surface.root.as_ref()).first().copied()
                    {
                        app.surface.focus_frame(fid);
                    }
                }
                run_command(app, Arc::new(cmd::OpenPath(path)));
            }
        }
    }
    app.dirty = true;
}

/// Build the sidebar pane content for `slot` from the loaded plugin,
/// if it contributed one. Called by `render::build_sidebar_pane`.
pub fn sidebar_pane(app: &App, slot: SidebarSlot) -> Option<LuaPane> {
    app.plugins
        .as_ref()
        .and_then(|w| w.pane_for(slot))
        .map(PluginPane::into_pane)
}

/// Resolve `app.surface.focus` to a sidebar slot the plugin contributed
/// a pane to. Returns `None` if focus isn't on a plugin sidebar.
pub fn focused_plugin_slot(app: &App) -> Option<SidebarSlot> {
    let leaf = devix_surface::pane_at_indices(app.surface.root.as_ref(), &app.surface.focus)
        .and_then(devix_surface::pane_leaf_id)?;
    let devix_surface::LeafRef::Sidebar(slot) = leaf else { return None };
    if app.plugins.as_ref()?.contributed_slots().contains(&slot) {
        Some(slot)
    } else {
        None
    }
}

/// Forward a key event to the plugin pane bound to `slot`, if the
/// plugin registered an `on_key` callback. Returns `true` when the
/// event was sent (the input dispatcher should treat it as consumed).
pub fn forward_key_to_plugin(
    app: &App,
    slot: SidebarSlot,
    event: crossterm::event::KeyEvent,
) -> bool {
    sidebar_pane(app, slot)
        .map(|p| p.forward_key(event))
        .unwrap_or(false)
}

/// Mouse counterpart of [`forward_key_to_plugin`].
pub fn forward_click_to_plugin(
    app: &App,
    slot: SidebarSlot,
    x: u16,
    y: u16,
    button: crossterm::event::MouseButton,
) -> bool {
    sidebar_pane(app, slot)
        .map(|p| p.forward_click(x, y, button))
        .unwrap_or(false)
}

/// Find the sidebar slot the mouse is hovering over (if any) and
/// whether the plugin contributed a pane to it. Used by wheel-scroll
/// handling so we can route scroll events into a plugin pane that
/// isn't currently focused.
pub fn plugin_slot_at(app: &App, col: u16, row: u16) -> Option<SidebarSlot> {
    let plugin_slots = app.plugins.as_ref()?.contributed_slots();
    for (slot, rect) in &app.surface.render_cache.sidebar_rects {
        if !plugin_slots.contains(slot) {
            continue;
        }
        if col >= rect.x
            && col < rect.x + rect.width
            && row >= rect.y
            && row < rect.y + rect.height
        {
            return Some(*slot);
        }
    }
    None
}

/// Bump the scroll offset of the plugin pane bound to `slot` by
/// `delta` rows. Returns `true` when the pane existed and was scrolled
/// (caller should mark dirty); `false` when no plugin pane lives in
/// the slot. Uses the renderer-published `visible_rows` so the cap is
/// always correct even when the sidebar resizes.
pub fn scroll_plugin_pane(app: &App, slot: SidebarSlot, delta: i32) -> bool {
    let Some(snapshot) = app.plugins.as_ref().and_then(|w| w.pane_for(slot)) else {
        return false;
    };
    let line_count = snapshot
        .lines
        .lock()
        .map(|l| l.len())
        .unwrap_or(0)
        .min(u16::MAX as usize) as u16;
    let height = snapshot
        .visible_rows
        .load(std::sync::atomic::Ordering::Acquire);
    let pane = snapshot.into_pane();
    pane.scroll_by(delta, line_count, height);
    true
}

