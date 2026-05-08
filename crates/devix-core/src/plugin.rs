//! Plugin module — formerly `devix-plugin`. Lua plugin host.
//!
//! A small mlua-backed runtime that loads one Lua file at startup and
//! collects three kinds of contributions against the editor's
//! existing surfaces:
//!
//! 1. **Actions** (`devix.register_action`): plugin-supplied commands
//!    that flow through the regular `CommandRegistry` and `Keymap`.
//! 2. **Panes** (`devix.register_pane`): plugin-supplied content drawn
//!    into a sidebar slot (`left` / `right`). The call returns a
//!    [`LuaPaneHandle`] userdata so the plugin can mutate the pane's
//!    line content (`pane:set_lines(...)`) and register input callbacks
//!    (`pane:on_key(...)`, `pane:on_click(...)`) after load.
//! 3. **Open requests** (`devix.open_path`): the plugin can ask the
//!    editor to open a file path, routed through the existing
//!    `cmd::OpenPath` action editor.
//!
//! ## Threading
//!
//! [`PluginRuntime::load`] owns a dedicated OS thread running a
//! `current_thread` tokio runtime; the Lua VM lives entirely on that
//! thread. The editor's render and input loops never touch Lua. Three
//! channels cross the boundary:
//!
//! - `invoke_tx → invoke_rx: u64`         — fire one action callback.
//! - `input_tx  → input_rx : PluginInput` — deliver one Pane input.
//! - `msg_tx    ← msg_rx   : PluginMsg`   — host messages back to the
//!   editor (status, dirty, open path).
//!
//! Lua-side state (lines, callbacks) is shared with the editor through
//! `Arc<Mutex<...>>` and `Arc<AtomicBool>` — these are containers; the
//! `Lua` itself is never reached from another thread.
//!
//! ## Sandboxing
//!
//! [`PluginHost::new`] starts from `StdLib::ALL_SAFE` (omits `os` /
//! `io` / `package` / `debug` / `ffi`) and additionally nils out
//! `dofile`, `loadfile`, `load`, `loadstring`, `require`, and the
//! matching globals so plugins cannot escape the sandbox by re-loading
//! code from disk. New Rust→Lua entry points always go in the `devix`
//! table; the bare globals are not extended.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, anyhow};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton};
use crate::editor::{
    Chord, Command, CommandId, CommandRegistry, Context, EditorCommand, Editor, Keymap,
};
use crate::{Action, Event, HandleCtx, Outcome, Pane, Rect, RenderCtx, SidebarSlot};
use mlua::{Function, Lua, LuaOptions, RegistryKey, StdLib, Table, UserData,
    UserDataMethods, Value};
use tokio::sync::mpsc::{
    UnboundedReceiver, UnboundedSender, error::TryRecvError, unbounded_channel,
};

/// One command contributed by a Lua plugin. Pure data — the Lua callback
/// itself stays inside the host, keyed by `handle`.
#[derive(Clone, Debug)]
pub struct CommandSpec {
    pub id: String,
    pub label: String,
    /// Optional chord pre-parsed at registration time. The Lua side
    /// passes a string ("ctrl+shift+p"); the plugin host runs it through
    /// [`parse_chord`] before producing the spec, so consumers (app
    /// keymap binding) do not need to re-parse. `None` means "no chord
    /// bound" — either Lua passed nothing or the string was unparseable
    /// (the unparseable case is silent for now; future host versions
    /// may editor it as a load-time warning).
    pub chord: Option<Chord>,
    pub handle: u64,
}

/// One sidebar contribution. The `lines` and `scroll` storage is
/// shared with the renderer — `pane:set_lines(...)` from Lua mutates
/// the same `Vec`, and the renderer / mouse-wheel forwarder mutate the
/// same scroll offset. `visible_rows` is read-only from Lua's
/// perspective: the renderer writes the last-painted body height so
/// plugins can keep selection in view.
#[derive(Clone, Debug)]
pub struct PaneSpec {
    pub slot: SidebarSlot,
    pub pane_id: u64,
    pub lines: Arc<Mutex<Vec<String>>>,
    pub scroll: Arc<AtomicU16>,
    pub visible_rows: Arc<AtomicU16>,
    pub has_on_key: Arc<AtomicBool>,
    pub has_on_click: Arc<AtomicBool>,
}

#[derive(Clone, Debug, Default)]
pub struct Contributions {
    pub commands: Vec<CommandSpec>,
    pub panes: Vec<PaneSpec>,
}

/// Outbound messages from the plugin thread to the editor.
#[derive(Clone, Debug)]
pub enum PluginMsg {
    /// Push a string onto the status line.
    Status(String),
    /// A `pane:set_lines` call mutated render-visible state; the editor
    /// should redraw on the next tick.
    PaneChanged,
    /// Plugin asked to open `path` in the editor.
    OpenPath(PathBuf),
}

