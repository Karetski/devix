# devix — layering note

Short, opinionated design note answering one question: *what does each crate actually own, and what is the smallest set of primitives that lets new features extend without smuggling?* The recent runtime-spec landing solved the run-loop shape but left layering choices that force four workarounds (TLS render-services, `borrowed_pane` Forwarder, downcast-driven tree walks, two `Application` constructors). This note is the framing we'll use to attack them in order — each smell maps to one principle below.

## The six crates today

| Crate | Owns today | Should own |
|---|---|---|
| `devix-text` | rope, selection, char/byte ops | same |
| `devix-syntax` | tree-sitter wrapper, highlight queries | same |
| `devix-panes` | `Pane` trait + `Event`/`Rect` + chrome widgets (TabStrip, SidebarPane, Palette) + composites (TabbedPane, SidebarSlotPane) | trait + chrome widgets only — composites that need editor borrows move out |
| `devix-editor` | `Editor`, documents/cursors slotmaps, *structural* layout tree (Split/Frame/Sidebar), commands, keymap, render-services TLS | same minus TLS; structural tree becomes a closed enum it owns natively |
| `devix-plugin` | plugin runtime, `MsgSink` callback into the loop | same; emits closures, not named pulses |
| `devix-app` | `Application` + `Service`/`Pulse`/`Effect`/`AppContext` + binary glue + clipboard | `Application` + closure-pulse + `AppContext` + binary glue. `Service` and named-Pulse types go away. |

## The four root causes (one per smell cluster)

### R1. `Pane` is the wrong abstraction for the structural layout tree.

`devix-panes::Pane` is *deliberately* framework-neutral — it knows nothing about editors, documents, themes, focus. That's correct for chrome widgets (palette popup, sidebar border) and plugin-contributed content. But `LayoutFrame` and `LayoutSidebar` are *editor-specific* by nature — to paint, they need `documents`, `cursors`, `theme`, `focused_leaf`, `render_cache`. We tried to force them through the framework-neutral trait anyway; the TLS render-services smuggle (`crates/editor/src/services.rs`) and the `borrowed_pane` Forwarder (`crates/editor/src/tree.rs:295-314`) are the only way to do that. Both are symptoms of the mismatch.

The fix is to stop pretending. Editor-internal layout is its own type — a closed `LayoutNode` enum that the editor crate owns natively, walks natively, and renders by directly composing the ratatui frame plus `panes::Pane`-typed leaves at the *content boundary*: modal palette, plugin sidebar contents, chrome widgets. `Pane` stays exactly the right abstraction for those. Splits/frames/sidebars stop being `Box<dyn Pane>` and become enum variants.

This kills smells #1 (TLS), #2 (Forwarder), #7 (two context mechanisms), and most of #3 (downcasts).

### R2. The structural layout types are a closed set.

Splits, frames, sidebars are the layout vocabulary the editor knows. New variants come from us, not from third parties — there's no plugin marketplace where a plugin contributes a "TripleVerticalSplit" layout primitive. Plugins extend through *content* (a pane that goes inside a sidebar, a modal pane, an editor command, a keybinding) — never through layout shape. So the layout vocabulary is closed. A closed vocabulary is an `enum`, not `Box<dyn Trait>`.

Moving to `enum LayoutNode { Split(...), Frame(...), Sidebar(...) }` removes every `as_any().downcast_ref::<LayoutSplit>()` in `tree.rs`. Walks become exhaustive matches. The compiler tells you when a new variant breaks a walk. Closes smell #3.

### R3. There's exactly one cross-thread message kind: "do this on the main thread".

The `Pulse` trait was sold as "every new subsystem-to-runtime message is a struct + impl Pulse". In practice that's not what it is — pulses live in `crates/app/src/pulse.rs` (not in their producers' crates) and reach into `cmd::OpenPath`, `frame_ids`, `focus_frame`, etc. The "extension point" is purely cosmetic; a god-enum and a god-set-of-structs are the same shape with different syntax.

