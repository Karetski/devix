//! Plugin host — owns the Lua VM and the registered callback table.
//!
//! The host stays on one thread for its lifetime (the worker thread the
//! supervised runtime spawns); `Lua` itself is never reached across
//! thread boundaries. Crossing data uses the `Arc<Mutex<…>>` shared
//! state in [`Contributions`] and the editor-side handles in
//! [`super::pane_handle`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU16};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, anyhow};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton};
use mlua::{Function, Lua, LuaOptions, RegistryKey, StdLib, Table, Value};

use crate::SidebarSlot;
use crate::settings_store::SettingsStore;

use super::pane_handle::{LuaPaneHandle, PaneCallbackKeys, PaneCallbackKind};
use super::{
    CommandSpec, Contributions, PaneSpec, PluginInput, PluginMsg, key_code_to_string, next_handle,
    parse_chord, parse_lines_value,
};

pub(crate) type SharedSettingsStore = Arc<Mutex<SettingsStore>>;

/// The plugin host: owns the Lua VM and the callback registry. Stays
/// on one thread for its lifetime — never crosses thread boundaries
/// (Lua itself is reached only from the worker thread).
pub struct PluginHost {
    lua: Lua,
    /// Registry-keyed Lua callbacks. Action `run` callbacks and pane
    /// `on_key` / `on_click` all live here, indexed by a monotonic
    /// handle.
    callbacks: Arc<Mutex<HashMap<u64, RegistryKey>>>,
    /// Per-pane callback handle map.
    pane_callbacks: Arc<Mutex<HashMap<u64, PaneCallbackKeys>>>,
    /// Status / dirty / open-path messages produced by callbacks.
    /// Drained after each invoke and forwarded through the runtime's
    /// outbound channel.
    outbox: Arc<Mutex<Vec<PluginMsg>>>,
    /// Monotonic handle generator. Mutex (not atomics) so we can keep
    /// the same interior across the whole host without spreading
    /// `AtomicU64` types through the API.
    next_handle: Arc<Mutex<u64>>,
    /// Contributions accumulated by registered Lua functions during
    /// `load_file`. Cleared on every fresh `load_file`.
    contributions: Arc<Mutex<Contributions>>,
    /// Optional settings store for `devix.setting(key)` lookups +
    /// `devix.on_setting_changed(cb)` registrations. None when
    /// running in legacy/test contexts that don't share a store
    /// with the editor — Lua reads return `nil`.
    settings_store: Option<SharedSettingsStore>,
    /// Registered `devix.on_setting_changed(callback)` handles.
    /// Shared with the runtime's bus subscriber so it knows which
    /// Lua callbacks to fire when `Pulse::SettingChanged` arrives.
    setting_callbacks: Arc<Mutex<Vec<u64>>>,
}

impl PluginHost {
    pub fn new() -> Result<Self> {
        Self::new_with(None)
    }

    /// Construct with a shared settings store. The Lua bridge's
    /// `devix.setting(key)` reads through this store; an editor that
    /// owns the same `Arc<Mutex<SettingsStore>>` mutates via
    /// `SettingsStore::set` and the bus delivers `Pulse::SettingChanged`
    /// to subscribers (including the plugin runtime's bridge).
    pub fn new_with(settings_store: Option<SharedSettingsStore>) -> Result<Self> {
        let lua = Lua::new_with(StdLib::ALL_SAFE, LuaOptions::default())
            .context("creating safe Lua state")?;
        strip_dangerous_globals(&lua)?;

        let callbacks = Arc::new(Mutex::new(HashMap::<u64, RegistryKey>::new()));
        let pane_callbacks = Arc::new(Mutex::new(HashMap::<u64, PaneCallbackKeys>::new()));
        let outbox = Arc::new(Mutex::new(Vec::<PluginMsg>::new()));
        let next_handle = Arc::new(Mutex::new(1u64));
        let contributions = Arc::new(Mutex::new(Contributions::default()));

        let setting_callbacks = Arc::new(Mutex::new(Vec::<u64>::new()));

        let host = Self {
            lua,
            callbacks,
            pane_callbacks,
            outbox,
            next_handle,
            contributions,
            settings_store,
            setting_callbacks,
        };
        host.install_devix_table()?;
        Ok(host)
    }

    /// Crate-private accessor — the runtime's bus subscriber locks
    /// this list to know which registered callbacks to fire when
    /// `Pulse::SettingChanged` arrives.
    pub(crate) fn setting_callbacks(&self) -> Arc<Mutex<Vec<u64>>> {
        self.setting_callbacks.clone()
    }

