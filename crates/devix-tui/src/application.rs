//! `Application` — the runtime.
//!
//! Single struct that owns every long-lived resource by direct field. No
//! delegate trait, no DI container, no globals, no `Service` trait
//! around what is in practice one input thread plus a plugin runtime.
//! UIKit analogue: `UIApplication` collapsed with the one true delegate.

use std::io::{Stdout, stdout};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use anyhow::Result;
use crossterm::execute;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use devix_core::{CommandRegistry, Editor, Keymap, LayoutCtx};
use devix_core::{Clipboard, Theme};
use devix_core::PluginRuntime;
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};

use crate::context::AppContext;
use crate::event_sink::{EventSink, LoopMessage};
use crate::input as events;
use crate::input_thread::InputThread;
use crate::interpreter as render;

const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(3);

pub struct Application<B: Backend = CrosstermBackend<Stdout>> {
    pub editor: Editor,
    pub commands: CommandRegistry,
    pub keymap: Keymap,
    pub theme: Theme,
    pub clipboard: Box<dyn Clipboard>,

    /// Plugin host, if one was loaded. Holding the runtime keeps its
    /// worker thread alive; dropping it closes the channels and the
    /// worker exits.
    plugin: Option<PluginRuntime>,

    sink: EventSink,
    rx: Receiver<LoopMessage>,
    terminal: Terminal<B>,
    pub(crate) quit: bool,
    pub(crate) dirty: bool,
    /// `true` when `new()` entered raw mode + alternate screen. `Drop`
    /// restores only when this is set, so test harnesses using
    /// `for_test` (which never entered raw mode) don't emit escape
    /// sequences on tear-down.
    owns_tty: bool,
}

impl Application<CrosstermBackend<Stdout>> {
    /// Build the application around a pre-built `(EventSink, Receiver)`
    /// pair. The caller wires producers (the editor's disk watcher, the
    /// plugin runtime's message sink, future LSP clients) against
    /// `sink.clone()` before constructing the application; producers
    /// are born outside the runtime, so the channel has to exist first.
    pub fn new(
        editor: Editor,
        commands: CommandRegistry,
        keymap: Keymap,
        theme: Theme,
        clipboard: Box<dyn Clipboard>,
        sink: EventSink,
        rx: Receiver<LoopMessage>,
    ) -> Result<Self> {
        let terminal = build_terminal_with_panic_hook()?;
        Ok(Self {
            editor,
            commands,
            keymap,
            theme,
            clipboard,
            plugin: None,
            sink,
            rx,
            terminal,
            quit: false,
            dirty: true,
            owns_tty: true,
        })
    }
}

impl<B: Backend> Application<B> {
    pub fn sink(&self) -> &EventSink {
        &self.sink
    }

    /// Hand a loaded plugin runtime to the application. The runtime
    /// already wired its message sink into the loop channel at load
    /// time; the application just holds it so the worker stays alive.
    pub fn set_plugin(&mut self, runtime: PluginRuntime) {
        self.plugin = Some(runtime);
    }

    pub fn run(mut self) -> Result<()> {
        let input = InputThread::spawn(self.sink.clone(), self.editor.bus.clone())?;
        while !self.quit {
            if self.dirty {
                self.render()?;
                self.dirty = false;
            }
            match self.rx.recv() {
                Ok(LoopMessage::Input(ev)) => self.deliver_input(ev),
                Ok(LoopMessage::Wake) => {
                    // No-op — the bus drain below picks up whatever
                    // the wake-sender pushed.
                }
                Ok(LoopMessage::Quit) => self.quit = true,
                Err(_) => break,
            }
            // Drain cross-thread `publish_async` pulses on the main
            // thread per pulse-bus.md. Typed dispatch via drain_into
            // (foundations-review 2026-05-07) so handlers can take
            // `&mut Editor` directly. Bus subscribers (Fn handlers
            // for cross-cutting concerns like logging / plugins)
            // are still served by the bus's regular `drain`; the
            // typed-dispatch loop runs first so the editor mutates
            // before subscribers observe a derived RenderDirty.
            self.dispatch_typed_pulses();
            self.editor.bus.drain();
        }
        input.shutdown(SHUTDOWN_DEADLINE);
        // Dropping the plugin runtime closes its channels; its worker
        // exits its `tokio::select!` loop.
        drop(self.plugin.take());
        let _ = self.terminal.show_cursor();
        Ok(())
    }

    /// Run a one-shot delivery with a freshly-built `AppContext`.
    /// The context's `dirty_request` / `quit_request` flags are
    /// folded back into the runtime after the delivery returns —
    /// replaces the `Effect::{Redraw, Quit}` queueing path retired
    /// in T-63.
    fn with_context<F: FnOnce(&mut AppContext<'_>)>(&mut self, label: &str, f: F) {
        let mut dirty_request = false;
        let mut quit_request = false;
        {
            let mut ctx = AppContext {
                editor: &mut self.editor,
                commands: &self.commands,
                keymap: &self.keymap,
                theme: &self.theme,
                clipboard: self.clipboard.as_mut(),
                sink: &self.sink,
                dirty_request: &mut dirty_request,
                quit_request: &mut quit_request,
            };
            if catch_unwind(AssertUnwindSafe(|| f(&mut ctx))).is_err() {
                eprintln!("{} handler panicked; dropping", label);
            }
        }
        if dirty_request {
            self.dirty = true;
        }
        if quit_request {
            self.quit = true;
        }
    }

