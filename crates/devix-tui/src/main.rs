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
    apply_keymap_overrides, keymap_overrides_path, parse_manifest_bytes, theme_from_manifest,
};
use devix_core::Theme;
use devix_core::{MsgSink, PluginMsg, PluginRuntime, default_plugin_path};

fn main() -> Result<()> {
    let path = std::env::args().nth(1).map(PathBuf::from);

    // Build the channel up front so producers wire directly into it.
    let (sink, rx) = EventSink::channel();

    let mut editor = Editor::open(path)?;
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

    let mut plugin_runtime = match default_plugin_path() {
        Some(p) => {
            // Plugin → editor messages: every variant publishes a
            // typed pulse onto the bus, then wakes the main loop
            // (LoopMessage::Wake) so its blocking `rx.recv` returns
            // and drains the bus on the next tick. T-63 retires the
            // EventSink closure path here entirely.
            let bus = editor.bus.clone();
            let wake_sink = sink.clone();
            let msg_sink: MsgSink = Arc::new(move |msg| {
                match msg {
                    PluginMsg::PaneChanged => {
                        bus.publish_async(devix_protocol::pulse::Pulse::RenderDirty {
                            reason: devix_protocol::pulse::DirtyReason::Frontend,
                        });
                    }
                    PluginMsg::Status(_) => return,
                    PluginMsg::OpenPath(fs_path) => {
                        bus.publish_async(devix_protocol::pulse::Pulse::OpenPathRequested {
                            fs_path,
                            source: devix_protocol::pulse::InvocationSource::Plugin,
                        });
                    }
                }
                let _ = wake_sink.wake();
            });
            PluginRuntime::load_with_sink(&p, msg_sink).ok()
        }
        None => None,
    };

    if let Some(rt) = plugin_runtime.as_mut() {
        rt.install(&mut commands, &mut keymap, &mut editor);
    }

    // Load the "default" theme from the embedded built-in manifest.
    // Falls back to the in-source `Theme::default()` if the manifest
    // doesn't carry the id (defence-in-depth — the embedded manifest
    // does, and `builtin_manifest::*` tests gate on it).
    let theme = parse_manifest_bytes(
        devix_core::BUILTIN_MANIFEST.as_bytes(),
        std::path::Path::new("<builtin>"),
    )
    .ok()
    .and_then(|m| theme_from_manifest(&m, "default"))
    .unwrap_or_else(Theme::default);
    let clipboard = clipboard::init();

    let mut app = Application::new(
        editor, commands, keymap, theme, clipboard, sink, rx,
    )?;

    if let Some(rt) = plugin_runtime {
        app.set_plugin(rt);
    }

    app.run()
}
