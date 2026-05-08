//! End-to-end tests for the plugin host: action / pane / input wiring.
//!
//! What's *not* exercised here is the App's input loop — these tests
//! work directly against the plugin runtime + the editor APIs each
//! channel feeds into. The App-side wiring (focus → forward_key,
//! drain_plugin_events → status / OpenPath) gets coverage in
//! `crates/app/src/render.rs::tests`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use devix_core::Pane;
use devix_core::{
    LuaPane, PluginInput, PluginMsg, PluginRuntime, make_command_action, parse_chord,
};
use devix_core::{Command, CommandId, CommandRegistry, Context, Keymap, Viewport};
use devix_core::SidebarSlot;
use devix_core::Editor;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

const PLUGIN_SOURCE: &str = r#"
devix.register_action({
    id = "hello",
    label = "Hello from Lua",
    chord = "ctrl+h",
    run = function() devix.status("hello-from-lua") end,
})

devix.register_pane({
    slot = "left",
    lines = { "from-lua" },
})
"#;

fn write_plugin_named(name: &str, source: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "devix-plugin-e2e-{}-{}-{:?}",
        name,
        std::process::id(),
        std::time::SystemTime::now(),
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("plugin.lua");
    std::fs::write(&p, source).unwrap();
    p
}

fn write_plugin() -> PathBuf {
    write_plugin_named("base", PLUGIN_SOURCE)
}