/// Inbound input forwarded to the plugin thread for delivery to a
/// pane's registered callback. The editor's input loop builds these
/// when a focused plugin pane should see the event.
#[derive(Clone, Debug)]
pub enum PluginInput {
    Key { pane_id: u64, event: KeyEvent },
    Click { pane_id: u64, x: u16, y: u16, button: MouseButton },
}

/// Per-pane callback handles. Keys into [`PluginHost::callbacks`].
#[derive(Default)]
struct PaneCallbackKeys {
    on_key: Option<u64>,
    on_click: Option<u64>,
}

/// Userdata handed back to Lua from `devix.register_pane`. Holds the
/// shared lines / flag state plus the channels needed to mutate it from
/// inside Lua callbacks (`set_lines`, `on_key`, `on_click`).
#[derive(Clone)]
pub struct LuaPaneHandle {
    pane_id: u64,
    lines: Arc<Mutex<Vec<String>>>,
    scroll: Arc<AtomicU16>,
    visible_rows: Arc<AtomicU16>,
    has_on_key: Arc<AtomicBool>,
    has_on_click: Arc<AtomicBool>,
    callbacks: Arc<Mutex<HashMap<u64, RegistryKey>>>,
    pane_callbacks: Arc<Mutex<HashMap<u64, PaneCallbackKeys>>>,
    next_handle: Arc<Mutex<u64>>,
    outbox: Arc<Mutex<Vec<PluginMsg>>>,
}

impl LuaPaneHandle {
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

