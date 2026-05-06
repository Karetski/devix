# devix — Runtime Spec

The runtime is the layer that owns the terminal, the run loop, and every long-lived background subsystem. Today's implementation spreads across eight files in `crates/app/src/` — `main.rs`, `app.rs`, `runtime.rs`, `events.rs`, `plugin.rs`, `watcher.rs`, `clipboard.rs`, `render.rs` — with no shared abstraction for the runtime concerns they each touch. This spec replaces the six load-bearing ones (`app`, `runtime`, `events`, `plugin`, `watcher`, plus the bin glue in `main`) with **five named types** — `Application`, `Service`, `Pulse`, `Effect`, `AppContext`. `render.rs`'s body lifts into `Application::render`; `clipboard.rs` stays as the binary-side helper that produces a `Box<dyn Clipboard>` and hands it to `Application::new`. Every present and foreseeable runtime concern (LSP, settings reload, debug adapters, multi-window, plugin hot reload) is built from a small combination of those five plus two existing extension primitives (`EditorCommand`, `Pane`) from the editor and panes crates.

The design is grounded in source-level reading of nine reference systems — Helix, Kakoune, Neovim, Emacs, VSCode, IntelliJ, gpui (ships inside Zed), floem (used by Lapce), UIKit. File:line citations live in Appendix A. Long-form forensic notes were captured in the conversation transcript that produced this spec; clone the references locally to follow citations.

## The five primitives

### 1. `Application` — the runtime

```rust
pub struct Application<B: Backend = CrosstermBackend<std::io::Stdout>> {
    pub editor: Editor,
    pub commands: CommandRegistry,
    pub keymap: Keymap,
    pub theme: Theme,
    pub clipboard: Box<dyn Clipboard>,

    services: Vec<Box<dyn Service>>,
    effects: VecDeque<Effect>,
    sink: EventSink,
    rx: Receiver<LoopMessage>,
    terminal: Terminal<B>,
    quit: bool,
    dirty: bool,
}
```

Single struct. Owns everything by direct fields. No DI, no globals, no `Arc<Mutex<App>>`. UIKit analogue: `UIApplication` collapsed with the one true delegate. There is no `ApplicationDelegate` because Rust doesn't need that indirection — neither do Helix, Kakoune, Neovim, Emacs, gpui, or floem. VSCode and IntelliJ use DI containers for the same wiring role; we explicitly reject DI (see the rejection list under *Why this is the right shape*). The `B: Backend` generic exists only so tests can swap `CrosstermBackend` for `TestBackend`; production code uses the default and never spells `B`.

### 2. `Service` — long-lived background subsystem

```rust
pub trait Service: Send + 'static {
    fn name(&self) -> &'static str;
    fn start(&mut self, sink: EventSink) -> Result<()>;
    fn stop(self: Box<Self>, deadline: Duration) {}
}
```

One open primitive for "subsystem that owns a thread (or executor) and pushes events back through a channel". `InputService`, `WatcherService`, `PluginService`, future `LspService`, future `DapService` — each is one `impl Service`. UIKit analogue: a long-lived agent (`NSURLSession`-shape) configured once, delivering events back through a known channel.

### 3. `Pulse` — typed message from a service to the runtime

```rust
pub trait Pulse: Send + 'static {
    fn deliver(self: Box<Self>, ctx: &mut AppContext<'_>);
    fn name(&self) -> &'static str { std::any::type_name::<Self>() }
}

pub enum LoopMessage {
    Input(crossterm::event::Event),
    Pulse(Box<dyn Pulse>),
    Quit,
}
```

One open primitive for "thing pushed from off-main-thread to main-thread". Each pulse type knows how to deliver itself with `&mut AppContext`. Helix's `Jobs::callbacks` shape, trait-form instead of closure-form for named tracing.

```rust
struct DiskChanged { doc: DocId }
impl Pulse for DiskChanged {
    fn deliver(self: Box<Self>, ctx: &mut AppContext<'_>) {
        // Watcher service emits one DiskChanged per detection per doc
        // (instead of today's `drain_disk_events` batch in watcher.rs:12-56,
        // which collected all affected DocIds first). Three-way body —
        // mirrors current behavior:
        //   dirty buffer:           set disk_changed_pending = true;
        //                           ctx.request_redraw().
        //   active && clean:        ctx.run(&cmd::ReloadFromDisk) — clears
        //                           the pending flag, reloads, clamps the
        //                           active cursor; ctx.run already enqueues
        //                           Effect::Redraw.
        //   background && clean:    direct reload + clamp every cursor on
        //                           this doc (no command path);
        //                           ctx.request_redraw().
    }
}

struct PluginEmitted { msg: PluginMsg }
impl Pulse for PluginEmitted {
    fn deliver(self: Box<Self>, ctx: &mut AppContext<'_>) {
        match self.msg {
            PluginMsg::OpenPath(p) => {
                // Mirrors plugin.rs:105-114: focus the first frame if none
                // is active, then run cmd::OpenPath through the regular
                // command path.
                if ctx.editor.active_frame().is_none() {
                    if let Some(fid) = devix_editor::frame_ids(ctx.editor.root.as_ref())
                        .first().copied()
                    {
                        ctx.editor.focus_frame(fid);
                    }
                }
                ctx.run(&cmd::OpenPath(p));
            }
            PluginMsg::PaneChanged => ctx.request_redraw(),
            PluginMsg::Status(_)   => {} // status surface deferred
        }
    }
}
```

