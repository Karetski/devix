//! App-side plugin wiring.

use std::collections::HashSet;

use std::sync::Arc;

use devix_plugin::{
    CommandSpec, LuaPane, PluginMsg, PluginPane, PluginRuntime, default_plugin_path,
};
use devix_commands::{Command, CommandId, CommandRegistry, Keymap, cmd};
use devix_core::SidebarSlot;

use crate::app::App;
use crate::events::run_command;

pub struct PluginWiring {
    pub runtime: PluginRuntime,
    #[allow(dead_code)]
    pub leaked_strings: Vec<&'static str>,
}

impl PluginWiring {
    pub fn install(
        runtime: PluginRuntime,
        commands: &mut CommandRegistry,
        keymap: &mut Keymap,
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
            bind_chord_if_any(spec, id, keymap);
        }
        Self {
            runtime,
            leaked_strings: leaked,
        }
    }

    pub fn pane_for(&self, slot: SidebarSlot) -> Option<PluginPane> {
        self.runtime.pane_for(slot)
    }

    pub fn contributed_slots(&self) -> HashSet<SidebarSlot> {
        self.runtime
            .contributions()
            .panes
            .iter()
            .map(|p| p.slot)
            .collect()
    }
}

fn bind_chord_if_any(spec: &CommandSpec, id: CommandId, keymap: &mut Keymap) {
    if let Some(chord) = spec.chord {
        keymap.bind_command(chord, id);
    }
}

fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// Try to load the plugin pointed at by `DEVIX_PLUGIN`. Failures are
/// silently ignored; the editor still launches.
pub fn try_load(
    commands: &mut CommandRegistry,
    keymap: &mut Keymap,
    wakeup: Option<devix_plugin::Wakeup>,
) -> Option<PluginWiring> {
    let path = default_plugin_path()?;
    match PluginRuntime::load_with_wakeup(&path, wakeup) {
        Ok(rt) => Some(PluginWiring::install(rt, commands, keymap)),
        Err(_) => None,
    }
}

/// Drain any plugin-emitted messages and apply them to App state.
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
            PluginMsg::Status(_) => {
                // Status bar removed; plugin status messages are dropped.
            }
            PluginMsg::PaneChanged => {}
            PluginMsg::OpenPath(path) => {
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
    app.request_redraw();
}

pub fn sidebar_pane(app: &App, slot: SidebarSlot) -> Option<LuaPane> {
    app.plugins
        .as_ref()
        .and_then(|w| w.pane_for(slot))
        .map(PluginPane::into_pane)
}

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

pub fn forward_key_to_plugin(
    app: &App,
    slot: SidebarSlot,
    event: crossterm::event::KeyEvent,
) -> bool {
    sidebar_pane(app, slot)
        .map(|p| p.forward_key(event))
        .unwrap_or(false)
}

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