    fn set_callback(&self, kind: PaneCallbackKind, key: RegistryKey) -> Result<(), mlua::Error> {
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

#[derive(Copy, Clone, Debug)]
enum PaneCallbackKind {
    OnKey,
    OnClick,
}

impl UserData for LuaPaneHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("set_lines", |_, this, value: Value| {
            let lines = parse_lines_value(value)?;
            this.replace_lines(lines);
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

fn next_handle(counter: &Arc<Mutex<u64>>) -> mlua::Result<u64> {
    let mut h = counter
        .lock()
        .map_err(|e| mlua::Error::external(anyhow!("{e}")))?;
    let cur = *h;
    *h += 1;
    Ok(cur)
}

/// The plugin host: owns the Lua VM and the callback registry. Stays on
/// one thread for its lifetime — never crosses thread boundaries (Lua
/// itself is reached only from the worker thread).
pub struct PluginHost {
    lua: Lua,
    /// Registry-keyed Lua callbacks. Action `run` callbacks and pane
    /// `on_key` / `on_click` all live here, indexed by a monotonic
    /// handle.
    callbacks: Arc<Mutex<HashMap<u64, RegistryKey>>>,
    /// Per-pane callback handle map.
    pane_callbacks: Arc<Mutex<HashMap<u64, PaneCallbackKeys>>>,
    /// Status / dirty / open-path messages produced by callbacks. Drained
    /// after each invoke and forwarded through [`PluginRuntime`]'s
    /// outbound channel.
    outbox: Arc<Mutex<Vec<PluginMsg>>>,
    /// Monotonic handle generator. Mutex (not atomics) so we can keep
    /// the same interior across the whole host without spreading
    /// `AtomicU64` types through the API.
    next_handle: Arc<Mutex<u64>>,
    /// Contributions accumulated by registered Lua functions during
    /// `load_file`. Cleared on every fresh `load_file`.
    contributions: Arc<Mutex<Contributions>>,
}

impl PluginHost {
    pub fn new() -> Result<Self> {
        let lua = Lua::new_with(StdLib::ALL_SAFE, LuaOptions::default())
            .context("creating safe Lua state")?;
        strip_dangerous_globals(&lua)?;

        let callbacks = Arc::new(Mutex::new(HashMap::<u64, RegistryKey>::new()));
        let pane_callbacks = Arc::new(Mutex::new(HashMap::<u64, PaneCallbackKeys>::new()));
        let outbox = Arc::new(Mutex::new(Vec::<PluginMsg>::new()));
        let next_handle = Arc::new(Mutex::new(1u64));
        let contributions = Arc::new(Mutex::new(Contributions::default()));

        let host = Self {
            lua,
            callbacks,
            pane_callbacks,
            outbox,
            next_handle,
            contributions,
        };
        host.install_devix_table()?;
        Ok(host)
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
            let next_handle = self.next_handle.clone();
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
                    let handle = next_handle_locked(&next_handle)?;
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
        //
        // Returns a userdata the plugin keeps to mutate the pane after
        // load. The optional callbacks in the constructor table are a
        // convenience: equivalent to the explicit `pane:on_key(...)` /
        // `pane:on_click(...)` methods on the returned handle.
        {
            let callbacks = self.callbacks.clone();
            let pane_callbacks = self.pane_callbacks.clone();
            let next_handle = self.next_handle.clone();
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
                    let pane_id = next_handle_locked(&next_handle)?;
                    let lines = Arc::new(Mutex::new(initial_lines));
                    let scroll = Arc::new(AtomicU16::new(0));
                    let visible_rows = Arc::new(AtomicU16::new(0));
                    let has_on_key = Arc::new(AtomicBool::new(false));
                    let has_on_click = Arc::new(AtomicBool::new(false));

                    let handle = LuaPaneHandle {
                        pane_id,
                        lines: lines.clone(),
                        scroll: scroll.clone(),
                        visible_rows: visible_rows.clone(),
                        has_on_key: has_on_key.clone(),
                        has_on_click: has_on_click.clone(),
                        callbacks: callbacks.clone(),
                        pane_callbacks: pane_callbacks.clone(),
                        next_handle: next_handle.clone(),
                        outbox: outbox.clone(),
                    };

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
        //
        // Read-only filesystem access. The Lua-level `io` library is
        // stripped, but plugins legitimately need to enumerate the
        // workspace to render trees / pickers; this is the narrow,
        // sandboxed entry point.
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

        // devix.open_path(path) — ask the editor to open `path` through
        // the existing `OpenPath` action editor. Pure outbound message;
        // the editor decides where to display it.
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

        lua.globals().set("devix", devix)?;
        Ok(())
    }

    /// Run a Lua source file. Returns whatever contributions accumulated
    /// during execution.
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
    /// the callback are surfaced as a status message rather than
    /// propagating — a misbehaving plugin must not take down the editor.
    pub fn invoke(&self, handle: u64) {
        self.invoke_with::<()>(handle, ());
    }

    /// Like [`invoke`] but passes a typed argument to the Lua callback.
    /// Used by pane input dispatch (`on_key` / `on_click` get a table
    /// describing the event).
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

    /// Look up the on-key handle for a pane, if registered.
    fn on_key_handle(&self, pane_id: u64) -> Option<u64> {
        self.pane_callbacks
            .lock()
            .ok()?
            .get(&pane_id)?
            .on_key
    }

    /// Look up the on-click handle for a pane, if registered.
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
    fn dispatch_input(&self, input: PluginInput) {
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
}

fn parse_lines_value(value: Value) -> mlua::Result<Vec<String>> {
    match value {
        Value::Nil => Ok(Vec::new()),
        Value::String(s) => Ok(vec![s.to_str()?.to_string()]),
        Value::Table(t) => {
            let mut out = Vec::new();
            for v in t.sequence_values::<String>() {
                out.push(v?);
            }
            Ok(out)
        }
        other => Err(mlua::Error::external(anyhow!(
            "expected lines as string or table of strings, got {:?}",
            other
        ))),
    }
}

fn next_handle_locked(counter: &Arc<Mutex<u64>>) -> mlua::Result<u64> {
    next_handle(counter)
}

fn key_code_to_string(code: KeyCode) -> String {
    match code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "enter".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::BackTab => "backtab".into(),
        KeyCode::Esc => "esc".into(),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::Home => "home".into(),
        KeyCode::End => "end".into(),
        KeyCode::PageUp => "pageup".into(),
        KeyCode::PageDown => "pagedown".into(),
        KeyCode::Backspace => "backspace".into(),
        KeyCode::Delete => "delete".into(),
        KeyCode::Insert => "insert".into(),
        KeyCode::F(n) => format!("f{n}"),
        other => format!("{other:?}").to_lowercase(),
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

/// Push-callback for plugin messages. Production callers pass one of
/// these to [`PluginRuntime::load_with_sink`]; the worker thread invokes
/// it directly for every emitted [`PluginMsg`], so the editor's run loop
/// never has to drain a queue. T-63 retired the prior `Wakeup` hook —
/// the MsgSink itself is the wake mechanism (it publishes onto the
/// editor's bus and pings the main loop directly).
pub type MsgSink = Arc<dyn Fn(PluginMsg) + Send + Sync + 'static>;

/// Plugin runtime: owns the host on a dedicated thread and exposes
/// channel handles the editor uses to dispatch invokes / forward input
/// / drain status.
pub struct PluginRuntime {
    invoke_tx: UnboundedSender<u64>,
    input_tx: UnboundedSender<PluginInput>,
    msg_rx: UnboundedReceiver<PluginMsg>,
    contributions: Contributions,
    /// Strings leaked to satisfy the `'static` lifetime on
    /// `CommandId(&'static str)` / `Command::label`. Lives as long as the
    /// runtime so registered commands stay valid.
    #[allow(dead_code)]
    leaked_strings: Vec<&'static str>,
    /// Held only to keep the worker thread alive for the lifetime of
    /// the runtime; dropped on shutdown so the receiver gets `None` and
    /// the loop exits.
    #[allow(dead_code)]
    join: Option<std::thread::JoinHandle<()>>,
}

impl PluginRuntime {
    /// Load without a push-sink. Messages buffer on an internal
    /// queue; consumers drain via [`PluginRuntime::drain_messages`].
    /// Kept for tests; production uses [`load_with_sink`].
    pub fn load(path: &Path) -> Result<Self> {
        Self::load_full(path, None)
    }

    /// Load with a push-callback. Every emitted [`PluginMsg`] is handed
    /// directly to `sink` from the plugin worker thread; nothing is
    /// buffered on this side. Production path.
    pub fn load_with_sink(path: &Path, sink: MsgSink) -> Result<Self> {
        Self::load_full(path, Some(sink))
    }

    fn load_full(
        path: &Path,
        msg_sink: Option<MsgSink>,
    ) -> Result<Self> {
        let (invoke_tx, mut invoke_rx) = unbounded_channel::<u64>();
        let (input_tx, mut input_rx) = unbounded_channel::<PluginInput>();
        let (msg_tx, msg_rx) = unbounded_channel::<PluginMsg>();
        let (init_tx, init_rx) = std::sync::mpsc::channel::<Result<Contributions>>();

        let path = path.to_path_buf();
        let join = std::thread::Builder::new()
            .name("devix-plugin".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = init_tx.send(Err(anyhow!(e)));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let host = match PluginHost::new() {
                        Ok(h) => h,
                        Err(e) => {
                            let _ = init_tx.send(Err(e));
                            return;
                        }
                    };
                    let contributions = match host.load_file(&path) {
                        Ok(c) => c,
                        Err(e) => {
                            let _ = init_tx.send(Err(e));
                            return;
                        }
                    };
                    forward_messages(&host, &msg_tx, msg_sink.as_ref());
                    if init_tx.send(Ok(contributions)).is_err() {
                        return;
                    }
                    loop {
                        tokio::select! {
                            maybe_handle = invoke_rx.recv() => {
                                match maybe_handle {
                                    Some(handle) => host.invoke(handle),
                                    None => break,
                                }
                            }
                            maybe_input = input_rx.recv() => {
                                match maybe_input {
                                    Some(input) => host.dispatch_input(input),
                                    None => break,
                                }
                            }
                        }
                        forward_messages(&host, &msg_tx, msg_sink.as_ref());
                    }
                });
            })
            .context("spawning plugin host thread")?;

        let contributions = init_rx
            .recv()
            .context("plugin host thread exited before reporting load result")??;
        Ok(Self {
            invoke_tx,
            input_tx,
            msg_rx,
            contributions,
            leaked_strings: Vec::new(),
            join: Some(join),
        })
    }

    pub fn contributions(&self) -> &Contributions {
        &self.contributions
    }

    pub fn invoke_sender(&self) -> UnboundedSender<u64> {
        self.invoke_tx.clone()
    }

    pub fn input_sender(&self) -> UnboundedSender<PluginInput> {
        self.input_tx.clone()
    }

    /// Drain any messages currently buffered. Non-blocking.
    pub fn drain_messages(&mut self) -> Vec<PluginMsg> {
        let mut out = Vec::new();
        loop {
            match self.msg_rx.try_recv() {
                Ok(m) => out.push(m),
                Err(TryRecvError::Empty) => return out,
                Err(TryRecvError::Disconnected) => return out,
            }
        }
    }

    /// Wire this runtime's contributions into the editor:
    /// - register every contributed command in `commands`,
    /// - bind every contributed chord in `keymap`,
    /// - install every contributed pane onto its sidebar slot in
    ///   `editor` (toggling the slot open if needed).
    ///
    /// Run once at startup before the run loop. After this returns, the
    /// editor's command registry, keymap, and structural Pane tree all
    /// know about the plugin; the host doesn't need any plugin-specific
    /// indirection beyond owning the runtime so messages keep draining.
    pub fn install(
        &mut self,
        commands: &mut CommandRegistry,
        keymap: &mut Keymap,
        editor: &mut Editor,
    ) {
        let sender = self.invoke_tx.clone();
        for spec in &self.contributions.commands {
            let id_static: &'static str = leak_str(&spec.id);
            let label_static: &'static str = leak_str(&spec.label);
            self.leaked_strings.push(id_static);
            self.leaked_strings.push(label_static);

            let id = CommandId(id_static);
            let action = make_command_action(spec, sender.clone());
            commands.register(Command {
                id,
                label: label_static,
                category: Some("Plugin"),
                action,
            });
            if let Some(chord) = spec.chord {
                keymap.bind_command(chord, id);
            }
        }
        // Snapshot the slots so we don't borrow `self.contributions` and
        // `self.input_tx` simultaneously.
        let pane_specs: Vec<(SidebarSlot, PaneSpec)> = self
            .contributions
            .panes
            .iter()
            .map(|p| (p.slot, p.clone()))
            .collect();
        for (slot, spec) in pane_specs {
            let pane = LuaPane {
                pane_id: spec.pane_id,
                lines: spec.lines.clone(),
                scroll: spec.scroll.clone(),
                visible_rows: spec.visible_rows.clone(),
                has_on_key: spec.has_on_key.clone(),
                has_on_click: spec.has_on_click.clone(),
                input_tx: self.input_tx.clone(),
            };
            editor.install_sidebar_pane(slot, Box::new(pane));
        }
    }

    /// Pane handle for `slot`, if the plugin contributed one. The
    /// returned [`PluginPane`] holds clones of the shared state — the
    /// renderer reads `lines` and `scroll` each frame, the input
    /// forwarder calls `input_tx`.
    pub fn pane_for(&self, slot: SidebarSlot) -> Option<PluginPane> {
        let spec = self
            .contributions
            .panes
            .iter()
            .find(|p| p.slot == slot)?;
        Some(PluginPane {
            pane_id: spec.pane_id,
            lines: spec.lines.clone(),
            scroll: spec.scroll.clone(),
            visible_rows: spec.visible_rows.clone(),
            has_on_key: spec.has_on_key.clone(),
            has_on_click: spec.has_on_click.clone(),
            input_tx: self.input_tx.clone(),
        })
    }
}

fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn forward_messages(
    host: &PluginHost,
    msg_tx: &UnboundedSender<PluginMsg>,
    msg_sink: Option<&MsgSink>,
) {
    for msg in host.drain_messages() {
        match msg_sink {
            // Direct push: hand the message straight to the host's run
            // loop. The MsgSink itself wakes the main loop (T-63
            // retired the separate Wakeup hook).
            Some(sink) => sink(msg),
            // Buffered fallback: queue for `drain_messages`. Tests
            // poll the queue directly.
            None => {
                if msg_tx.send(msg).is_err() {
                    break;
                }
            }
        }
    }
}

/// Action wrapper: a plugin-contributed command stored in the editor's
/// regular `CommandRegistry`. Invoking it sends a handle to the plugin
/// thread; the actual Lua work runs there.
pub struct LuaAction {
    handle: u64,
    sender: UnboundedSender<u64>,
}

impl LuaAction {
    pub fn new(handle: u64, sender: UnboundedSender<u64>) -> Self {
        Self { handle, sender }
    }
}

impl<'a> Action<Context<'a>> for LuaAction {
    fn invoke(&self, _ctx: &mut Context<'a>) {
        let _ = self.sender.send(self.handle);
    }
}

/// Storage-typed alias so palette / keymap call sites can hold these
/// behind `Arc<dyn EditorCommand>` like every other command.
pub type PluginCommandAction = Arc<dyn EditorCommand>;

pub fn make_command_action(spec: &CommandSpec, sender: UnboundedSender<u64>) -> PluginCommandAction {
    Arc::new(LuaAction::new(spec.handle, sender))
}

/// Sidebar pane painted from a Lua-driven list of lines. Reads the
/// shared `lines` storage on every render and forwards events to the
/// plugin thread when a callback is registered.
pub struct LuaPane {
    pub lines: Arc<Mutex<Vec<String>>>,
    pane_id: u64,
    scroll: Arc<AtomicU16>,
    visible_rows: Arc<AtomicU16>,
    has_on_key: Arc<AtomicBool>,
    has_on_click: Arc<AtomicBool>,
    input_tx: UnboundedSender<PluginInput>,
}

impl LuaPane {
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
    /// Clamped to `[0, max_top]` where `max_top` is the highest offset
    /// that still leaves at least one line on screen given the visible
    /// `height` and total `line_count`. Returns the new offset.
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
    /// Returns `true` when the event was sent (the plugin claims it),
    /// `false` when no callback is registered (caller should fall
    /// through).
    pub fn forward_key(&self, event: KeyEvent) -> bool {
        if !self.has_on_key() {
            return false;
        }
        let _ = self.input_tx.send(PluginInput::Key {
            pane_id: self.pane_id,
            event,
        });
        true
    }

    /// Forward a click event. Same fall-through semantics as `forward_key`.
    pub fn forward_click(&self, x: u16, y: u16, button: MouseButton) -> bool {
        if !self.has_on_click() {
            return false;
        }
        let _ = self.input_tx.send(PluginInput::Click {
            pane_id: self.pane_id,
            x,
            y,
            button,
        });
        true
    }
}

impl Pane for LuaPane {
    fn render(&self, area: Rect, ctx: &mut RenderCtx<'_, '_>) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let snapshot: Vec<String> = self
            .lines
            .lock()
            .map(|l| l.clone())
            .unwrap_or_default();
        let line_count = snapshot.len();
        // Clamp the scroll offset against the current line count and
        // viewport height. Lua-side `scroll_to` doesn't know how many
        // lines we have, and `set_lines` can shrink the buffer below
        // the previous offset.
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
                let _ = self.input_tx.send(PluginInput::Key {
                    pane_id: self.pane_id,
                    event: *k,
                });
                Outcome::Consumed
            }
            Event::Mouse(m) => {
                match m.kind {
                    MouseEventKind::Down(button) if self.has_on_click() => {
                        let _ = self.input_tx.send(PluginInput::Click {
                            pane_id: self.pane_id,
                            x: m.column,
                            y: m.row,
                            button,
                        });
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
                }
            }
            _ => Outcome::Ignored,
        }
    }

    fn is_focusable(&self) -> bool {
        true
    }
}

/// Snapshot of a plugin pane's shared state. Returned by
/// [`PluginRuntime::pane_for`] so the editor can build a [`LuaPane`]
/// without reaching into `Contributions` directly.
#[derive(Clone)]
pub struct PluginPane {
    pub pane_id: u64,
    pub lines: Arc<Mutex<Vec<String>>>,
    pub scroll: Arc<AtomicU16>,
    pub visible_rows: Arc<AtomicU16>,
    pub has_on_key: Arc<AtomicBool>,
    pub has_on_click: Arc<AtomicBool>,
    pub input_tx: UnboundedSender<PluginInput>,
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
            input_tx: self.input_tx,
        }
    }