The actual message kind is universal: `Box<dyn FnOnce(&mut AppContext) + Send>`. That's it. A producer that wants to do something on the main thread sends a closure. Disk-changed → closure that runs the three-way handler. Plugin emitted → closure that matches on `PluginMsg`. Mouse-wheel coalescing → closure that calls `cmd::ScrollBy`. Producers stay in their own crates and import only `EventSink`. `pulse.rs` deletes; the `Pulse` trait deletes.

For tracing: the send call optionally takes a `&'static str` name (cheap, not part of the closure type). Nobody needs typed pulse structs to grep stack traces.

This closes smell #6 (pulses-as-god-enum) and partially #4 (one less reason for app to know about producers).

### R4. `Service` is weakly motivated — too generic for what we actually have.

The trait was meant to unify "long-lived background subsystem". Three implementors were planned: `InputService`, `WatcherService`, `PluginService`. Today: `InputService` does real work (thread + atomic stop + bounded shutdown). `WatcherService` is deleted (notify owns its own thread). `PluginService::start` is a no-op (the worker was already spawned by `load_with_sink`). Two of three don't fit; the trait is carrying one implementor.

That's not an abstraction, that's a trait around a thread. Replace with two concrete things on `Application`:
- `input: InputThread` — the one thing that genuinely owns a poll thread; `Drop` handles shutdown.
- `plugin: Option<PluginRuntime>` — already self-managing; just hold it.

`Service`, `add_service`, `start_services`, `stop_services`, `services: Vec<Box<dyn Service>>` all delete. Closes smell #5.

The "future LSP / DAP service" objection: when LSP arrives, it'll either be (a) a tokio task owning a JSON-RPC client, in which case it's a struct on `Application` like `plugin`, or (b) something genuinely shaped like "thread that needs uniform start/stop". If (b) recurs across two unrelated subsystems, *then* introduce the trait — informed by two real implementors instead of one.

## Resulting primitive set

After applying R1–R4, the runtime exposes **three** primitives plus one supporting handle. Down from five plus one.

| Primitive | What it is | Extension point? |
|---|---|---|
| `Application` | the runtime. Direct fields for editor, registries, theme, clipboard, input thread, plugin, effects queue, sink, rx, terminal. | no — singleton |
| `AppContext<'a>` | unified `&mut` surface threaded through every delivery. | no — passed by `&mut` |
| `Effect` | closed enum: `Redraw`, `Quit`, `Run(closure)`. | no — internal |
| `EventSink` | cross-thread handle producers hold. Sends `LoopMessage::Input(ev)` or `LoopMessage::Pulse(Box<FnOnce(&mut AppContext) + Send>)` or `LoopMessage::Quit`. | no — handle |

Extension points reduce to two, both pre-existing:
- **`EditorCommand`** (in `devix-editor`): a new editor mutation.
- **`Pane`** (in `devix-panes`): a new visual leaf — modal, sidebar content, chrome widget. *Not* a new layout structure.

Layout structure extends through new variants on `LayoutNode` — done by us, not third parties.

## What the layering looks like after R1–R4

```
devix-text ────────────────────────────┐
devix-syntax ──────────────────────────┤
                                       ↓
devix-panes  Pane trait + chrome widgets + Event/Rect
            (editor-agnostic; nothing imports devix-editor)
                                       ↓
devix-editor LayoutNode enum (native walks, no downcasts)
             Editor + documents + cursors + commands + keymap
             EditorPane (Pane impl) for the buffer body — needs editor borrows
                                       ↓
devix-plugin PluginRuntime + LuaPane (Pane impl for sidebar content)
                                       ↓
devix-app    Application + AppContext + Effect + EventSink
             InputThread (concrete struct, Drop-based shutdown)
             binary glue (main.rs, clipboard)
```