A new subsystem-to-runtime message is a struct + `impl Pulse`. There is no central enum that grows.

### 4. `Effect` — deferred mutation

```rust
pub type EffectFn = Box<dyn for<'a> FnOnce(&mut AppContext<'a>)>;

pub enum Effect {
    Redraw,
    Quit,
    Run(EffectFn),
}
```

One closed primitive for "runtime-internal deferred work, drained between messages". Solves three current problems at once: the manual `dirty: bool` flag, the `pending_scroll: isize` accumulator, and the borrow conflicts when one mutation needs to trigger another. `Effect::Run` is the open escape hatch — anything not in the closed enum is a closure. The `for<'a>` HRTB lets the boxed closure accept whatever lifetime `AppContext` is reborrowed at on the next loop iteration; without it, the closure would be tied to one specific lifetime and the type wouldn't compile.

The closed-vs-open call: Effect's variants are *runtime-internal coordination*; the runtime needs to know how to handle each kind. New runtime ops add variants. New domain ops use `Run`. Same shape as gpui's `Effect` enum (`crates/gpui/src/app.rs:2573-2594`).

### 5. `AppContext` — the unified `&mut` surface

```rust
pub struct AppContext<'a> {
    pub editor: &'a mut Editor,
    pub commands: &'a CommandRegistry,        // read-only in-loop
    pub keymap: &'a Keymap,                   // read-only in-loop
    pub theme: &'a Theme,                     // read-only in-loop
    pub clipboard: &'a mut dyn Clipboard,
    pub sink: &'a EventSink,
    effects: &'a mut VecDeque<Effect>,
}

impl AppContext<'_> {
    pub fn request_redraw(&mut self) { self.effects.push_back(Effect::Redraw); }
    pub fn quit(&mut self)           { self.effects.push_back(Effect::Quit); }
    pub fn defer<F>(&mut self, f: F)
        where F: for<'a> FnOnce(&mut AppContext<'a>) + 'static
    {
        self.effects.push_back(Effect::Run(Box::new(f)));
    }

    /// Invoke an editor command. Takes `&dyn EditorCommand` so both
    /// concrete struct-typed commands (via deref coercion) and the
    /// `Arc<dyn EditorCommand>` instances stored in `CommandRegistry`
    /// (`crates/editor/src/commands/registry.rs`) and `Keymap`
    /// (`crates/editor/src/commands/keymap.rs`) can flow through one
    /// entry point.
    ///
    /// Bridges to `devix_editor::Context` (which expects an immediate
    /// `quit: &mut bool` flag); if the command sets it, translates to
    /// `Effect::Quit` so quit stays deferred at the runtime layer.
    /// Viewport calc body lifts from `crates/app/src/events.rs::run_command`
    /// (active-frame rect + gutter-width based on line-count digits).
    pub fn run(&mut self, action: &dyn EditorCommand) {
        let viewport = active_viewport(self.editor);   // free helper, lifted from events.rs
        let mut quit = false;
        {
            // Explicit reborrows: the editor/clipboard fields on AppContext are
            // themselves &mut references; we need to reborrow them for shorter
            // lifetimes here so `self` isn't partially moved.
            let mut cx = devix_editor::Context {
                editor:    &mut *self.editor,
                clipboard: &mut *self.clipboard,
                quit:      &mut quit,
                viewport,
                commands:  self.commands,
            };
            action.invoke(&mut cx);
        }
        if quit { self.effects.push_back(Effect::Quit); }
        self.effects.push_back(Effect::Redraw);
    }
}
```

Built fresh from `&mut Application` for the duration of one delivery. No `Arc`, no `RefCell`. Single owner, single thread, nothing to lock. UIKit analogue: the receiving end of the responder chain. MLIR analogue: `PatternRewriter` — the unified mutation API.

Three fields are immutable refs (`commands`, `keymap`, `theme`) because the loop only reads them; mutation happens at startup before any pulse delivers. Two deferred features want to mutate them:
- **Hot plugin reload** wants `&mut commands` + `&mut keymap` to re-register actions and re-bind chords.
- **Settings/theme reload** wants `&mut theme`.

Both are blocked by the same constraint: `Effect::Run` receives `AppContext`, not `&mut Application`, so its closure inherits these read-only refs. When the first of those features lands, the cheapest fix is to relax the relevant fields to `&'a mut`; the alternative is a new `Effect` variant whose body in `flush_effects` operates on `&mut Application` directly. Pick when forced.

### Supporting handle: `EventSink`