    /// Snapshot of the current line content. Convenience for tests and
    /// the few sites that want a `Vec<String>` instead of the shared
    /// `Arc<Mutex<...>>`.
    pub fn lines_snapshot(&self) -> Vec<String> {
        self.lines.lock().map(|l| l.clone()).unwrap_or_default()
    }
}

/// Parse a chord string like `"ctrl+h"` / `"ctrl+shift+p"` into a
/// [`Chord`]. Returns `None` if the modifier or key fragment is
/// unrecognized — callers should treat that as "no chord, palette-only".
pub fn parse_chord(s: &str) -> Option<Chord> {
    let mut mods = KeyModifiers::NONE;
    let mut key: Option<KeyCode> = None;
    for part in s.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= KeyModifiers::CONTROL,
            "shift" => mods |= KeyModifiers::SHIFT,
            "alt" | "option" => mods |= KeyModifiers::ALT,
            "meta" | "cmd" | "super" => mods |= KeyModifiers::SUPER,
            other => {
                if key.is_some() {
                    return None;
                }
                key = parse_key(other);
            }
        }
    }
    key.map(|c| Chord::new(c, mods))
}

fn parse_key(s: &str) -> Option<KeyCode> {
    let lower = s.to_ascii_lowercase();
    let one_char = lower.chars().count() == 1;
    if one_char {
        return Some(KeyCode::Char(lower.chars().next().unwrap()));
    }
    Some(match lower.as_str() {
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "esc" | "escape" => KeyCode::Esc,
        "space" => KeyCode::Char(' '),
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "f1" => KeyCode::F(1),
        "f2" => KeyCode::F(2),
        "f3" => KeyCode::F(3),
        "f4" => KeyCode::F(4),
        "f5" => KeyCode::F(5),
        "f6" => KeyCode::F(6),
        "f7" => KeyCode::F(7),
        "f8" => KeyCode::F(8),
        "f9" => KeyCode::F(9),
        "f10" => KeyCode::F(10),
        "f11" => KeyCode::F(11),
        "f12" => KeyCode::F(12),
        _ => return None,
    })
}

