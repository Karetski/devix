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
use devix_app::clipboard;
use devix_app::{AppContext, Application, EventSink};
use devix_editor::{DocId, Editor, build_registry, cmd, default_keymap};
use devix_panes::Theme;
use devix_plugin::{MsgSink, PluginMsg, PluginRuntime, default_plugin_path};

fn main() -> Result<()> {
    let path = std::env::args().nth(1).map(PathBuf::from);

    // Build the channel up front so producers wire directly into it.
    let (sink, rx) = EventSink::channel();

    let mut editor = Editor::open(path)?;
    {
        let sink = sink.clone();
        editor.attach_disk_sink(Arc::new(move |doc: DocId| {
            let _ = sink.pulse(move |ctx: &mut AppContext<'_>| handle_disk_changed(ctx, doc));
        }));
    }

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

/// Disk watcher reported a change for `doc`. Three-way handling:
/// dirty buffer → mark pending and prompt; active+clean → reload via
/// the command path; background+clean → silent reload + cursor clamp.
fn handle_disk_changed(ctx: &mut AppContext<'_>, doc: DocId) {
    let active_doc_id = ctx.editor.active_cursor().map(|c| c.doc);
    let dirty = ctx
        .editor
        .documents
        .get(doc)
        .map(|d| d.buffer.dirty())
        .unwrap_or(false);

    if dirty {
        if let Some(d) = ctx.editor.documents.get_mut(doc) {
            d.disk_changed_pending = true;
        }
        ctx.request_redraw();
    } else if Some(doc) == active_doc_id {
        ctx.run(&cmd::ReloadFromDisk);
    } else if let Some(d) = ctx.editor.documents.get_mut(doc) {
        if d.reload_from_disk().is_ok() {
            let max = ctx.editor.documents[doc].buffer.len_chars();
            for cursor in ctx.editor.cursors.values_mut() {
                if cursor.doc == doc {
                    cursor.selection.clamp(max);
                }
            }
        }
        ctx.request_redraw();
    }
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
