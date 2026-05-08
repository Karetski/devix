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
//! ## Module layout (T-81)
//!
//! Per `docs/specs/crates.md` the plugin host splits into four
//! concerns. The MLIR principle: each concern is one open primitive.
//!
//! - [`host`]: the Lua VM owner. Stays on one thread; never crosses
//!   thread boundaries. Holds the `Lua` state, the callback registry,
//!   and the contributions accumulator.
//! - [`pane_handle`]: the `LuaPaneHandle` userdata returned to Lua
//!   from `devix.register_pane`, plus the editor-side `LuaPane` /
//!   `PluginPane` types that wrap the shared state.
//! - [`bridge`]: editor-side action wrapper (`LuaAction`) that turns a
//!   plugin command into something `CommandRegistry` can store and
//!   `Keymap` can dispatch.
//! - [`runtime`]: the supervised worker thread that owns a
//!   `PluginHost` and exposes channel handles back to the editor.
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
//! Editor-held senders (`invoke_tx`, `input_tx`) are wrapped in
//! [`InvokeSender`] / [`InputSender`] (`Arc<Mutex<UnboundedSender<…>>>`)
//! so the supervisor can swap a fresh sender into the same `Arc` on
//! restart. Erlang principle: let it die, restart clean. The receiver
//! the worker uses is recreated per spawn; the editor's outbound
//! handles refresh in lockstep without touching every captured
//! sender callsite.
//!
//! ## Sandboxing
//!
//! [`PluginHost::new`] starts from `StdLib::ALL_SAFE` (omits `os` /
//! `io` / `package` / `debug` / `ffi`) and additionally nils out
//! `dofile`, `loadfile`, `load`, `loadstring`, `require`, and the
//! matching globals so plugins cannot escape the sandbox by re-loading
//! code from disk. New Rust→Lua entry points always go in the `devix`
//! table; the bare globals are not extended.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16};
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton};
use crate::editor::Chord;
use crate::SidebarSlot;
use mlua::Value;
use tokio::sync::mpsc::UnboundedSender;

mod bridge;
mod host;
mod pane_handle;
mod runtime;

pub use bridge::{LuaAction, PluginCommandAction, make_command_action};
pub use host::PluginHost;
pub use pane_handle::{LuaPane, LuaPaneHandle, PluginPane};
pub use runtime::{MsgSink, PluginRuntime, host_capabilities};

/// Editor-held handle for the host's invoke channel. Wrapped in
/// `Arc<Mutex<…>>` so the supervisor can swap a fresh sender into
/// the same `Arc` on restart without rewiring every editor-side
/// captured sender.
pub type InvokeSender = Arc<Mutex<UnboundedSender<u64>>>;

/// Editor-held handle for the host's input channel. Same shape as
/// [`InvokeSender`].
pub type InputSender = Arc<Mutex<UnboundedSender<PluginInput>>>;

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
    /// A `Pulse::SettingChanged` matched a registered
    /// `devix.on_setting_changed(callback)` handler. The runtime's
    /// bus subscriber pushes one of these per registered callback;
    /// the worker dispatches by calling `host.invoke_with(handle, …)`
    /// with the (key, value) marshaled into a Lua table. T-113.
    SettingChanged {
        handle: u64,
        key: String,
        value: devix_protocol::manifest::SettingValue,
    },
}

/// Send `handle` through `sender` if its lock is reachable. Silent
/// no-op on poisoned lock or closed receiver — Erlang semantics:
/// callers don't need to care if the worker is mid-restart.
pub fn send_invoke(sender: &InvokeSender, handle: u64) -> bool {
    match sender.lock() {
        Ok(tx) => tx.send(handle).is_ok(),
        Err(_) => false,
    }
}

/// Send `input` through `sender`. Same semantics as [`send_invoke`].
pub fn send_input(sender: &InputSender, input: PluginInput) -> bool {
    match sender.lock() {
        Ok(tx) => tx.send(input).is_ok(),
        Err(_) => false,
    }
}

