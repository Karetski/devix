//! devix binary entry point.
//!
//! Build the loop channel first so producers (the editor's disk-watch
//! callback, the plugin runtime's message sink) can push closures
//! directly into the run loop without intermediate polling threads.
//! Then construct the editor, optionally load a plugin, and hand the
//! pre-wired channel to the application.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use devix_tui::clipboard;
use devix_tui::{Application, EventSink};
use devix_core::{Editor, build_registry, default_keymap};
use devix_core::manifest_loader::{
    apply_keymap_overrides, discover_plugin_manifests, keymap_overrides_path, load_manifest,
    parse_manifest_bytes, plugin_dir,
};
use devix_core::settings_store::settings_overrides_path;
use devix_core::{MsgSink, PluginMsg, PluginRuntime, default_plugin_path};

fn main() -> Result<()> {
    let path = std::env::args().nth(1).map(PathBuf::from);

    // Build the channel up front so producers wire directly into it.
    let (sink, rx) = EventSink::channel();

    let mut editor = Editor::open(path)?;
    // Seed the editor's theme + settings stores from the embedded
    // built-in manifest. Activate the "default" theme so
    // `editor.theme` reflects manifest-declared scope styles before
    // the first render. T-112 (theme) + T-113 (settings) made these
    // stores live on `Editor`.
    if let Ok(builtin) = parse_manifest_bytes(
        devix_core::BUILTIN_MANIFEST.as_bytes(),
        std::path::Path::new("<builtin>"),
    ) {
        editor.theme_store.register_from_manifest(&builtin);
        editor.settings_store.lock().unwrap().register_from_manifest(&builtin);
    }
    let _ = editor.set_theme("default");
    // Disk-watch events flow as Pulse::DiskChanged on the editor's
    // bus; Application::run drains and dispatches them to
    // handle_disk_changed on the main thread (T-61).

    let mut commands = build_registry();
    let mut keymap = default_keymap();

    // Apply user keymap overrides from
    // `$XDG_CONFIG_HOME/devix/keymap-overrides.json` (or the
    // ~/.config/devix/... fallback). Missing file is silent. Errors
    // surface to stderr so the editor still starts on a typo.
    if let Some(p) = keymap_overrides_path() {
        if let Err(e) = apply_keymap_overrides(&mut keymap, &commands, &p) {
            eprintln!("devix: keymap-overrides ignored ({}): {e}", p.display());
        }
    }

    // Plugin → editor message-sink factory shared by the legacy
    // single-file load path (DEVIX_PLUGIN) and the manifest-driven
    // discovery loop (T-110). Each variant publishes a typed pulse
    // onto the bus and wakes the main loop.
    let make_msg_sink = |bus: devix_core::PulseBus,
                         wake_sink: EventSink|
     -> MsgSink {
        Arc::new(move |msg| {
            match msg {
                PluginMsg::PaneChanged => {
                    let _ = bus.publish_async(devix_protocol::pulse::Pulse::RenderDirty {
                        reason: devix_protocol::pulse::DirtyReason::Frontend,
                    });
                }
                PluginMsg::Status(_) => return,
                PluginMsg::OpenPath(fs_path) => {
                    let _ = bus.publish_async(devix_protocol::pulse::Pulse::OpenPathRequested {
                        fs_path,
                        source: devix_protocol::pulse::InvocationSource::Plugin,
                    });
                }
            }
            // Wake the main loop regardless of whether the typed
            // pulse made it onto the bus — drop-newest may have
            // shed it under load, but the wake/dispatch is what
            // actually moves the loop forward.
            let _ = wake_sink.wake();
        })
    };

    // Manifest-driven plugin discovery (T-110, F-3 follow-up
    // 2026-05-12). Three phases so a plugin's startup-time
    // `devix.setting("my.key")` already observes both its
    // manifest-declared defaults *and* the user's overrides:
    //   1. Walk `plugin_dir()`, parse every `manifest.json`, and
    //      register its themes + settings on the editor stores.
    //   2. Apply user settings overrides from
    //      `$XDG_CONFIG_HOME/devix/settings.json` so plugin runtimes
    //      observe overrides during startup, not just defaults.
    //   3. For each parsed manifest, load the runtime and install
    //      commands/keymap/panes. Errors surface as `Pulse::PluginError`
    //      on the editor's bus or to stderr at this entry-point.
    struct PendingPlugin {
        manifest: devix_protocol::manifest::Manifest,
        entry: PathBuf,
    }
    let mut pending: Vec<PendingPlugin> = Vec::new();
    if let Some(dir) = plugin_dir() {
        match discover_plugin_manifests(&dir) {
            Ok(manifests) => {
                for manifest_path in manifests {
                    let plugin_root = match manifest_path.parent() {
                        Some(p) => p.to_path_buf(),
                        None => continue,
                    };
                    let manifest = match load_manifest(&manifest_path) {
                        Ok(m) => m,
                        Err(e) => {
                            eprintln!(
                                "devix: plugin manifest at `{}` rejected: {}",
                                manifest_path.display(),
                                e
                            );
                            continue;
                        }
                    };
                    let entry = manifest
                        .entry
                        .as_deref()
                        .map(|s| plugin_root.join(s))
                        .unwrap_or_else(|| plugin_root.join("main.lua"));
                    if !entry.is_file() {
                        eprintln!(
                            "devix: plugin `{}` entry `{}` not found",
                            manifest.name,
                            entry.display()
                        );
                        continue;
                    }
                    // Phase 1: register theme + settings before any
                    // runtime can call `devix.setting(...)`.
                    editor.theme_store.register_from_manifest(&manifest);
                    editor
                        .settings_store
                        .lock()
                        .unwrap()
                        .register_from_manifest(&manifest);
                    pending.push(PendingPlugin { manifest, entry });
                }
            }
            Err(e) => {
                eprintln!(
                    "devix: scanning plugin dir `{}` failed: {e}",
                    dir.display()
                );
            }
        }
    }

    // Phase 2: apply user settings overrides from
    // `$XDG_CONFIG_HOME/devix/settings.json`. Type mismatches and
    // out-of-list enum values surface to stderr; the rest of the
    // file's keys still apply. Runs **before** plugin runtime load
    // so plugins observe overrides during startup.
    if let Some(p) = settings_overrides_path() {
        if let Err(e) = editor
            .settings_store
            .lock()
            .unwrap()
            .apply_overrides_from_file(&p)
        {
            eprintln!("devix: settings overrides ignored ({}): {e}", p.display());
        }
    }

    // Phase 3: load each plugin's runtime, install its contributions,
    // and remember the runtime under its manifest name so subsequent
    // per-plugin events (e.g., `Pulse::PluginLoaded` after a supervised
    // restart) route back to the right runtime.
    let mut plugin_runtimes: Vec<(String, PluginRuntime)> = Vec::new();
    for PendingPlugin { manifest, entry } in pending {
        let msg_sink = make_msg_sink(editor.bus.clone(), sink.clone());
        let mut runtime = match PluginRuntime::load_supervised_with_settings(
            &entry,
            msg_sink,
            editor.bus.clone(),
            editor.settings_store.clone(),
        ) {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "devix: plugin `{}` failed to load: {}",
                    manifest.name, e
                );
                continue;
            }
        };
        let bus_for_install = editor.bus.clone();
        runtime.install_with_manifest(
            &mut commands,
            &mut keymap,
            &mut editor,
            &manifest,
            &bus_for_install,
        );
        plugin_runtimes.push((manifest.name, runtime));
    }

    // Backwards-compat single-file path. `DEVIX_PLUGIN` (or its
    // legacy default location) still loads one Lua file with the
    // pre-T-110 in-Lua registration semantics. Manifest-driven
    // plugins land at `/plugin/<name>/cmd/<id>`; this path keeps
    // working at `/cmd/<id>` until callers migrate. Loaded after
    // settings overrides apply so it sees the same store state
    // as manifest-driven plugins. The legacy path has no manifest
    // name; we synthesize one stable key so it lives in the same
    // map as manifest-driven plugins.
    if let Some(p) = default_plugin_path() {
        let msg_sink = make_msg_sink(editor.bus.clone(), sink.clone());
        if let Ok(mut rt) = PluginRuntime::load_supervised_with_settings(
            &p,
            msg_sink,
            editor.bus.clone(),
            editor.settings_store.clone(),
        ) {
            rt.install(&mut commands, &mut keymap, &mut editor);
            plugin_runtimes.push(("__legacy_single_file__".to_string(), rt));
        }
    }

    let clipboard = clipboard::init();

    let mut app = Application::new(editor, commands, keymap, clipboard, sink, rx)?;

    // F-4 follow-up 2026-05-12: hand every loaded runtime to the
    // application keyed by its manifest name. The previous shape
    // held at most one runtime and `std::mem::forget`-ed the rest
    // (no drop on shutdown; `Pulse::PluginLoaded` for restarts
    // routed to the wrong runtime). The map now keeps every worker
    // alive and routable.
    for (name, rt) in plugin_runtimes {
        app.add_plugin(name, rt);
    }

    app.run()
}
