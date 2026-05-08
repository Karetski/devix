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
            // Plugin → editor messages: PaneChanged migrates to the
            // bus as Pulse::RenderDirty (T-62 slice); Status is a
            // no-op today; OpenPath still routes through the closure
            // path because it requires invoking a typed command (the
            // command-invocation pulse mapping lands when plugin
            // contributions go through the protocol layer, T-110+).
            let sink_for_msgs = sink.clone();
            let bus = editor.bus.clone();
            let msg_sink: MsgSink = Arc::new(move |msg| match msg {
                PluginMsg::PaneChanged => {
                    bus.publish_async(devix_protocol::pulse::Pulse::RenderDirty {
                        reason: devix_protocol::pulse::DirtyReason::Frontend,
                    });
                }
                PluginMsg::Status(_) => {}
                PluginMsg::OpenPath(_) => {
                    let _ = sink_for_msgs.pulse(move |ctx: &mut AppContext<'_>| {
                        handle_plugin_msg(ctx, msg)
                    });
                }
            });
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