    /// Snapshot the registered Lua callbacks as a path-keyed
    /// [`PluginCallbacks`] registry under `plugin` (`<name>` in
    /// `/plugin/<name>/cb/<u64>`). The returned registry shares the
    /// underlying `Arc<Mutex<…>>` with the host, so live registrations
    /// remain visible. T-56 full close.
    pub fn plugin_callbacks(&self, plugin: impl Into<String>) -> super::PluginCallbacks {
        super::PluginCallbacks::new(plugin.into(), self.callbacks.clone())
    }

    fn install_devix_table(&self) -> Result<()> {
        let lua = &self.lua;
        let devix = lua.create_table()?;

        // devix.status(text)
        {
            let outbox = self.outbox.clone();
            devix.set(
                "status",
                lua.create_function(move |_, text: String| {
                    if let Ok(mut o) = outbox.lock() {
                        o.push(PluginMsg::Status(text));
                    }
                    Ok(())
                })?,
            )?;
        }

        // devix.register_action({ id, label, chord?, run })
        {
            let callbacks = self.callbacks.clone();
            let next_handle_arc = self.next_handle.clone();
            let contributions = self.contributions.clone();
            devix.set(
                "register_action",
                lua.create_function(move |lua, table: Table| {
                    let id: String = table.get("id")?;
                    let label: String = table.get("label")?;
                    let chord_raw: Option<String> = table.get("chord")?;
                    let chord = chord_raw.as_deref().and_then(parse_chord);
                    let run: Function = table.get("run")?;

                    let key = lua.create_registry_value(run)?;
                    let handle = next_handle(&next_handle_arc)?;
                    callbacks
                        .lock()
                        .map_err(|e| mlua::Error::external(anyhow!("{e}")))?
                        .insert(handle, key);
                    contributions
                        .lock()
                        .map_err(|e| mlua::Error::external(anyhow!("{e}")))?
                        .commands
                        .push(CommandSpec { id, label, chord, handle });
                    Ok(())
                })?,
            )?;
        }

        // devix.register_pane({ slot, lines?, on_key?, on_click? }) -> LuaPaneHandle
        {
            let callbacks = self.callbacks.clone();
            let pane_callbacks = self.pane_callbacks.clone();
            let next_handle_arc = self.next_handle.clone();
            let contributions = self.contributions.clone();
            let outbox = self.outbox.clone();
            devix.set(
                "register_pane",
                lua.create_function(move |lua, table: Table| {
                    let slot_str: String = table.get("slot")?;
                    let slot = match slot_str.as_str() {
                        "left" => SidebarSlot::Left,
                        "right" => SidebarSlot::Right,
                        other => {
                            return Err(mlua::Error::external(anyhow!(
                                "unknown sidebar slot: {other}",
                            )));
                        }
                    };
                    let initial_lines = match table.get::<Value>("lines")? {
                        Value::Nil => Vec::new(),
                        v => parse_lines_value(v)?,
                    };
                    let pane_id = next_handle(&next_handle_arc)?;
                    let lines = Arc::new(Mutex::new(initial_lines));
                    let scroll = Arc::new(AtomicU16::new(0));
                    let visible_rows = Arc::new(AtomicU16::new(0));
                    let has_on_key = Arc::new(AtomicBool::new(false));
                    let has_on_click = Arc::new(AtomicBool::new(false));
                    let view: Arc<Mutex<Option<devix_protocol::view::View>>> =
                        Arc::new(Mutex::new(None));

                    let handle = LuaPaneHandle::new(
                        pane_id,
                        lines.clone(),
                        scroll.clone(),
                        visible_rows.clone(),
                        has_on_key.clone(),
                        has_on_click.clone(),
                        view.clone(),
                        callbacks.clone(),
                        pane_callbacks.clone(),
                        next_handle_arc.clone(),
                        outbox.clone(),
                    );

                    if let Some(cb) = table.get::<Option<Function>>("on_key")? {
                        let key = lua.create_registry_value(cb)?;
                        handle.set_callback(PaneCallbackKind::OnKey, key)?;
                    }
                    if let Some(cb) = table.get::<Option<Function>>("on_click")? {
                        let key = lua.create_registry_value(cb)?;
                        handle.set_callback(PaneCallbackKind::OnClick, key)?;
                    }

                    contributions
                        .lock()
                        .map_err(|e| mlua::Error::external(anyhow!("{e}")))?
                        .panes
                        .push(PaneSpec {
                            slot,
                            pane_id,
                            lines,
                            scroll,
                            visible_rows,
                            has_on_key,
                            has_on_click,
                            view,
                        });

                    Ok(handle)
                })?,
            )?;
        }

        // devix.cwd() -> string
        devix.set(
            "cwd",
            lua.create_function(|_, ()| {
                let cwd = std::env::current_dir().map_err(mlua::Error::external)?;
                Ok(cwd.to_string_lossy().into_owned())
            })?,
        )?;

        // devix.read_dir(path) -> { { name, is_dir }, ... }
        devix.set(
            "read_dir",
            lua.create_function(|lua, path: String| {
                let iter = std::fs::read_dir(&path).map_err(mlua::Error::external)?;
                let result = lua.create_table()?;
                let mut idx = 1;
                for entry in iter.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let is_dir = entry
                        .file_type()
                        .map(|t| t.is_dir())
                        .unwrap_or(false);
                    let row = lua.create_table()?;
                    row.set("name", name)?;
                    row.set("is_dir", is_dir)?;
                    result.set(idx, row)?;
                    idx += 1;
                }
                Ok(result)
            })?,
        )?;