    fn deliver_input(&mut self, ev: crossterm::event::Event) {
        self.with_context("input", |ctx| events::handle(ev, ctx));
    }

    fn dispatch_typed_pulses(&mut self) {
        use devix_protocol::pulse::Pulse;
        let mut pulses: Vec<Pulse> = Vec::new();
        self.editor.bus.drain_into(&mut pulses);
        for pulse in pulses {
            match pulse {
                Pulse::DiskChanged { path, fs_path } => {
                    self.with_context("disk-changed", |ctx| {
                        handle_disk_changed(ctx, &path, &fs_path);
                    });
                }
                Pulse::RenderDirty { .. } => {
                    self.dirty = true;
                }
                Pulse::OpenPathRequested { fs_path, .. } => {
                    self.with_context("open-path-requested", |ctx| {
                        if ctx.editor.active_frame().is_none() {
                            if let Some(fid) = ctx.editor.panes.frames().first().copied() {
                                ctx.editor.focus_frame(fid);
                            }
                        }
                        ctx.run(&devix_core::cmd::OpenPath(fs_path));
                    });
                }
                Pulse::ShutdownRequested => {
                    self.quit = true;
                }
                // Other variants land in T-63 as more producers
                // migrate. Unhandled pulses fall back to bus
                // subscribers via `drain` immediately after.
                _ => {
                    // Re-enqueue so bus subscribers see it. Cheap —
                    // round-trips through the cross-thread queue.
                    self.editor.bus.publish_async(pulse);
                }
            }
        }
    }

    fn render(&mut self) -> Result<()> {
        let Self {
            ref mut terminal,
            ref mut editor,
            ref keymap,
            ref theme,
            ref commands,
            ..
        } = *self;
        terminal.draw(|frame| {
            let area = frame.area();
            editor.layout(area);

            let focused_leaf = editor.panes.at_path(&editor.focus).and_then(|n| devix_core::editor::registry::pane_leaf_id(n));
            let layout_ctx = LayoutCtx {
                documents: &editor.documents,
                cursors: &editor.cursors,
                theme,
                render_cache: &editor.render_cache,
                focused_leaf,
            };
            editor.panes.render(area, frame, &layout_ctx);

            if let Some(modal) = editor.modal.as_ref() {
                render::paint_modal(modal, area, frame, theme, commands, keymap);
            }
        })?;
        Ok(())
    }
}

impl<B: Backend> Drop for Application<B> {
    fn drop(&mut self) {
        if self.owns_tty {
            let _ = restore_terminal();
        }
    }
}

fn build_terminal_with_panic_hook() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen, EnableMouseCapture)?;

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        prev(info);
    }));

    Ok(Terminal::new(CrosstermBackend::new(stdout()))?)
}

fn restore_terminal() -> Result<()> {
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    Ok(())
}

mod test_support {
    use super::*;
    use ratatui::backend::TestBackend;

    impl Application<TestBackend> {
        /// Construct an `Application` against a `TestBackend` for tests.
        /// Skips raw-mode entry and the panic hook the production
        /// constructor performs.
        pub fn for_test(
            editor: Editor,
            commands: CommandRegistry,
            keymap: Keymap,
            theme: Theme,
            clipboard: Box<dyn Clipboard>,
            size: (u16, u16),
        ) -> Self {
            let (sink, rx) = EventSink::channel();
            let backend = TestBackend::new(size.0, size.1);
            let terminal = Terminal::new(backend).expect("test terminal");
            Self {
                editor,
                commands,
                keymap,
                theme,
                clipboard,
                plugin: None,
                sink,
                rx,
                terminal,
                quit: false,
                dirty: true,
                owns_tty: false,
            }
        }

        /// One iteration of the loop with non-blocking recv. Returns
        /// `false` when no message was available, so test drivers don't
        /// hang.
        pub fn try_step(&mut self) -> bool {
            if self.dirty {
                let _ = self.render();
                self.dirty = false;
            }
            match self.rx.try_recv() {
                Ok(LoopMessage::Input(ev)) => self.deliver_input(ev),
                Ok(LoopMessage::Wake) => {}
                Ok(LoopMessage::Quit) => self.quit = true,
                Err(_) => return false,
            }
            self.dispatch_typed_pulses();
            self.editor.bus.drain();
            true
        }

        pub fn buffer(&self) -> &ratatui::buffer::Buffer {
            self.terminal.backend().buffer()
        }

        pub fn is_dirty(&self) -> bool {
            self.dirty
        }

        pub fn is_quit(&self) -> bool {
            self.quit
        }

        pub fn force_render(&mut self) {
            let _ = self.render();
        }
    }
}

/// Disk watcher reported a change for `path`. Three-way handling:
/// dirty buffer → mark pending and prompt; active+clean → reload via
/// the command path; background+clean → silent reload + cursor clamp.
/// Migrated from main.rs (T-61) to live alongside the typed-pulse
/// dispatch that calls it.
fn handle_disk_changed(
    ctx: &mut AppContext<'_>,
    path: &devix_protocol::path::Path,
    _fs_path: &std::path::Path,
) {
    let Some(doc) = devix_core::DocId::id_from_path(path) else { return };
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
        ctx.run(&devix_core::cmd::ReloadFromDisk);
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