/// Resolve the path the App should try to load on startup. Reads
/// `DEVIX_PLUGIN`; returns `None` if unset.
pub fn default_plugin_path() -> Option<PathBuf> {
    std::env::var_os("DEVIX_PLUGIN").map(PathBuf::from)
}

// -- Namespace path encoding (T-56) -----------------------------------------
//
// Plugin Lua callbacks are addressed at /plugin/<name>/cb/<u64> per
// `docs/specs/namespace.md` § *Migration table*. T-56 ships the
// canonical path encoding plus a decoder; the full
// `Lookup<Resource = LuaCallback>` impl is deferred until T-110 /
// T-111 (per the 2026-05-07 foundations-review amendment) when
// manifest-driven plugin loading lands and storage consolidation
// becomes load-bearing.

use devix_protocol::path::Path as DevixPath;

/// Encode a plugin callback handle as its canonical path
/// `/plugin/<name>/cb/<u64>`. Returns `None` if `plugin_name`
/// violates the segment grammar (e.g. uppercase, contains
/// reserved chars).
pub fn plugin_callback_path(plugin_name: &str, handle: u64) -> Option<DevixPath> {
    DevixPath::parse("/plugin")
        .ok()?
        .join(plugin_name)
        .ok()?
        .join("cb")
        .ok()?
        .join(&handle.to_string())
        .ok()
}