        // devix.open_path(path) — ask the editor to open `path`.
        {
            let outbox = self.outbox.clone();
            devix.set(
                "open_path",
                lua.create_function(move |_, path: String| {
                    if let Ok(mut o) = outbox.lock() {
                        o.push(PluginMsg::OpenPath(PathBuf::from(path)));
                    }
                    Ok(())
                })?,
            )?;
        }

        // devix.setting(key) -> value | nil
        //
        // Read the current value of a setting registered through any
        // manifest's `contributes.settings`. The Lua return type
        // mirrors the resolved `SettingValue` (boolean, number,
        // string, or enum-string). Unknown keys return `nil`. T-113.
        {
            let store = self.settings_store.clone();
            devix.set(
                "setting",
                lua.create_function(move |lua, key: String| {
                    let Some(store) = store.as_ref() else {
                        return Ok(mlua::Value::Nil);
                    };
                    let guard = store.lock().map_err(|e| {
                        mlua::Error::external(anyhow!("settings store poisoned: {e}"))
                    })?;
                    match guard.get(&key) {
                        Some(value) => setting_value_to_lua(lua, value),
                        None => Ok(mlua::Value::Nil),
                    }
                })?,
            )?;
        }

        // devix.on_setting_changed(callback)
        //
        // Register a Lua function called with `(key, value)` on every
        // `Pulse::SettingChanged`. T-113.
        {
            let callbacks = self.callbacks.clone();
            let setting_callbacks = self.setting_callbacks.clone();
            let next_handle_arc = self.next_handle.clone();
            devix.set(
                "on_setting_changed",
                lua.create_function(move |lua, cb: Function| {
                    let key = lua.create_registry_value(cb)?;
                    let handle = next_handle(&next_handle_arc)?;
                    callbacks
                        .lock()
                        .map_err(|e| mlua::Error::external(anyhow!("{e}")))?
                        .insert(handle, key);
                    setting_callbacks
                        .lock()
                        .map_err(|e| mlua::Error::external(anyhow!("{e}")))?
                        .push(handle);
                    Ok(())
                })?,
            )?;
        }