```rust
use std::sync::mpsc::{SyncSender, SendError};

#[derive(Clone)]
pub struct EventSink(SyncSender<LoopMessage>);

impl EventSink {
    /// Blocks if the channel is full. Backpressure is intentional: a slow
    /// main loop should slow down the producer (input thread, plugin host,
    /// LSP transport) rather than silently drop messages. Returns `Err`
    /// only when the receiver has been dropped — i.e. the run loop has
    /// exited; producer threads should treat that as a shutdown signal
    /// and exit their own read loops.
    pub fn input(&self, ev: crossterm::event::Event) -> Result<(), SendError<LoopMessage>> {
        self.0.send(LoopMessage::Input(ev))
    }
    pub fn pulse<P: Pulse>(&self, p: P) -> Result<(), SendError<LoopMessage>> {
        self.0.send(LoopMessage::Pulse(Box::new(p)))
    }
    /// Used by signal handlers (SIGTERM/SIGINT) and the test harness to ask
    /// the run loop to exit from outside the main thread; in-loop quit goes
    /// through `Effect::Quit` via `AppContext::quit`.
    pub fn quit(&self) -> Result<(), SendError<LoopMessage>> {
        self.0.send(LoopMessage::Quit)
    }
}
```

The channel is `std::sync::mpsc::sync_channel(1024)`; `EventSink` wraps the sender, the run loop owns the receiver. Cloneable; cloned once per Service at `start()`. Producers ignore `Err` only when they have no shutdown path; long-running threads (input reader, plugin worker, future LSP transport) propagate it to break their own loop. Replaces today's `Waker` plus per-subsystem queues plus `IDLE_TIMEOUT` polling. Not a primitive in its own right — just the cross-thread handle services need to participate.

## The run loop

Sync. One thread. One channel. No tokio at the runtime level. Services that need tokio (the plugin host already has one) bring their own runtime; the framework stays sync.

```rust
impl Application<CrosstermBackend<std::io::Stdout>> {
    pub fn new(
        editor: Editor,
        commands: CommandRegistry,
        keymap: Keymap,
        theme: Theme,
        clipboard: Box<dyn Clipboard>,
    ) -> Result<Self> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1024);
        // Builder mirrors today's `Tty::enter` + `Terminal::new(CrosstermBackend::new(stdout()))`,
        // plus installs the panic hook that restores the terminal.
        let terminal = build_terminal_with_panic_hook()?;
        let mut app = Self {
            editor, commands, keymap, theme, clipboard,
            services: Vec::new(),
            effects: VecDeque::new(),
            sink: EventSink(tx),
            rx,
            terminal,
            quit: false,
            dirty: true,
        };
        app.add_service(InputService::default());           // crossterm reader, always present
        Ok(app)
    }
}

impl<B: Backend> Application<B> {
    pub fn add_service(&mut self, s: impl Service) {
        self.services.push(Box::new(s));
    }

    pub fn run(mut self) -> Result<()> {
        const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(3);
        self.start_services();              // includes InputService
        while !self.quit {
            if self.dirty {
                self.render()?;
                self.dirty = false;
            }
            // recv only errors when every sender drops; `self.sink` keeps
            // one alive for the loop's lifetime, so the `Err` arm is
            // defensive — pattern-matched explicitly rather than using
            // `unwrap` so test/teardown paths that drop the sink early
            // still exit cleanly instead of panicking.
            match self.rx.recv() {
                Ok(LoopMessage::Input(ev)) => self.deliver_input(ev),
                Ok(LoopMessage::Pulse(p))  => self.deliver_pulse(p),
                Ok(LoopMessage::Quit)      => self.quit = true,
                Err(_)                     => break,
            }
            self.flush_effects();
        }
        self.stop_services(SHUTDOWN_DEADLINE);
        Ok(())
    }

    fn start_services(&mut self) {
        // Disjoint borrows: `sink` shares `self.sink` (immutable),
        // `retain_mut` mutably borrows `self.services` — different fields,
        // both fine at once.
        let sink = &self.sink;
        self.services.retain_mut(|s| match s.start(sink.clone()) {
            Ok(()) => true,
            Err(e) => { eprintln!("service {} failed to start: {e}", s.name()); false }
        });
    }

    fn stop_services(&mut self, deadline: Duration) {
        for service in self.services.drain(..) {
            service.stop(deadline);          // Box<dyn Service> consumed
        }
    }

    fn context(&mut self) -> AppContext<'_> {
        AppContext {
            editor:    &mut self.editor,
            commands:  &self.commands,
            keymap:    &self.keymap,
            theme:     &self.theme,
            clipboard: self.clipboard.as_mut(),
            sink:      &self.sink,
            effects:   &mut self.effects,
        }
    }

    fn deliver_input(&mut self, ev: crossterm::event::Event) {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        let mut ctx = self.context();
        // Walks the focused-leaf responder chain (Pane::handle), the modal
        // slot if open, and the keymap. The new `events::handle` is the
        // current `crates/app/src/events.rs::handle_event` renamed and
        // reshaped to take `&mut AppContext` instead of `&mut App`.
        if catch_unwind(AssertUnwindSafe(|| events::handle(ev, &mut ctx))).is_err() {
            eprintln!("input handler panicked; dropping event");
        }
    }

    fn deliver_pulse(&mut self, p: Box<dyn Pulse>) {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        let name = p.name();
        let mut ctx = self.context();
        if catch_unwind(AssertUnwindSafe(|| p.deliver(&mut ctx))).is_err() {
            eprintln!("pulse {name} panicked; dropping");
        }
    }

    fn flush_effects(&mut self) {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        while let Some(e) = self.effects.pop_front() {
            match e {
                Effect::Redraw => self.dirty = true,
                Effect::Quit   => self.quit = true,
                Effect::Run(f) => {
                    let mut ctx = self.context();
                    let _ = catch_unwind(AssertUnwindSafe(|| f(&mut ctx)));
                }
            }
        }
    }

    fn render(&mut self) -> Result<()> {
        // Splitting borrow: `terminal.draw(|frame| ...)` mutably borrows
        // `self.terminal`, so the closure cannot also capture `&mut self`.
        // Destructure into the disjoint pieces the closure needs.
        let Self {
            ref mut terminal,
            ref mut editor,
            ref keymap,
            ref theme,
            ref commands,
            ..  // sink/rx/services/effects/quit/dirty/clipboard not used during paint
        } = *self;
        terminal.draw(|frame| {
            let area = frame.area();
            editor.layout(area);                  // pre-paint mutation (single &mut Editor)
            // Body lifts from crates/app/src/render.rs: `editor.root.render(area, &mut ctx)`
            // wrapped in `RenderServices::scope`, plus the modal Pane on top. RenderServices
            // (crates/editor/src/services.rs) takes `documents`, `cursors`, `theme`, render
            // cache, focused leaf, and a plugin-sidebar resolver — `commands` and `keymap`
            // are needed only by the palette-paint path (`paint_palette` in render.rs).
        })?;
        Ok(())
    }
}
```

