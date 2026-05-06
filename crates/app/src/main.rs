//! devix binary entry point.
//!
//! Build the loop channel first so producers (the editor's disk-watch
//! callback, the plugin runtime's message sink) can push pulses
//! directly into the run loop without intermediate polling threads.
//! Then construct the editor, optionally load a plugin, and hand the
//! pre-wired channel to the application.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use devix_app::clipboard;
use devix_app::{Application, DiskChanged, EventSink, PluginEmitted, PluginService};
use devix_editor::{DocId, Editor, build_registry, default_keymap};
use devix_panes::Theme;
use devix_plugin::{MsgSink, PluginRuntime, default_plugin_path};

fn main() -> Result<()> {
    let path = std::env::args().nth(1).map(PathBuf::from);

    // Build the channel up front so producers wire directly into it.
    let (sink, rx) = EventSink::channel();

    let mut editor = Editor::open(path)?;
    {
        let sink = sink.clone();
        editor.attach_disk_sink(Arc::new(move |doc: DocId| {
            let _ = sink.pulse(DiskChanged { doc });
        }));
    }

    let mut commands = build_registry();
    let mut keymap = default_keymap();

    let mut plugin_runtime = match default_plugin_path() {
        Some(p) => {
            let msg_sink: MsgSink = {
                let sink = sink.clone();
                Arc::new(move |msg| {
                    let _ = sink.pulse(PluginEmitted { msg });
                })
            };
            PluginRuntime::load_with_sink(&p, msg_sink).ok()
        }
        None => None,
    };

    if let Some(rt) = plugin_runtime.as_mut() {
        rt.install(&mut commands, &mut keymap, &mut editor);
    }

    let theme = Theme::default();
    let clipboard = clipboard::init();

    let mut app = Application::with_channel(
        editor, commands, keymap, theme, clipboard, sink, rx,
    )?;

    if let Some(rt) = plugin_runtime {
        app.add_service(PluginService::new(rt));
    }

    app.run()
}
