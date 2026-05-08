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
use devix_tui::{AppContext, Application, EventSink};
use devix_core::{Editor, build_registry, cmd, default_keymap};
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

    let mut plugin_runtime = match default_plugin_path() {
        Some(p) => {
            let msg_sink: MsgSink = {
                let sink = sink.clone();
                Arc::new(move |msg| {
                    let _ = sink.pulse(move |ctx: &mut AppContext<'_>| {
                        handle_plugin_msg(ctx, msg)
                    });
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

    let mut app = Application::new(
        editor, commands, keymap, theme, clipboard, sink, rx,
    )?;

    if let Some(rt) = plugin_runtime {
        app.set_plugin(rt);
    }

    app.run()
}


/// Plugin host pushed a message; route it.
fn handle_plugin_msg(ctx: &mut AppContext<'_>, msg: PluginMsg) {
    match msg {
        PluginMsg::Status(_) => {}
        PluginMsg::PaneChanged => ctx.request_redraw(),
        PluginMsg::OpenPath(path) => {
            if ctx.editor.active_frame().is_none() {
                if let Some(fid) = ctx.editor.root.frames().first().copied() {
                    ctx.editor.focus_frame(fid);
                }
            }
            ctx.run(&cmd::OpenPath(path));
        }
    }
}