/// Decode a `/plugin/<name>/cb/<u64>` path into its plugin name +
/// handle. Returns `None` for any other shape.
pub fn handle_from_callback_path(path: &DevixPath) -> Option<(String, u64)> {
    let mut segs = path.segments();
    if segs.next()? != "plugin" {
        return None;
    }
    let name = segs.next()?.to_string();
    if segs.next()? != "cb" {
        return None;
    }
    let handle: u64 = segs.next()?.parse().ok()?;
    if segs.next().is_some() {
        return None;
    }
    Some((name, handle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp(name: &str, contents: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "devix-plugin-test-{}-{}",
            std::process::id(),
            name,
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("{name}.lua"));
        std::fs::write(&p, contents).unwrap();
        p
    }

    #[test]
    fn host_runs_and_collects_contributions() {
        let p = write_temp(
            "basic",
            r#"
                devix.register_action({
                    id = "hello",
                    label = "Hello from Lua",
                    chord = "ctrl+h",
                    run = function() devix.status("from-lua-status") end,
                })
                devix.register_pane({
                    slot = "left",
                    lines = { "from-lua" },
                })
            "#,
        );
        let host = PluginHost::new().unwrap();
        let c = host.load_file(&p).unwrap();
        assert_eq!(c.commands.len(), 1);
        assert_eq!(c.commands[0].id, "hello");
        assert_eq!(c.commands[0].chord, parse_chord("ctrl+h"));
        assert_eq!(c.panes.len(), 1);
        assert_eq!(c.panes[0].slot, SidebarSlot::Left);
        assert_eq!(
            *c.panes[0].lines.lock().unwrap(),
            vec!["from-lua".to_string()],
        );

        host.invoke(c.commands[0].handle);
        let msgs = host.drain_messages();
        assert!(matches!(&msgs[..], [PluginMsg::Status(s)] if s == "from-lua-status"));
    }

    #[test]
    fn dangerous_globals_are_stripped() {
        let host = PluginHost::new().unwrap();
        for name in ["io", "os", "package", "debug", "dofile", "loadfile", "load"] {
            let v: Value = host.lua.globals().get(name).unwrap();
            assert!(matches!(v, Value::Nil), "global `{name}` was not stripped");
        }
    }

    #[test]
    fn parse_chord_understands_common_modifiers() {
        let c = parse_chord("ctrl+h").unwrap();
        assert_eq!(c.code, KeyCode::Char('h'));
        assert!(c.mods.contains(KeyModifiers::CONTROL));

        let c = parse_chord("Ctrl+Shift+P").unwrap();
        assert_eq!(c.code, KeyCode::Char('p'));
        assert!(c.mods.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT));

        assert!(parse_chord("notakey").is_none());
        assert_eq!(parse_chord("F12").unwrap().code, KeyCode::F(12));
    }

    #[test]
    fn runtime_load_invoke_drain_roundtrip() {
        let p = write_temp(
            "rt",
            r#"
                devix.register_action({
                    id = "rt.hello",
                    label = "RT Hello",
                    run = function() devix.status("rt-fired") end,
                })
            "#,
        );
        let mut rt = PluginRuntime::load(&p).unwrap();
        assert_eq!(rt.contributions().commands.len(), 1);

        let handle = rt.contributions().commands[0].handle;
        rt.invoke_sender().send(handle).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            let msgs = rt.drain_messages();
            let status = msgs.iter().find_map(|m| match m {
                PluginMsg::Status(s) => Some(s.clone()),
                _ => None,
            });
            if let Some(s) = status {
                assert_eq!(s, "rt-fired");
                break;
            }
            if std::time::Instant::now() > deadline {
                panic!("plugin runtime did not deliver the status message in time");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn read_dir_lists_directory_entries() {
        let dir = std::env::temp_dir().join(format!(
            "devix-plugin-readdir-{}",
            std::process::id(),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();

        let lua_src = format!(
            r#"
                local items = devix.read_dir({path:?})
                local lines = {{}}
                for _, e in ipairs(items) do
                    table.insert(lines, e.name .. (e.is_dir and "/" or ""))
                end
                table.sort(lines)
                devix.register_pane({{ slot = "left", lines = lines }})
            "#,
            path = dir.to_string_lossy(),
        );
        let p = write_temp("readdir", &lua_src);
        let host = PluginHost::new().unwrap();
        let c = host.load_file(&p).unwrap();
        assert_eq!(c.panes.len(), 1);
        assert_eq!(
            *c.panes[0].lines.lock().unwrap(),
            vec!["a.txt".to_string(), "sub/".to_string()],
        );
    }

    #[test]
    fn cwd_returns_current_directory() {
        let p = write_temp(
            "cwd",
            r#"
                devix.register_pane({ slot = "left", lines = { devix.cwd() } })
            "#,
        );
        let host = PluginHost::new().unwrap();
        let c = host.load_file(&p).unwrap();
        let expected = std::env::current_dir().unwrap().to_string_lossy().into_owned();
        assert_eq!(*c.panes[0].lines.lock().unwrap(), vec![expected]);
    }

    #[test]
    fn unknown_slot_in_register_pane_errors_during_load() {
        let p = write_temp(
            "badslot",
            r#"devix.register_pane({ slot = "middle", lines = { "x" } })"#,
        );
        let host = PluginHost::new().unwrap();
        let err = host.load_file(&p).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("middle"), "error should mention the bad slot, got: {msg}");
    }

    #[test]
    fn pane_set_lines_replaces_shared_storage_and_signals_dirty() {
        let p = write_temp(
            "set_lines",
            r#"
                local pane = devix.register_pane({ slot = "left", lines = { "old" } })
                devix.register_action({
                    id = "refresh",
                    label = "Refresh",
                    run = function()
                        pane:set_lines({ "alpha", "beta" })
                    end,
                })
            "#,
        );
        let host = PluginHost::new().unwrap();
        let c = host.load_file(&p).unwrap();
        let lines = c.panes[0].lines.clone();
        assert_eq!(*lines.lock().unwrap(), vec!["old".to_string()]);

        host.invoke(c.commands[0].handle);
        assert_eq!(
            *lines.lock().unwrap(),
            vec!["alpha".to_string(), "beta".to_string()],
        );

        let msgs = host.drain_messages();
        assert!(
            msgs.iter().any(|m| matches!(m, PluginMsg::PaneChanged)),
            "set_lines should emit PaneChanged, got {msgs:?}",
        );
    }

    #[test]
    fn pane_on_key_callback_runs_when_dispatched() {
        let p = write_temp(
            "on_key",
            r#"
                local pane = devix.register_pane({ slot = "left" })
                pane:on_key(function(ev)
                    devix.status("got-key:" .. ev.key)
                end)
            "#,
        );
        let host = PluginHost::new().unwrap();
        let c = host.load_file(&p).unwrap();
        let pane_id = c.panes[0].pane_id;
        assert!(c.panes[0].has_on_key.load(Ordering::Acquire));

        host.dispatch_input(PluginInput::Key {
            pane_id,
            event: KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        });
        let msgs = host.drain_messages();
        assert!(
            msgs.iter().any(|m| matches!(m, PluginMsg::Status(s) if s == "got-key:enter")),
            "expected on_key to fire and push status, got {msgs:?}",
        );
    }

    #[test]
    fn pane_on_click_callback_receives_coordinates() {
        let p = write_temp(
            "on_click",
            r#"
                local pane = devix.register_pane({ slot = "left" })
                pane:on_click(function(ev)
                    devix.status(string.format("click:%d,%d:%s", ev.x, ev.y, ev.button))
                end)
            "#,
        );
        let host = PluginHost::new().unwrap();
        let c = host.load_file(&p).unwrap();
        let pane_id = c.panes[0].pane_id;
        assert!(c.panes[0].has_on_click.load(Ordering::Acquire));

        host.dispatch_input(PluginInput::Click {
            pane_id,
            x: 3,
            y: 7,
            button: MouseButton::Left,
        });
        let msgs = host.drain_messages();
        assert!(
            msgs.iter().any(|m| matches!(m, PluginMsg::Status(s) if s == "click:3,7:left")),
            "expected on_click to fire with coords, got {msgs:?}",
        );
    }

    #[test]
    fn open_path_emits_open_message() {
        let p = write_temp(
            "open",
            r#"
                devix.register_action({
                    id = "open",
                    label = "Open",
                    run = function() devix.open_path("/tmp/devix-test-target") end,
                })
            "#,
        );
        let host = PluginHost::new().unwrap();
        let c = host.load_file(&p).unwrap();
        host.invoke(c.commands[0].handle);
        let msgs = host.drain_messages();
        assert!(
            msgs.iter().any(|m| matches!(m, PluginMsg::OpenPath(p) if p == &PathBuf::from("/tmp/devix-test-target"))),
            "expected OpenPath message, got {msgs:?}",
        );
    }
}

#[cfg(test)]
mod namespace_tests {
    use super::*;

    #[test]
    fn callback_path_round_trips() {
        let p = plugin_callback_path("file-tree", 42).unwrap();
        assert_eq!(p.as_str(), "/plugin/file-tree/cb/42");
        let (name, handle) = handle_from_callback_path(&p).unwrap();
        assert_eq!(name, "file-tree");
        assert_eq!(handle, 42);
    }

    #[test]
    fn callback_path_rejects_invalid_name() {
        // Path grammar (namespace.md) accepts ASCII alphanumeric +
        // `-`, `_`, `.`. Lowercase-only is a manifest.md plugin-name
        // contract, not a path-grammar one — "FileTree" is a legal
        // path segment even though the manifest validator would
        // reject it as a plugin name.
        assert!(plugin_callback_path("FileTree", 1).is_some());
        // Reserved chars (whitespace, `:`) violate the path grammar.
        assert!(plugin_callback_path("file tree", 1).is_none());
        assert!(plugin_callback_path("file:tree", 1).is_none());
        // Empty name fails.
        assert!(plugin_callback_path("", 1).is_none());
    }

    #[test]
    fn handle_from_callback_path_rejects_other_shapes() {
        let p = devix_protocol::path::Path::parse("/buf/3").unwrap();
        assert!(handle_from_callback_path(&p).is_none());
        let p = devix_protocol::path::Path::parse("/plugin/x/foo/4").unwrap();
        assert!(handle_from_callback_path(&p).is_none());
        let p = devix_protocol::path::Path::parse("/plugin/x/cb/abc").unwrap();
        assert!(handle_from_callback_path(&p).is_none());
        let p = devix_protocol::path::Path::parse("/plugin/x/cb/4/extra").unwrap();
        assert!(handle_from_callback_path(&p).is_none());
    }
}