`devix-panes` is below `devix-editor`. `LayoutNode` is the editor's private layout vocabulary; only `Pane`-typed *leaves* at the content boundary cross the layer line into `panes`. The editor's `render` walks its own enum and calls `Pane::render` on the contained widgets/modals/plugin contents — passing whatever borrows it has. No TLS, no Forwarder, no downcasts.

## Bootstrapping (smell #4 fully resolved)

Today: `Application::new` *or* `Application::with_channel`, plus `Editor::attach_disk_sink`, plus `PluginRuntime::load_with_sink`. Four wiring paths.

After R3 (closure pulses): the producers still need a sink before `Application` exists, because they're born outside it. That's intrinsic — the editor's notify watchers and the plugin worker spawn before the app does. Two clean shapes:

- **(a) builder.** `Application::builder()` returns `(EventSink, ApplicationBuilder)`. The caller wires the editor and plugin against the sink, then `builder.editor(e).plugin(p).build()`. One wiring path; no `with_channel` exception.
- **(b) inside-out.** `Application` owns the sink and exposes `app.editor_mut()` for setup; producers register via `app.editor_mut().attach_disk_sink(app.sink_clone())` after construction. Means `Application::new` accepts an empty editor, which is awkward.

Pick (a). It's a 30-line refactor of `main.rs` and removes the dual constructor.

## Migration order (do not parallelize)

Each step strictly removes complexity. Each compiles green. R1 is the biggest; do it first because R3 and R4 collapse trivially after the layering is right.

1. **R1+R2 together — `LayoutNode` enum.** Convert the structural tree from `Box<dyn Pane>` to a closed enum in `devix-editor`. Delete `RenderServices` (TLS, scope, with) — its data flows by argument now. Delete `borrowed_pane`, `pane_at_indices`, `pane_at_indices_mut`, `find_frame*`, `find_sidebar_mut`, `pane_leaf_id`, `sidebar_present`, `frame_ids` — replace with enum methods or exhaustive matches. `Pane` stays as the leaf-content trait for modals, plugin sidebar content, and chrome. Editor's `render` becomes a direct walk. Largest churn step; expected ~−400 lines net.
2. **R3 — closure pulses.** Replace the `Pulse` trait + named pulse structs with `Box<dyn FnOnce(&mut AppContext) + Send>`. Move each former pulse's body to its producer site (`Editor::attach_disk_sink`'s callback closes over the body; `PluginRuntime`'s `MsgSink` ditto; the mouse-wheel deferred work uses an inline closure). Delete `crates/app/src/pulse.rs`. Expected ~−100 lines.
3. **R4 — drop `Service`.** Replace `services: Vec<Box<dyn Service>>` with `input: InputThread` and `plugin: Option<PluginRuntime>` fields on `Application`. Delete `Service` trait, `add_service`, `start_services`, `stop_services`, the `services/` directory. Expected ~−150 lines.
4. **Bootstrapping — `Application::builder`.** Single wiring path. `Application::new` and `with_channel` collapse. Expected ~−30 lines, plus simpler `main.rs`.

After step 4: `crates/app/src/` is `application.rs` + `context.rs` + `effect.rs` + `event_sink.rs` + `events.rs` + `clipboard.rs` + `lib.rs` + `main.rs` + `render.rs` (+ `input.rs`, the concrete input thread). Down from today's 12 files. Editor's `tree.rs` halves.

## Non-goals

- **No new abstractions.** The point of this note is *fewer* primitives, not different ones. We're removing `Service`, `Pulse`, `RenderServices`, the pane downcast vocabulary.
- **No re-litigation of crates layout.** Six crates is the right count; each owns a distinct concern.
- **No async / tokio at the runtime level.** That's RUNTIME-SPEC's call and it stands; closure pulses are sync and main-thread-only by construction.
- **No third-party plugin host hardening.** Three-strikes, separate process, signed manifests — out of scope until we ship a marketplace.