Terminal lifecycle (raw mode entry, alternate screen, panic-restore hook) lives in `Application::new` and a `Drop for Application` (not shown above for brevity), not in `run()`. The constructor takes ownership of stdout, builds the `Terminal<B>`, and registers the panic hook that restores the terminal on unwind; the `Drop` impl restores raw mode and the cursor on normal exit. Mirrors the existing `crates/app/src/runtime.rs::Tty` pattern, just owned by `Application` instead of being a free RAII guard.

Notes:

- **`rx.recv()` blocks until a message arrives.** No idle timer; no per-tick polling. Backpressure lives on the channel itself (`sync_channel(1024)`).
- **Render before block.** Drain effects, paint if dirty, *then* block on the next message. Avoids "key arrives, partial redraw, key processed, second redraw" thrash. Pattern from Kakoune (`src/main.cc:828`) and Neovim (`src/nvim/state.c:71`).
- **`catch_unwind` at the outer boundary, not per-call.** Three landing pads: one per input delivery, one per pulse delivery, one per `Effect::Run`. Cost is negligible when no panic; high value when one fires (one bad plugin pulse, key handler, or deferred closure doesn't crash the editor). Same shape as Emacs's `safe_run_hooks` (`src/keyboard.c:1893`).
- **`InputService` handles terminal input.** Its `start()` spawns a thread that calls `crossterm::event::poll(POLL_TIMEOUT)` then `read()` when ready, pushing `LoopMessage::Input` into the sink; the poll timeout (e.g. 100 ms) is what makes shutdown bounded. `stop()` flips an atomic "stopping" flag the loop checks each iteration; the next poll-timeout returns and the thread exits. Without `poll`, a blocking `read()` would not see the flag until the next terminal event.
- **Service start failure is log-and-continue.** The Service is dropped, the editor launches without it. Caveat: a `start()` that fails *after* spawning a thread or registering a watcher may leak the spawned resource — the dropped Service hasn't had `stop()` called on it. Auto-disable on repeat panics (VSCode's three-strikes pattern) is deferred. For v1, services with non-trivial startup should clean up their own partial state inside `start()` before returning `Err`.

## The render path

Unchanged from current code in spirit. `Editor::layout(area)` runs pre-paint mutation; the structural Pane tree paints itself onto the back buffer; modal Panes paint last (z-top); ratatui's `Buffer::diff` (existing) computes cell-level changes and writes only changed cells. The body lifts from current `crates/app/src/render.rs::render` into `Application::render` with a splitting borrow on `*self` to disjoint-borrow `terminal` from `editor`/`commands`/`keymap`/`theme` (shown in the run-loop sketch above). No frame fence in v1; if tree-sitter incremental parse produces visible stale highlights, lift Helix's `lock_frame` pattern then.

## Diagnosis — what's wrong today