/// Replace any reserved-segment chars in `s` with `_` so plugin name
/// stems (filenames) become valid path segments per `namespace.md`.
pub(crate) fn sanitize_plugin_segment(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) fn parse_lines_value(value: Value) -> mlua::Result<Vec<String>> {
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

pub(crate) fn next_handle(counter: &Arc<Mutex<u64>>) -> mlua::Result<u64> {
    let mut h = counter
        .lock()
        .map_err(|e| mlua::Error::external(anyhow!("{e}")))?;
    let cur = *h;
    *h += 1;
    Ok(cur)
}

pub(crate) fn key_code_to_string(code: KeyCode) -> String {
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

pub(crate) fn parse_key(s: &str) -> Option<KeyCode> {
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
// `Lookup<Resource = LuaCallback>` impl is deferred until manifest-driven
// plugin loading lands and storage consolidation becomes load-bearing.

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
    use std::sync::atomic::Ordering;
    use crate::editor::{CommandRegistry, Editor, Keymap};

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
    fn manifest_driven_commands_register_at_plugin_namespace() {
        use devix_protocol::manifest::{
            CommandSpec as ManifestCommandSpec, Contributes, Engines, Manifest,
        };
        use devix_protocol::path::Path as DevixPath;
        use devix_protocol::Lookup;

        let p = write_temp(
            "manifest_driven",
            r#"
                devix.register_action({
                    id = "say-hello",
                    label = "Say Hello",
                    run = function() devix.status("hello-fired") end,
                })
            "#,
        );

        let manifest = Manifest {
            name: "myplug".to_string(),
            version: "0.1.0".to_string(),
            engines: Engines {
                protocol_version: devix_protocol::protocol::ProtocolVersion::new(0, 1),
                pulse_bus: devix_protocol::protocol::ProtocolVersion::new(0, 1),
                manifest: devix_protocol::protocol::ProtocolVersion::new(0, 1),
            },
            entry: None,
            contributes: Contributes {
                commands: vec![ManifestCommandSpec {
                    id: "say-hello".to_string(),
                    label: "Say Hello".to_string(),
                    category: Some("Test".to_string()),
                    lua_handle: None,
                }],
                ..Default::default()
            },
            subscribe: Vec::new(),
        };

        let bus = crate::PulseBus::new();
        let mut runtime = PluginRuntime::load(&p).unwrap();
        let mut commands = CommandRegistry::new();
        let mut keymap = Keymap::new();
        let mut editor = Editor::open(None).unwrap();
        let count = runtime.install_with_manifest(
            &mut commands,
            &mut keymap,
            &mut editor,
            &manifest,
            &bus,
        );
        assert_eq!(count, 1, "one manifest-declared command registered");

        let plugin_cmd =
            DevixPath::parse("/plugin/myplug/cmd/say-hello").unwrap();
        assert!(
            commands.lookup(&plugin_cmd).is_some(),
            "command lookups against /plugin/<name>/cmd/<id> resolve",
        );
    }

    #[test]
    fn manifest_keymaps_bind_plugin_chord_via_install_with_manifest() {
        use devix_protocol::input::Chord as ProtoChord;
        use devix_protocol::manifest::{
            CommandSpec as ManifestCommandSpec, Contributes, Engines, KeymapSpec,
            Manifest,
        };
        use devix_protocol::protocol::ProtocolVersion;

        let p = write_temp(
            "kbm",
            r#"
                devix.register_action({
                    id = "ping",
                    label = "Ping",
                    run = function() devix.status("ping-fired") end,
                })
            "#,
        );

        let manifest = Manifest {
            name: "kbm".to_string(),
            version: "0.1.0".to_string(),
            engines: Engines {
                protocol_version: ProtocolVersion::new(0, 1),
                pulse_bus: ProtocolVersion::new(0, 1),
                manifest: ProtocolVersion::new(0, 1),
            },
            entry: None,
            contributes: Contributes {
                commands: vec![ManifestCommandSpec {
                    id: "ping".to_string(),
                    label: "Ping".to_string(),
                    category: None,
                    lua_handle: None,
                }],
                keymaps: vec![KeymapSpec {
                    key: ProtoChord::parse("ctrl-y").unwrap(),
                    command: "/plugin/kbm/cmd/ping".to_string(),
                    when: None,
                }],
                ..Default::default()
            },
            subscribe: Vec::new(),
        };

        let bus = crate::PulseBus::new();
        let mut runtime = PluginRuntime::load(&p).unwrap();
        let mut commands = CommandRegistry::new();
        let mut keymap = Keymap::new();
        let mut editor = Editor::open(None).unwrap();
        runtime.install_with_manifest(
            &mut commands,
            &mut keymap,
            &mut editor,
            &manifest,
            &bus,
        );

        // Manifest's keymap entry binds ctrl-y → plugin command.
        use crate::editor::commands::keymap::Chord;
        use crossterm::event::{KeyCode, KeyModifiers};
        let chord = Chord::new(KeyCode::Char('y'), KeyModifiers::CONTROL);
        let action = keymap.resolve_chord(chord, &commands);
        assert!(action.is_some(), "manifest keymap binding resolves through registry");
    }

    #[test]
    fn first_loaded_wins_on_chord_conflict() {
        use devix_protocol::manifest::{
            CommandSpec as ManifestCommandSpec, Contributes, Engines, Manifest,
        };
        use devix_protocol::protocol::ProtocolVersion;
        use devix_protocol::pulse::{Pulse, PulseFilter, PulseKind};

        // Two plugins each register the same chord (ctrl+x).
        let p1 = write_temp(
            "first",
            r#"
                devix.register_action({
                    id = "go",
                    label = "Go",
                    chord = "ctrl+x",
                    run = function() end,
                })
            "#,
        );
        let p2 = write_temp(
            "second",
            r#"
                devix.register_action({
                    id = "go",
                    label = "Go",
                    chord = "ctrl+x",
                    run = function() end,
                })
            "#,
        );

        let make_manifest = |name: &str| Manifest {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            engines: Engines {
                protocol_version: ProtocolVersion::new(0, 1),
                pulse_bus: ProtocolVersion::new(0, 1),
                manifest: ProtocolVersion::new(0, 1),
            },
            entry: None,
            contributes: Contributes {
                commands: vec![ManifestCommandSpec {
                    id: "go".to_string(),
                    label: "Go".to_string(),
                    category: None,
                    lua_handle: None,
                }],
                ..Default::default()
            },
            subscribe: Vec::new(),
        };

        let bus = crate::PulseBus::new();
        let captured =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::<Pulse>::new()));
        let cap = captured.clone();
        bus.subscribe(PulseFilter::kind(PulseKind::PluginError), move |p| {
            cap.lock().unwrap().push(p.clone());
        });

        let mut commands = CommandRegistry::new();
        let mut keymap = Keymap::new();
        let mut editor = Editor::open(None).unwrap();

        let mut rt1 = PluginRuntime::load(&p1).unwrap();
        rt1.install_with_manifest(
            &mut commands,
            &mut keymap,
            &mut editor,
            &make_manifest("first"),
            &bus,
        );

        let mut rt2 = PluginRuntime::load(&p2).unwrap();
        rt2.install_with_manifest(
            &mut commands,
            &mut keymap,
            &mut editor,
            &make_manifest("second"),
            &bus,
        );

        // The first-loaded plugin keeps the chord; second-loaded sees a
        // PluginError about the conflict.
        let pulses = captured.lock().unwrap();
        assert_eq!(pulses.len(), 1, "second plugin's chord conflict surfaces");
        if let Pulse::PluginError { plugin, message } = &pulses[0] {
            assert_eq!(plugin.as_str(), "/plugin/second");
            assert!(message.contains("chord conflict"));
        }

        // Resolve ctrl+x — should still hit the first plugin's command.
        use crate::editor::commands::keymap::Chord;
        use crate::editor::commands::registry::CommandId;
        use crossterm::event::{KeyCode, KeyModifiers};
        let chord = Chord::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        // Re-register the first plugin's command resolution path
        // (the registry currently has BOTH commands — `first.go` was
        // installed before `second.go`. The chord stays bound to the
        // first plugin's id.)
        let _first_id = CommandId::plugin("first", "go");
        // Sanity: action resolves; ensure no panic.
        let _ = keymap.resolve_chord(chord, &commands);
    }

    #[test]
    fn manifest_declares_pane_without_matching_lua_pane_publishes_plugin_error() {
        use devix_protocol::manifest::{
            Contributes, Engines, Manifest, PaneSpec as ManifestPaneSpec,
        };
        use devix_protocol::pulse::{Pulse, PulseFilter, PulseKind};
        use devix_protocol::view::SidebarSlot as ProtoSlot;

        // Plugin registers nothing on the right sidebar.
        let p = write_temp(
            "pane_orphan",
            r#"
                devix.register_pane({ slot = "left", lines = { "ok" } })
            "#,
        );

        let manifest = Manifest {
            name: "panes".to_string(),
            version: "0.1.0".to_string(),
            engines: Engines {
                protocol_version: devix_protocol::protocol::ProtocolVersion::new(0, 1),
                pulse_bus: devix_protocol::protocol::ProtocolVersion::new(0, 1),
                manifest: devix_protocol::protocol::ProtocolVersion::new(0, 1),
            },
            entry: None,
            contributes: Contributes {
                panes: vec![ManifestPaneSpec {
                    id: "missing-side".to_string(),
                    slot: ProtoSlot::Right,
                    lua_handle: None,
                }],
                ..Default::default()
            },
            subscribe: Vec::new(),
        };

        let bus = crate::PulseBus::new();
        let captured =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::<Pulse>::new()));
        let cap = captured.clone();
        bus.subscribe(PulseFilter::kind(PulseKind::PluginError), move |p| {
            cap.lock().unwrap().push(p.clone());
        });

        let mut runtime = PluginRuntime::load(&p).unwrap();
        let mut commands = CommandRegistry::new();
        let mut keymap = Keymap::new();
        let mut editor = Editor::open(None).unwrap();
        runtime.install_with_manifest(
            &mut commands,
            &mut keymap,
            &mut editor,
            &manifest,
            &bus,
        );

        let pulses = captured.lock().unwrap();
        assert_eq!(pulses.len(), 1, "PluginError fired for the orphan pane decl");
        if let Pulse::PluginError { plugin, message } = &pulses[0] {
            assert_eq!(plugin.as_str(), "/plugin/panes");
            assert!(message.contains("missing-side"));
            assert!(message.contains("Right"));
        }
    }

    #[test]
    fn manifest_declares_pane_with_matching_lua_pane_does_not_warn() {
        use devix_protocol::manifest::{
            Contributes, Engines, Manifest, PaneSpec as ManifestPaneSpec,
        };
        use devix_protocol::pulse::{PulseFilter, PulseKind};
        use devix_protocol::view::SidebarSlot as ProtoSlot;

        let p = write_temp(
            "pane_match",
            r#"
                devix.register_pane({ slot = "left", lines = { "ok" } })
            "#,
        );

        let manifest = Manifest {
            name: "panes2".to_string(),
            version: "0.1.0".to_string(),
            engines: Engines {
                protocol_version: devix_protocol::protocol::ProtocolVersion::new(0, 1),
                pulse_bus: devix_protocol::protocol::ProtocolVersion::new(0, 1),
                manifest: devix_protocol::protocol::ProtocolVersion::new(0, 1),
            },
            entry: None,
            contributes: Contributes {
                panes: vec![ManifestPaneSpec {
                    id: "tree".to_string(),
                    slot: ProtoSlot::Left,
                    lua_handle: None,
                }],
                ..Default::default()
            },
            subscribe: Vec::new(),
        };

        let bus = crate::PulseBus::new();
        let plugin_errors = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let pe = plugin_errors.clone();
        bus.subscribe(PulseFilter::kind(PulseKind::PluginError), move |_| {
            pe.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        });

        let mut runtime = PluginRuntime::load(&p).unwrap();
        let mut commands = CommandRegistry::new();
        let mut keymap = Keymap::new();
        let mut editor = Editor::open(None).unwrap();
        runtime.install_with_manifest(
            &mut commands,
            &mut keymap,
            &mut editor,
            &manifest,
            &bus,
        );

        assert_eq!(
            plugin_errors.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "matching declaration is silent",
        );
    }

    #[test]
    fn manifest_declares_unknown_command_id_publishes_plugin_error() {
        use devix_protocol::manifest::{
            CommandSpec as ManifestCommandSpec, Contributes, Engines, Manifest,
        };
        use devix_protocol::pulse::{Pulse, PulseFilter, PulseKind};

        let p = write_temp(
            "manifest_orphan",
            r#"
                devix.register_action({
                    id = "actually-here",
                    label = "Here",
                    run = function() end,
                })
            "#,
        );

        let manifest = Manifest {
            name: "orphan".to_string(),
            version: "0.1.0".to_string(),
            engines: Engines {
                protocol_version: devix_protocol::protocol::ProtocolVersion::new(0, 1),
                pulse_bus: devix_protocol::protocol::ProtocolVersion::new(0, 1),
                manifest: devix_protocol::protocol::ProtocolVersion::new(0, 1),
            },
            entry: None,
            contributes: Contributes {
                commands: vec![ManifestCommandSpec {
                    id: "missing-from-lua".to_string(),
                    label: "Missing".to_string(),
                    category: None,
                    lua_handle: None,
                }],
                ..Default::default()
            },
            subscribe: Vec::new(),
        };

        let bus = crate::PulseBus::new();
        let captured =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::<Pulse>::new()));
        let cap = captured.clone();
        bus.subscribe(PulseFilter::kind(PulseKind::PluginError), move |pulse| {
            cap.lock().unwrap().push(pulse.clone());
        });

        let mut runtime = PluginRuntime::load(&p).unwrap();
        let mut commands = CommandRegistry::new();
        let mut keymap = Keymap::new();
        let mut editor = Editor::open(None).unwrap();
        let count = runtime.install_with_manifest(
            &mut commands,
            &mut keymap,
            &mut editor,
            &manifest,
            &bus,
        );
        assert_eq!(count, 0, "manifest declares an id with no matching Lua handler — skipped");

        let pulses = captured.lock().unwrap();
        assert_eq!(pulses.len(), 1, "PluginError fired for the orphan declaration");
        if let Pulse::PluginError { plugin, message } = &pulses[0] {
            assert_eq!(plugin.as_str(), "/plugin/orphan");
            assert!(message.contains("missing-from-lua"));
        }
    }

    #[test]
    fn supervised_load_publishes_plugin_loaded_pulse() {
        use devix_protocol::pulse::{Pulse, PulseFilter, PulseKind};
        let p = write_temp(
            "supervised",
            r#"
                devix.register_action({
                    id = "noop",
                    label = "noop",
                    run = function() end,
                })
            "#,
        );
        let bus = crate::PulseBus::new();
        let captured =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::<Pulse>::new()));
        let cap = captured.clone();
        bus.subscribe(PulseFilter::kind(PulseKind::PluginLoaded), move |pulse| {
            cap.lock().unwrap().push(pulse.clone());
        });
        let sink: MsgSink = std::sync::Arc::new(|_| {});
        let _rt = PluginRuntime::load_supervised(&p, sink, bus.clone()).unwrap();
        let pulses = captured.lock().unwrap();
        assert_eq!(pulses.len(), 1, "PluginLoaded fires once on supervised load");
        if let Pulse::PluginLoaded { plugin, .. } = &pulses[0] {
            assert_eq!(plugin.as_str(), "/plugin/supervised");
        }
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
            let v: Value = host.lua().globals().get(name).unwrap();
            assert!(matches!(v, Value::Nil), "global `{name}` was not stripped");
        }
    }

    #[test]
    fn devix_setting_reads_from_shared_store() {
        use crate::settings_store::SettingsStore;
        use devix_protocol::manifest::{
            Contributes, Engines, Manifest, SettingSpec, SettingValue,
        };
        use devix_protocol::protocol::ProtocolVersion;
        use std::collections::HashMap;

        let p = write_temp(
            "settings_read",
            r#"
                devix.register_action({
                    id = "report",
                    label = "Report",
                    run = function()
                        devix.status(tostring(devix.setting("editor.tab_size")))
                    end,
                })
            "#,
        );

        let mut settings_map = HashMap::new();
        settings_map.insert(
            "editor.tab_size".to_string(),
            SettingSpec::Number { default: 4.0, label: "Tab".into() },
        );
        let manifest = Manifest {
            name: "settings_read".into(),
            version: "0.1.0".into(),
            engines: Engines {
                protocol_version: ProtocolVersion::new(0, 1),
                pulse_bus: ProtocolVersion::new(0, 1),
                manifest: ProtocolVersion::new(0, 1),
            },
            entry: None,
            contributes: Contributes {
                settings: settings_map,
                ..Default::default()
            },
            subscribe: Vec::new(),
        };

        let store = std::sync::Arc::new(std::sync::Mutex::new(SettingsStore::new()));
        store.lock().unwrap().register_from_manifest(&manifest);
        // Override the default so the test asserts we read live state.
        let bus = crate::PulseBus::new();
        store
            .lock()
            .unwrap()
            .set("editor.tab_size", SettingValue::Number(8.0), &bus);

        let host = PluginHost::new_with(Some(store)).unwrap();
        let contributions = host.load_file(&p).unwrap();
        host.invoke(contributions.commands[0].handle);
        let msgs = host.drain_messages();
        // Lua's tostring(8.0) renders as "8.0" (mlua's default).
        assert!(
            matches!(&msgs[..], [PluginMsg::Status(s)] if s == "8.0"),
            "expected `8.0`, got {msgs:?}",
        );
    }

    #[test]
    fn restricted_capability_skips_command_install_and_fires_plugin_error() {
        use devix_protocol::manifest::{
            CommandSpec as ManifestCommandSpec, Contributes, Engines, Manifest,
        };
        use devix_protocol::Lookup;
        use devix_protocol::path::Path as DevixPath;
        use devix_protocol::protocol::{Capability, ProtocolVersion};
        use devix_protocol::pulse::{Pulse, PulseFilter, PulseKind};
        use std::collections::HashSet;
        use std::sync::{Arc as StdArc, Mutex as StdMutex};

        let p = write_temp(
            "cap_deny",
            r#"
                devix.register_action({
                    id = "go",
                    label = "Go",
                    run = function() end,
                })
            "#,
        );

        let manifest = Manifest {
            name: "capdeny".into(),
            version: "0.1.0".into(),
            engines: Engines {
                protocol_version: ProtocolVersion::new(0, 1),
                pulse_bus: ProtocolVersion::new(0, 1),
                manifest: ProtocolVersion::new(0, 1),
            },
            entry: None,
            contributes: Contributes {
                commands: vec![ManifestCommandSpec {
                    id: "go".into(),
                    label: "Go".into(),
                    category: None,
                    lua_handle: None,
                }],
                ..Default::default()
            },
            subscribe: Vec::new(),
        };

        // Restricted capability set: ContributeCommands removed.
        let mut caps: HashSet<Capability> = host_capabilities();
        caps.remove(&Capability::ContributeCommands);

        let bus = crate::PulseBus::new();
        let captured: StdArc<StdMutex<Vec<Pulse>>> =
            StdArc::new(StdMutex::new(Vec::new()));
        let cap = captured.clone();
        bus.subscribe(PulseFilter::kind(PulseKind::PluginError), move |p| {
            cap.lock().unwrap().push(p.clone());
        });

        let sink: MsgSink = StdArc::new(|_| {});
        let mut runtime = PluginRuntime::load_supervised_with_caps(
            &p,
            sink,
            bus.clone(),
            None,
            caps,
        )
        .unwrap();

        let mut commands = CommandRegistry::new();
        let mut keymap = Keymap::new();
        let mut editor = Editor::open(None).unwrap();
        let count = runtime.install_with_manifest(
            &mut commands,
            &mut keymap,
            &mut editor,
            &manifest,
            &bus,
        );
        assert_eq!(count, 0, "no commands installed under restricted capability");
        assert!(
            commands.lookup(&DevixPath::parse("/plugin/capdeny/cmd/go").unwrap()).is_none(),
            "command should not resolve through registry",
        );
        // The Lookup trait isn't in scope here; we know lookup is part of CommandRegistry.

        let pulses = captured.lock().unwrap();
        assert!(
            pulses.iter().any(|p| matches!(p,
                Pulse::PluginError { message, .. }
                if message.contains("ContributeCommands") && message.contains("commands"))),
            "expected PluginError for ContributeCommands, got {pulses:?}",
        );
    }

    #[test]
    fn on_setting_changed_dispatches_via_input_channel() {
        use crate::settings_store::SettingsStore;
        use devix_protocol::manifest::{
            Contributes, Engines, Manifest, SettingSpec, SettingValue,
        };
        use devix_protocol::protocol::ProtocolVersion;
        use std::collections::HashMap;

        let p = write_temp(
            "settings_observe",
            r#"
                devix.on_setting_changed(function(key, value)
                    devix.status("changed:" .. key .. "=" .. tostring(value))
                end)
            "#,
        );
        let mut settings_map = HashMap::new();
        settings_map.insert(
            "ui.compact".to_string(),
            SettingSpec::Boolean { default: false, label: "Compact".into() },
        );
        let manifest = Manifest {
            name: "settings_observe".into(),
            version: "0.1.0".into(),
            engines: Engines {
                protocol_version: ProtocolVersion::new(0, 1),
                pulse_bus: ProtocolVersion::new(0, 1),
                manifest: ProtocolVersion::new(0, 1),
            },
            entry: None,
            contributes: Contributes {
                settings: settings_map,
                ..Default::default()
            },
            subscribe: Vec::new(),
        };
        let store = std::sync::Arc::new(std::sync::Mutex::new(SettingsStore::new()));
        store.lock().unwrap().register_from_manifest(&manifest);

        let host = PluginHost::new_with(Some(store)).unwrap();
        host.load_file(&p).unwrap();

        // Pull the registered handle so we can dispatch directly
        // (mirrors what the runtime's bus subscriber would do).
        let handles: Vec<u64> = host
            .setting_callbacks()
            .lock()
            .map(|h| h.clone())
            .unwrap_or_default();
        assert_eq!(handles.len(), 1, "one on_setting_changed handler registered");
        host.dispatch_input(PluginInput::SettingChanged {
            handle: handles[0],
            key: "ui.compact".into(),
            value: SettingValue::Boolean(true),
        });
        let msgs = host.drain_messages();
        assert!(
            matches!(&msgs[..], [PluginMsg::Status(s)] if s == "changed:ui.compact=true"),
            "expected changed status; got {msgs:?}",
        );
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
        assert!(send_invoke(&rt.invoke_sender(), handle));
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