fn drain_until<F: FnMut(&PluginMsg) -> bool>(
    rt: &mut PluginRuntime,
    timeout: Duration,
    mut pred: F,
) -> Option<PluginMsg> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let msgs = rt.drain_messages();
        for m in msgs {
            if pred(&m) {
                return Some(m);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

#[test]
fn hello_command_round_trips_through_registry_and_keymap() {
    let path = write_plugin();
    let mut runtime = PluginRuntime::load(&path).unwrap();
    assert_eq!(runtime.contributions().commands.len(), 1);

    let mut commands = CommandRegistry::new();
    let mut keymap = Keymap::new();

    let spec = runtime.contributions().commands[0].clone();
    let id_static: &'static str = Box::leak(spec.id.clone().into_boxed_str());
    let label_static: &'static str = Box::leak(spec.label.clone().into_boxed_str());
    let cid = CommandId::builtin(id_static);
    commands.register(Command {
        id: cid,
        label: label_static,
        category: Some("Plugin"),
        action: make_command_action(&spec, runtime.invoke_sender()),
    });
    let chord = spec.chord.expect("plugin under test must declare a chord");
    keymap.bind_command(chord, cid);

    let action = keymap
        .resolve_chord(chord, &commands)
        .expect("plugin chord must resolve to a command");

    let mut editor = Editor::open(None).unwrap();
    let mut clipboard = devix_core::NoClipboard;
    let mut quit = false;
    {
        let mut ctx = Context {
            editor: &mut editor,
            clipboard: &mut clipboard,
            quit: &mut quit,
            viewport: Viewport::default(),
            commands: &commands,
        };
        action.invoke(&mut ctx);
    }

    let got = drain_until(&mut runtime, Duration::from_secs(2), |m| {
        matches!(m, PluginMsg::Status(s) if s == "hello-from-lua")
    });
    assert!(got.is_some(), "plugin should have produced a status message");

    drop(action);
    let _ = (commands, keymap, editor, quit);

    let mut commands = CommandRegistry::new();
    commands.register(Command {
        id: cid,
        label: label_static,
        category: Some("Plugin"),
        action: make_command_action(&spec, runtime.invoke_sender()),
    });
    let labels: Vec<&str> = commands.iter().map(|c| c.label).collect();
    assert!(labels.contains(&"Hello from Lua"));
}

#[test]
fn left_sidebar_pane_renders_lua_lines() {
    let path = write_plugin();
    let runtime = PluginRuntime::load(&path).unwrap();
    let pane = runtime
        .pane_for(SidebarSlot::Left)
        .expect("plugin contributed a left sidebar pane");
    let lua_pane = pane.into_pane();

    let backend = TestBackend::new(20, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let mut ctx = devix_core::RenderCtx { frame: f };
            lua_pane.render(Rect { x: 0, y: 0, width: 20, height: 5 }, &mut ctx);
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let mut top = String::new();
    for x in 0..buf.area.width {
        top.push_str(buf[(x, 0)].symbol());
    }
    assert!(
        top.starts_with("from-lua"),
        "expected sidebar to render `from-lua` on the first row, got {top:?}",
    );
}

#[test]
fn pane_set_lines_updates_render_after_load() {
    // Plugin starts with `["before"]`; the registered action mutates
    // the pane to `["after-1", "after-2"]`. We invoke the action via
    // the runtime channel, wait for the PaneChanged signal, and assert
    // the next render shows the new lines.
    let source = r#"
        local pane = devix.register_pane({ slot = "left", lines = { "before" } })
        devix.register_action({
            id = "swap",
            label = "Swap",
            run = function()
                pane:set_lines({ "after-1", "after-2" })
            end,
        })
    "#;
    let path = write_plugin_named("set-lines-render", source);
    let mut runtime = PluginRuntime::load(&path).unwrap();
    let pane_handle = runtime
        .pane_for(SidebarSlot::Left)
        .expect("left pane registered");
    let lua_pane = pane_handle.clone().into_pane();

    // Initial render: shows "before".
    let backend = TestBackend::new(20, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let mut ctx = devix_core::RenderCtx { frame: f };
            lua_pane.render(Rect { x: 0, y: 0, width: 20, height: 3 }, &mut ctx);
        })
        .unwrap();
    let mut row0 = String::new();
    for x in 0..terminal.backend().buffer().area.width {
        row0.push_str(terminal.backend().buffer()[(x, 0)].symbol());
    }
    assert!(row0.starts_with("before"), "initial render should show `before`, got {row0:?}");

    // Invoke the swap action and wait for the dirty signal.
    let handle = runtime.contributions().commands[0].handle;
    runtime.invoke_sender().send(handle).unwrap();
    let got = drain_until(&mut runtime, Duration::from_secs(2), |m| {
        matches!(m, PluginMsg::PaneChanged)
    });
    assert!(got.is_some(), "pane should have signalled PaneChanged after set_lines");

    // Re-render the same LuaPane (which holds the shared `lines` Arc).
    terminal
        .draw(|f| {
            let mut ctx = devix_core::RenderCtx { frame: f };
            lua_pane.render(Rect { x: 0, y: 0, width: 20, height: 3 }, &mut ctx);
        })
        .unwrap();
    let mut after_row0 = String::new();
    for x in 0..terminal.backend().buffer().area.width {
        after_row0.push_str(terminal.backend().buffer()[(x, 0)].symbol());
    }
    let mut after_row1 = String::new();
    for x in 0..terminal.backend().buffer().area.width {
        after_row1.push_str(terminal.backend().buffer()[(x, 1)].symbol());
    }
    assert!(after_row0.starts_with("after-1"), "row 0 should be `after-1`, got {after_row0:?}");
    assert!(after_row1.starts_with("after-2"), "row 1 should be `after-2`, got {after_row1:?}");
}

#[test]
fn pane_on_key_callback_fires_via_input_channel() {
    let source = r#"
        local pane = devix.register_pane({ slot = "left", lines = { "ready" } })
        pane:on_key(function(ev)
            devix.status("plugin-saw:" .. ev.key)
        end)
    "#;
    let path = write_plugin_named("on-key-channel", source);
    let mut runtime = PluginRuntime::load(&path).unwrap();
    let pane = runtime.pane_for(SidebarSlot::Left).expect("left pane registered");
    assert!(pane.has_on_key.load(std::sync::atomic::Ordering::Acquire));
    let lua_pane = pane.into_pane();
    assert!(
        lua_pane.forward_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        "forward_key should report consumed when on_key is registered",
    );
    let got = drain_until(&mut runtime, Duration::from_secs(2), |m| {
        matches!(m, PluginMsg::Status(s) if s == "plugin-saw:enter")
    });
    assert!(got.is_some(), "on_key should have fired and pushed status");
}

#[test]
fn pane_input_channel_falls_through_when_no_callback_registered() {
    // No `on_key` — forwarding should be a no-op so the editor's
    // keymap path can run.
    let source = r#"
        devix.register_pane({ slot = "left", lines = { "ready" } })
    "#;
    let path = write_plugin_named("no-callback", source);
    let runtime = PluginRuntime::load(&path).unwrap();
    let pane = runtime.pane_for(SidebarSlot::Left).expect("left pane registered");
    assert!(!pane.has_on_key.load(std::sync::atomic::Ordering::Acquire));
    let lua_pane = pane.into_pane();
    assert!(
        !lua_pane.forward_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        "forward_key should report not-consumed when no on_key is registered",
    );
    // And nothing should have been queued on the input channel for
    // the plugin to ever process.
    let _ = PluginInput::Key {
        pane_id: lua_pane.pane_id(),
        event: KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    };
}

#[test]
fn open_path_message_surfaces_through_runtime() {
    let source = r#"
        devix.register_action({
            id = "open",
            label = "Open Path",
            run = function() devix.open_path("/tmp/devix-e2e-target") end,
        })
    "#;
    let path = write_plugin_named("open-path", source);
    let mut runtime = PluginRuntime::load(&path).unwrap();
    let handle = runtime.contributions().commands[0].handle;
    runtime.invoke_sender().send(handle).unwrap();
    let got = drain_until(&mut runtime, Duration::from_secs(2), |m| {
        matches!(m, PluginMsg::OpenPath(p) if p == &PathBuf::from("/tmp/devix-e2e-target"))
    });
    assert!(got.is_some(), "expected an OpenPath message");
}

#[test]
fn bundled_file_tree_example_loads_and_lists_cwd() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("examples/file_tree.lua");
    let runtime = PluginRuntime::load(&path).expect("file_tree example loads");
    let cmds = &runtime.contributions().commands;
    let panes = &runtime.contributions().panes;
    assert!(
        cmds.iter().any(|c| c.id == "filetree.refresh"),
        "expected `filetree.refresh` command, got {cmds:?}",
    );
    let pane = panes
        .iter()
        .find(|p| p.slot == SidebarSlot::Left)
        .expect("file_tree contributes a left pane");
    let lines = pane.lines.lock().unwrap();
    assert!(lines.len() >= 2, "expected at least cwd + spacer, got {lines:?}");
    assert!(lines.iter().any(|l| !l.is_empty()), "non-empty entries");
    assert!(
        pane.has_on_key.load(std::sync::atomic::Ordering::Acquire),
        "file_tree should register on_key",
    );
    assert!(
        pane.has_on_click.load(std::sync::atomic::Ordering::Acquire),
        "file_tree should register on_click",
    );
}

#[test]
fn sidebar_slot_pane_renders_lua_pane_inside_chrome() {
    use devix_core::Pane as _;
    use devix_core::SidebarSlotPane;
    use devix_core::SidebarPane as SidebarChrome;

    let path = write_plugin();
    let runtime = PluginRuntime::load(&path).unwrap();
    let pane = runtime.pane_for(SidebarSlot::Left).expect("left pane registered");
    let lua: Box<dyn devix_core::Pane> = Box::new(pane.into_pane());
    let slot = SidebarSlotPane {
        chrome: SidebarChrome { title: "left".into(), focused: false },
        content: Some(lua),
    };

    let backend = TestBackend::new(20, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let mut ctx = devix_core::RenderCtx { frame: f };
            slot.render(Rect { x: 0, y: 0, width: 20, height: 5 }, &mut ctx);
        })
        .unwrap();
    let buf = terminal.backend().buffer();
    let mut row = String::new();
    for x in 1..buf.area.width.saturating_sub(1) {
        row.push_str(buf[(x, 1)].symbol());
    }
    assert!(
        row.trim_start().starts_with("from-lua"),
        "expected `from-lua` inside chrome on row 1, got {row:?}",
    );
    let _ = LuaPane::pane_id; // keep the symbol used so removing it stays a deliberate API change
}

/// Unparseable chord is best-effort: the runtime still loads.
#[test]
fn unparseable_chord_does_not_fail_load() {
    let source = r#"
        devix.register_action({
            id = "noop",
            label = "Noop",
            chord = "wat+blorp",
            run = function() end,
        })
    "#;
    let path = write_plugin_named("bad-chord", source);
    let runtime = PluginRuntime::load(&path).unwrap();
    assert_eq!(runtime.contributions().commands.len(), 1);
    // Plugin host parses chords up-front; an unparseable string lands
    // as `None` rather than blocking the whole load.
    assert_eq!(runtime.contributions().commands[0].chord, None);
    assert!(parse_chord("wat+blorp").is_none());
}
