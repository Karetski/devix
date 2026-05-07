//! `Application` — the runtime.
//!
//! Single struct that owns every long-lived resource by direct field. No
//! delegate trait, no DI container, no globals, no `Service` trait
//! around what is in practice one input thread plus a plugin runtime.
//! UIKit analogue: `UIApplication` collapsed with the one true delegate.

use std::collections::VecDeque;
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
use crate::effect::Effect;
use crate::event_sink::{EventSink, LoopMessage, PulseFn};
use crate::events;
use crate::input::InputThread;
use crate::render;

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

    pub(crate) effects: VecDeque<Effect>,
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
            effects: VecDeque::new(),
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
        let input = InputThread::spawn(self.sink.clone())?;
        while !self.quit {
            if self.dirty {
                self.render()?;
                self.dirty = false;
            }
            match self.rx.recv() {
                Ok(LoopMessage::Input(ev)) => self.deliver_input(ev),
                Ok(LoopMessage::Pulse(p)) => self.deliver_pulse(p),
                Ok(LoopMessage::Quit) => self.quit = true,
                Err(_) => break,
            }
            self.flush_effects();
        }
        input.shutdown(SHUTDOWN_DEADLINE);
        // Dropping the plugin runtime closes its channels; its worker
        // exits its `tokio::select!` loop.
        drop(self.plugin.take());
        let _ = self.terminal.show_cursor();
        Ok(())
    }

    fn context(&mut self) -> AppContext<'_> {
        AppContext {
            editor: &mut self.editor,
            commands: &self.commands,
            keymap: &self.keymap,
            theme: &self.theme,
            clipboard: self.clipboard.as_mut(),
            sink: &self.sink,
            effects: &mut self.effects,
        }
    }

    fn deliver_input(&mut self, ev: crossterm::event::Event) {
        let mut ctx = self.context();
        if catch_unwind(AssertUnwindSafe(|| events::handle(ev, &mut ctx))).is_err() {
            eprintln!("input handler panicked; dropping event");
        }
    }

    fn deliver_pulse(&mut self, p: PulseFn) {
        let mut ctx = self.context();
        if catch_unwind(AssertUnwindSafe(|| p(&mut ctx))).is_err() {
            eprintln!("pulse panicked; dropping");
        }
    }

    fn flush_effects(&mut self) {
        while let Some(e) = self.effects.pop_front() {
            match e {
                Effect::Redraw => self.dirty = true,
                Effect::Quit => self.quit = true,
                Effect::Run(f) => {
                    let mut ctx = self.context();
                    let _ = catch_unwind(AssertUnwindSafe(|| f(&mut ctx)));
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

            let focused_leaf = editor.root.at_path(&editor.focus).and_then(|n| n.leaf_id());
            let layout_ctx = LayoutCtx {
                documents: &editor.documents,
                cursors: &editor.cursors,
                theme,
                render_cache: &editor.render_cache,
                focused_leaf,
            };
            editor.root.render(area, frame, &layout_ctx);

            if let Some(modal) = editor.modal.as_ref() {
                render::paint_modal(modal.as_ref(), area, frame, theme, commands, keymap);
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
                effects: VecDeque::new(),
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
                Ok(LoopMessage::Pulse(p)) => self.deliver_pulse(p),
                Ok(LoopMessage::Quit) => self.quit = true,
                Err(_) => return false,
            }
            self.flush_effects();
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