Six observations, file:line. Each is solved by one of the five primitives, plus the supporting `EventSink` handle (for observation #4 below) and the existing `Pane::handle` responder chain (for observation #5).

1. **`App` is a god-struct that pretends to be a delegate** (`crates/app/src/app.rs:20-33`). Mixes editor, commands, keymap, theme, clipboard, plugin, plus runtime concerns (`quit`, `dirty`, `pending_scroll`).
2. **`ApplicationDelegate` adds no power, only ceremony** (`runtime.rs:45-68`). Six methods on a trait with one implementor. UIKit needs a delegate because Objective-C wants subclassing; Rust doesn't.
3. **Subsystems are bolted into `tick()`** (`app.rs:76-83`). Hand-drains the disk watcher, then the plugin host, then applies a deferred scroll. Every reference editor unifies wakeup at the syscall (Helix `tokio::select!`, Neovim libuv, Kakoune `pselect`, Emacs `select(2)`); we re-poll on a 100ms cadence.
4. **`Waker` plus per-subsystem queues plus `IDLE_TIMEOUT`** (`runtime.rs:36, 86-91`). The worst of both worlds: subsystems push into private queues *and* the loop polls every drain function.
5. **Input routing is special-cased per subsystem** (`events.rs:50-57, 140-147`). Plugins ship as `LuaPane` (already a `Pane` impl) but the binary routes around `Pane::handle`.
6. **`pending_scroll: isize` is a deferred-mutation queue with one slot** (`app.rs:28`, `events.rs:178-180`). Mouse-wheel handler accumulates an integer; `tick` pulls it out. We need a real queue.

## Why this is the right shape

Lattner's principle: *a few well-chosen abstractions carry the system; new features extend existing concepts instead of adding new top-level ones.*

| MLIR | UIKit | devix runtime |
|---|---|---|
| `MLIRContext` | `UIApplication` | `Application` |
| `Dialect` (contributes typed Ops) | framework (contributes events/views) | `Service` (contributes typed pulses) |
| `Operation` (typed payload + verb) | `UIEvent` | `Pulse` |
| `Pass` action | run-loop step | `Effect` |
| `PatternRewriter` | event handler | `AppContext` |

The runtime exposes **five framework types** (`Application`, `Service`, `Pulse`, `Effect`, `AppContext`). The **extension surface** is four items: the two extensible framework types (`Service`, `Pulse`) plus two from the existing crates (`EditorCommand`, `Pane`):
- **a new `Service` impl** — adds a new background subsystem
- **a new `Pulse` impl** — adds a new typed message kind from a service
- **a new `EditorCommand` impl** — adds a new editor mutation (existing in `devix-editor`)
- **a new `Pane` impl** — adds a new visual element (existing in `devix-panes`)

`Application`, `Effect`, and `AppContext` are framework types that don't get extended — `Application` is the singleton owner, `Effect` is the closed enum of runtime ops with `Effect::Run` as the open closure escape hatch, `AppContext` is the unified `&mut` surface threaded through delivery. LSP, debug adapter, settings reload, remote sessions — each is some combination of `Service + Pulse + EditorCommand + Pane` that grows no top-level concept. Multi-scene (workspace-per-window) and an out-of-process plugin marketplace appear in the rejection list below; both would require additions inside `Application` (a `scenes` slotmap, an extension-host transport) but still extend through the same primitives at the boundary.

What this design *rejects*, with the trigger that would force revisiting:

- **No DI container** (VSCode, IntelliJ). Constructor-parameter decorators want runtime metadata; devix has roughly five long-lived subsystems (`InputService`, `WatcherService`, `PluginService`, future `LspService`, future `DapService`) whose types are known at compile time, so fields on `Application` are simpler. Forced by: dynamically-discovered subsystems with unknown types.
- **No reactive runtime** (floem). Signals/effects/memos earn their rent in a UI with hundreds of cells whose values depend on each other; devix has a small fixed subsystem set and a render-the-grid loop where the dep graph is hand-written in `fn render()`. Forced by: never (we won't get there at TUI scale).
- **No entity-map / lease pattern** (gpui). Helpful when subsystems mutually borrow at high rates. Forced by: a feature where two services need to mutate the same Editor concurrently.
- **No tokio at the top** (Helix). Helix's `block_in_place + helix_lsp::block_on` smell at 4+ sites is the proof. Sync top-level + per-service tokio is cleaner. Forced by: 50+ I/O streams in the runtime itself, or async commands in the responder chain.
- **No `Mode` enum primitive** (UIKit run-loop modes). `editor.modal: Option<Box<dyn Pane>>` already *is* the modal indicator. Forced by: a Pulse that should pause during a modal palette.
- **No `Phase` enum primitive** (VSCode lifecycle). Internal `Application` field, not a primitive. Forced by: third-party Service authors needing to declare "I should start at phase X".
- **No three-rail `Jobs`** (Helix `callbacks` + `status` + `wait_futures`). One `Effect::Run` rail covers v1. Forced by: LSP format-on-save (writes that must complete before quit).
- **No `FrameFence`** (Helix `lock_frame`). Forced by: visible stale tree-sitter highlight after rapid edits.
- **No three-strikes auto-disable for repeat-panicking services** (VSCode `ExtensionHostCrashTracker`). Forced by: shipping plugins to third parties.
- **No subscription RAII tokens** (IntelliJ `Disposer`, floem scope tree). Forced by: plugin hot reload.
- **No emergency save on uncaught panic.** The existing terminal-restore panic hook stays; buffer recovery is not attempted. Forced by: any user complaint about lost edits.
- **No multi-scene / `UIScene` split.** Single-scene now. Reserved seam: when needed, `Application::scenes: SlotMap<SceneId, Scene>` where `Scene` owns `Editor + (commands, keymap, theme, clipboard)`. Currently those fields live directly on `Application`. Forced by: workspace-per-window, multi-project mode, or headless plugin invocation.
- **No plugin extension host as a separate process** (VSCode). Forced by: shipping a publishable plugin marketplace where third-party crashes must not affect the editor.
- **No accessed-entities side-channel** (gpui per-window dirty tracking). Forced by: complex pane compositions where individual panes paint expensive layouts.

## Reference distillation

One paragraph each. file:line in Appendix A; clone the projects locally to follow citations.

**Helix** (Rust, tokio, ratatui). Single `Application` struct with `tokio::select!` over six event arms; `Editor::wait_event` is a nested second-tier select. Components return `EventResult::{Consumed,Ignored}(Option<Callback>)` so layers mutate the compositor without holding `&mut Compositor`. Redraw via `tokio::sync::Notify` + 33ms debounce + `RwLock<()>` frame fence. Three-rail `Jobs`. `should_close = tree.is_empty()` invariant. Smells: `block_in_place + block_on` from synchronous command paths (forensic readings cite four call sites in `compositor.rs`, `commands.rs`, `commands/typed.rs`, `commands/lsp.rs`); the panic hook restores the terminal but does not save buffers. **Devix takes**: the responder-chain shape (already in our `Pane::handle`), `should_close = is_empty()` invariant.

**Kakoune** (C++, `pselect`, client/server). Server loop is 22 lines; per-source pending-keys queue is the entire backpressure model; `m_ui_pending` bitmask + `m_last_setup` hash for render coalescing. `HookManager::run_hook` catches per-hook exceptions and continues. **Devix takes**: per-handler error containment, render coalescing via "drain effects, paint once".

**Neovim** (C, libuv, Lua). Loop is the `VimState` recursive state machine; async events smuggled into the key stream as synthetic `K_EVENT` keystrokes. Triple queue: events / fast_events / thread_events with `uv_async_t` cross-thread doorbell under mutex. `Channel` union over `Process | LibuvProc | PtyProc | RStream | StdioPair | InternalState`. Buffer-update push model: in-process synchronous notification at the mutation site. **Devix takes**: render-before-block discipline (`update_screen()` runs only when about to wait for input). The buffer-update push model is *not* what `Pulse` does — Pulse is cross-thread async messaging; in-process synchronous notification of buffer subscribers (when devix grows that, e.g. for a syntax tree mirror) belongs in `devix-editor`'s mutation path, not in the runtime.

**Emacs** (C + elisp). One `select(2)` over input fd, all process pipes, sockets, timers. `safe_run_hooks` removes hooks that error from the list — auto-uninstall. `MODIFF` counter on each buffer. Redisplay is preemptable. **Devix takes**: the outer-boundary `catch_unwind`-then-log-then-continue pattern (we don't auto-uninstall yet — see the three-strikes deferral above).

**VSCode** (TypeScript). Workbench bootstraps `IInstantiationService`; "the application" is the closure of services reachable through DI. Lifecycle phases driven by `ILifecycleService.phase` setter. Extensions in a separate process with `ExtensionHostCrashTracker` enforcing three-strikes-in-60s. **Devix takes**: three-strikes pattern (deferred — see the rejection list above).

**IntelliJ** (Java/Kotlin). Application is itself a DI container; Project is a child container. EDT + read/write/IW locks. ModalityState tags every queued runnable. `Disposer.register(parent, child)` is the universal teardown tree. **Devix takes**: Disposer-shaped teardown (deferred — see the subscription-RAII-tokens entry in the rejection list).

**gpui** (Rust, async-task). Whole graph in one `RefCell<App>`. `Entity<T>` is `(EntityId, refcount)` indexing into `EntityMap`. `lease()` *physically removes* the box from the map for the duration of `&mut`. `flush_effects` is a FIFO `VecDeque<Effect>` with `pending_notifications` deduplication. Beautiful trick: every `entity.read(cx)` during paint writes to `accessed_entities`; the next time any of those entities mutates, only windows that read it re-paint. **Devix takes**: the FIFO drain-until-empty shape (our `flush_effects` collapses many `Effect::Redraw` to one paint by setting `self.dirty = true` at the consumer rather than gpui's enqueue-time `pending_notifications` set; same outcome, simpler structure for a closed-enum Effect). Lease + accessed-entities + per-emitter dedup deferred — premature for our scale.

**floem** (Rust, winit, reactive). Thread-local `RUNTIME` with `current_effect`/`current_scope`/`signals`. Effects auto-track their dep set on each run. Scope tree is the lifecycle. `create_signal_from_channel` bridges a worker thread's `mpsc::Receiver` into a signal via `EventLoopProxy::send_event(UserEvent::Idle)`. **Devix takes**: the worker→mpsc→main-loop-doorbell shape, stripped of the signal layer (our `EventSink` *is* this).

**UIKit**. NSRunLoop modes gate which sources fire per iteration. Strict iteration order. Responder chain: hit-test → target view → superviews → view controller → window → application → app delegate, with claim-vs-forward as the two outcomes. UIScene since iOS 13 separates per-window state from process-level state. **Devix takes**: claim-vs-forward (already `Outcome::{Consumed,Ignored}`); UIScene shape reserved as a future seam.

## Migration plan

Six phases. Each compiles green. P0 is the only additive phase (it lays down the new primitive types as empty stubs); P1–P5 each strictly remove existing complexity rather than adding parallel complexity.

**P0 — primitive stubs.** Convert `crates/app` from binary-only to library + binary (helix-term shape): add `[lib]` alongside the existing `[[bin]]` in `crates/app/Cargo.toml`, add `src/lib.rs` exporting empty `Application`, `Service`, `Pulse`, `Effect`, `AppContext`, `EventSink`. No consumers; `main.rs` keeps using its current free functions. Validates names before any other code commits. ~50 lines.

**P1 — `Application` owns Editor + resources directly.** Delete `App` god-struct and `ApplicationDelegate`. Inline fields. Behavior unchanged. ~200 lines net delete.

**P2 — Effect queue replaces hand-managed `dirty` + `pending_scroll`.** Add `effects: VecDeque<Effect>`. `request_redraw → Effect::Redraw`; `pending_scroll → Effect::Run`. ~80 lines net delete.

**P3 — Input as a Service, pulses as the multiplex.** Replace `LoopEvent` with `LoopMessage`; convert input thread into `InputService` (uses `crossterm::event::poll` for bounded shutdown). `EventSink` is the cloneable handle. `Waker`, `IDLE_TIMEOUT`, and `MAX_DRAIN_PER_TICK` all delete; subsystems push pulses directly. ~100 lines net delete.

**P4 — Disk watcher & plugin host as Services.** Move `crates/app/src/watcher.rs` (free functions) into `services/watcher.rs`. The plugin migration is two-part because `crates/app/src/plugin.rs` is more than a `drain_*` helper — it also owns the input/render routing helpers (`sidebar_pane`, `focused_plugin_slot`, `forward_key_to_plugin`, `forward_click_to_plugin`, `plugin_slot_at`, `scroll_plugin_pane`). P4 moves the *runtime* surface (`PluginRuntime` ownership, `drain_plugin_events`, message routing) into `services/plugin.rs::PluginService`. The bridge: PluginService spawns a thread that holds `msg_rx: UnboundedReceiver<PluginMsg>` (today drained by `PluginRuntime::drain_messages`); each `PluginMsg` becomes `sink.pulse(PluginEmitted{msg})`. The existing `Wakeup` callback (`devix_plugin::Wakeup = Arc<dyn Fn() + Send + Sync + 'static>`) is replaced by direct `EventSink::pulse` calls — no more "doorbell + drain", just typed pulses. The render/input-routing helpers stay where they are until P5. ~150 lines net delete.

**P5 — Input routing flows uniformly through `Pane::handle`.** Two prerequisites:
- **Persistent `LuaPane` identity.** Today `crates/app/src/render.rs:103-106` builds a fresh `LuaPane` on every render via a closure resolver. For the responder chain to deliver `handle()` calls to the plugin pane, it must live persistently in the layout tree. Move plugin sidebar contributions into the `Editor`'s structural Pane tree as long-lived `Box<dyn Pane>` leaves, owned by the appropriate sidebar slot.
- **Delete plugin-specific routing** in `events.rs:50-57, 140-147` and the helpers in `plugin.rs` they call. The responder chain becomes the only routing.

`events.rs` collapses to a translate-and-walk shim; the input-routing helpers in `plugin.rs` delete. ~250 lines net delete.

After P5: `crates/app/src/` shrinks to a small named composable runtime — summing the per-phase deltas (~+50 then ~-780 across P1–P5) suggests roughly a third of current size, give or take how much new Service plumbing each phase adds back. Adding LSP, settings hot reload, debug adapter after that — each is one new Service file plus its Pulse types.

## What this kills

| Today | Replaced by |
|---|---|
| `App` struct | `Application` (one concern: the runtime) |
| `ApplicationDelegate` trait | (deleted) |
| `Waker` | `EventSink` |
| `App::dirty: bool` + `request_redraw()` | `Effect::Redraw` + `dirty: bool` (one place) |
| `App::pending_scroll: isize` | `Effect::Run(...)` |
| `IDLE_TIMEOUT`, `MAX_DRAIN_PER_TICK` | gone — channel is the backpressure |
| `App::tick` (drain disk + drain plugin) | each becomes a Pulse from its Service |
| `events.rs` plugin-aware routing | `Pane::handle` walks tree uniformly |
| `crates/app/src/{watcher,plugin}.rs` (free functions) | `services::{WatcherService, PluginService}` |

## Test strategy

Borrowed from Helix's `helix-term/tests/test/helpers.rs`. No real terminal needed; the `B: Backend` generic on `Application` is the only injection point required.

```rust
#[cfg(test)]
impl Application<TestBackend> {
    /// Constructs an `Application` against a `TestBackend` of the given
    /// dimensions plus default registry/keymap/theme/`NoClipboard`.
    /// Skips the panic-hook + raw-mode entry that the production
    /// constructor performs.
    pub fn for_test(editor: Editor, size: (u16, u16)) -> Self { /* … */ }

    /// One iteration of the loop with non-blocking recv. Mirrors `run`'s
    /// order — render-if-dirty first, then `try_recv` + deliver + flush.
    /// Returns `false` if no message was available, so the caller doesn't
    /// hang in tests.
    pub fn try_step(&mut self) -> bool {
        if self.dirty { let _ = self.render(); self.dirty = false; }
        match self.rx.try_recv() {
            Ok(LoopMessage::Input(ev)) => self.deliver_input(ev),
            Ok(LoopMessage::Pulse(p))  => self.deliver_pulse(p),
            Ok(LoopMessage::Quit)      => self.quit = true,
            Err(_)                     => return false,
        }
        self.flush_effects();
        true
    }

    pub fn buffer(&self) -> &ratatui::buffer::Buffer { self.terminal.backend().buffer() }
    pub fn is_dirty(&self) -> bool                   { self.dirty }
    pub fn is_quit(&self) -> bool                    { self.quit }
    pub fn sink(&self) -> &EventSink                 { &self.sink }
}
```

- **Drive input** — `app.sink().input(key('a'))?`.
- **Drive pulses** — `app.sink().pulse(MyPulse{...})?`.
- **Drain available work** — `while app.try_step() {}`.
- **Assert** — `app.buffer()`, `app.editor.<state>`, `app.is_dirty()`, `app.is_quit()`, etc.

Failure isolation tests use a `PanickingService` stub; assert the runtime continues, the panicking pulse is dropped, the next pulse delivers normally. The fields `quit`/`dirty`/`terminal` stay private; tests inspect them through the `#[cfg(test)]` accessor methods above so production code can't reach in.

## Appendix A — research index

For each pattern in this spec, the canonical reference worth reading first. Line numbers are approximate (target codebases move); pattern names are stable. Web URLs are given for UIKit references where there's no source repo.

| Pattern | Reference | Location |
|---|---|---|
| `Application` struct shape | Helix | `helix-term/src/application.rs:70-105` |
| `tokio::select!` two-tier multiplex | Helix | `helix-term/src/application.rs:325-365`, `helix-view/src/editor.rs:2285-2325` |
| `EventResult::{Consumed,Ignored}(callback)` | Helix | `helix-term/src/compositor.rs:144-182` |
| Notify-shaped redraw | Helix | `helix-event/src/redraw.rs` |
| `RwLock<()>` frame fence | Helix | `helix-event/src/redraw.rs` (`lock_frame`, `start_frame`) |
| Three-rail Jobs | Helix | `helix-term/src/job.rs:48-170` |
| `should_close = is_empty()` | Helix | `helix-view/src/editor.rs:2149-2151` |
| Server loop in 22 lines | Kakoune | `src/main.cc:825-846` |
| Per-source pending-keys queue | Kakoune | `src/client.cc:99-132` |
| `m_ui_pending` bitmask | Kakoune | `src/window.hh:76-86` |
| Per-hook try/catch isolation | Kakoune | `src/hook_manager.cc:160-165` |
| State machine + K_EVENT trick | Neovim | `src/nvim/state.c:34-107` |
| Triple queue + `uv_async_t` doorbell | Neovim | `src/nvim/event/loop.{h,c}` |
| Channel union | Neovim | `src/nvim/channel.h:17-105` |
| `safe_run_hooks` auto-uninstall | Emacs | `src/keyboard.c:1893-1978` |
| MODIFF counter | Emacs | `src/buffer.h` (definition); `src/keyboard.c:1478` (redisplay-gating compare in `command_loop_1`) |
| Single `select(2)` multiplex | Emacs | `src/process.c::wait_reading_process_output` (~5336-5800) |
| `createDecorator` (DI in 18 lines) | VSCode | `src/vs/platform/instantiation/common/instantiation.ts:109-126` |
| Lifecycle phases | VSCode | `src/vs/workbench/services/lifecycle/common/lifecycle.ts` |
| ExtensionHostCrashTracker | VSCode | `src/vs/workbench/services/extensions/common/abstractExtensionService.ts:92` |
| EDT + read/write/IW lock model | IntelliJ | `platform/core-api/src/com/intellij/openapi/application/Application.java:30-63` |
| ModalityState | IntelliJ | `platform/core-api/src/com/intellij/openapi/application/ModalityState.java` |
| Disposer tree | IntelliJ | `platform/util/src/com/intellij/openapi/util/Disposer.java` |
| Lease pattern | gpui | `crates/gpui/src/app/entity_map.rs:133-212` |
| `flush_effects` + Notify dedup | gpui | `crates/gpui/src/app.rs:1412-1489` |
| accessed_entities → window invalidator | gpui | `crates/gpui/src/app.rs:976-1003`, `window.rs:117-145` |
| `create_signal_from_channel` | floem | `floem/src/ext_event.rs:166-201` |
| Scope tree as lifecycle | floem | `floem/reactive/src/{scope,id}.rs` |
| NSRunLoop modes | UIKit | developer.apple.com/documentation/foundation/nsrunloop |
| Responder chain | UIKit | developer.apple.com/documentation/uikit/touches_presses_and_gestures/using_responders_and_the_responder_chain_to_handle_events |
| UIScene multi-window | UIKit | developer.apple.com/documentation/uikit/uiscene |