        lua.globals().set("devix", devix)?;
        Ok(())
    }

    /// Run a Lua source file. Returns whatever contributions
    /// accumulated during execution.
    pub fn load_file(&self, path: &Path) -> Result<Contributions> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("reading plugin file {}", path.display()))?;
        {
            let mut c = self.contributions.lock().expect("contributions poisoned");
            c.commands.clear();
            c.panes.clear();
        }
        self.lua
            .load(&source)
            .set_name(path.to_string_lossy())
            .exec()
            .with_context(|| format!("executing plugin {}", path.display()))?;
        Ok(self.contributions.lock().expect("contributions poisoned").clone())
    }

    /// Dispatch one registered Lua callback by `handle`. Errors during
    /// the callback surface as a status message rather than
    /// propagating — a misbehaving plugin must not take down the
    /// editor.
    pub fn invoke(&self, handle: u64) {
        self.invoke_with::<()>(handle, ());
    }

    /// Like [`Self::invoke`] but passes a typed argument to the Lua
    /// callback. Used by pane input dispatch (`on_key` / `on_click`
    /// get a table describing the event).
    pub fn invoke_with<A: mlua::IntoLuaMulti>(&self, handle: u64, args: A) {
        let func: mlua::Result<Function> = {
            let cb = self.callbacks.lock().expect("callbacks poisoned");
            match cb.get(&handle) {
                Some(key) => self.lua.registry_value(key),
                None => {
                    self.push_status(format!("plugin: unknown handle {handle}"));
                    return;
                }
            }
        };
        let result = func.and_then(|f| f.call::<()>(args));
        if let Err(e) = result {
            self.push_status(format!("plugin error: {e}"));
        }
    }

    fn on_key_handle(&self, pane_id: u64) -> Option<u64> {
        self.pane_callbacks
            .lock()
            .ok()?
            .get(&pane_id)?
            .on_key
    }

    fn on_click_handle(&self, pane_id: u64) -> Option<u64> {
        self.pane_callbacks
            .lock()
            .ok()?
            .get(&pane_id)?
            .on_click
    }

    /// Translate a [`PluginInput`] into a Lua call against the pane's
    /// registered callback. Best-effort: missing callback or Lua error
    /// goes to the status line.
    pub fn dispatch_input(&self, input: PluginInput) {
        match input {
            PluginInput::Key { pane_id, event } => {
                let Some(handle) = self.on_key_handle(pane_id) else { return };
                let table = match self.key_event_table(event) {
                    Ok(t) => t,
                    Err(e) => {
                        self.push_status(format!("plugin: key event marshal error: {e}"));
                        return;
                    }
                };
                self.invoke_with(handle, table);
            }
            PluginInput::Click { pane_id, x, y, button } => {
                let Some(handle) = self.on_click_handle(pane_id) else { return };
                let table = match self.click_event_table(x, y, button) {
                    Ok(t) => t,
                    Err(e) => {
                        self.push_status(format!("plugin: click event marshal error: {e}"));
                        return;
                    }
                };
                self.invoke_with(handle, table);
            }
            PluginInput::SettingChanged { handle, key, value } => {
                let lua_value = match setting_value_to_lua(&self.lua, &value) {
                    Ok(v) => v,
                    Err(e) => {
                        self.push_status(format!("plugin: setting marshal error: {e}"));
                        return;
                    }
                };
                self.invoke_with(handle, (key, lua_value));
            }
        }
    }

    fn key_event_table(&self, ev: KeyEvent) -> mlua::Result<Table> {
        let t = self.lua.create_table()?;
        t.set("key", key_code_to_string(ev.code))?;
        if let KeyCode::Char(c) = ev.code {
            t.set("char", c.to_string())?;
        }
        t.set("ctrl", ev.modifiers.contains(KeyModifiers::CONTROL))?;
        t.set("shift", ev.modifiers.contains(KeyModifiers::SHIFT))?;
        t.set("alt", ev.modifiers.contains(KeyModifiers::ALT))?;
        t.set("super", ev.modifiers.contains(KeyModifiers::SUPER))?;
        Ok(t)
    }

    fn click_event_table(
        &self,
        x: u16,
        y: u16,
        button: MouseButton,
    ) -> mlua::Result<Table> {
        let t = self.lua.create_table()?;
        t.set("x", x)?;
        t.set("y", y)?;
        t.set(
            "button",
            match button {
                MouseButton::Left => "left",
                MouseButton::Right => "right",
                MouseButton::Middle => "middle",
            },
        )?;
        Ok(t)
    }

    fn push_status(&self, msg: String) {
        if let Ok(mut o) = self.outbox.lock() {
            o.push(PluginMsg::Status(msg));
        }
    }

    /// Drain queued messages produced by Lua callbacks.
    pub fn drain_messages(&self) -> Vec<PluginMsg> {
        match self.outbox.lock() {
            Ok(mut o) => std::mem::take(&mut *o),
            Err(_) => Vec::new(),
        }
    }

    /// Crate-private accessor for the underlying `Lua` state. Used by
    /// the plugin module's tests to assert sandboxing properties
    /// (e.g. that `io` / `os` / `package` are nil-ed out at startup).
    /// Production callers go through the typed `devix.*` table.
    #[cfg(test)]
    pub(crate) fn lua(&self) -> &Lua {
        &self.lua
    }
}

/// Marshal a [`SettingValue`] into a Lua value. Booleans become
/// `Boolean`, numbers become `Number`, strings (and enum strings)
/// become `String`. Used by both `devix.setting(key)` lookups and
/// the `devix.on_setting_changed(cb)` dispatch path.
fn setting_value_to_lua(
    lua: &Lua,
    value: &devix_protocol::manifest::SettingValue,
) -> mlua::Result<mlua::Value> {
    use devix_protocol::manifest::SettingValue;
    match value {
        SettingValue::Boolean(b) => Ok(mlua::Value::Boolean(*b)),
        SettingValue::Number(n) => Ok(mlua::Value::Number(*n)),
        SettingValue::String(s) | SettingValue::EnumString(s) => {
            Ok(mlua::Value::String(lua.create_string(s)?))
        }
    }
}

fn strip_dangerous_globals(lua: &Lua) -> Result<()> {
    let g = lua.globals();
    for key in [
        "dofile",
        "loadfile",
        "load",
        "loadstring",
        "require",
        "package",
        "io",
        "os",
        "debug",
        "collectgarbage",
    ] {
        g.set(key, Value::Nil)?;
    }
    Ok(())
}
