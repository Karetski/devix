# devix — Architecture

## Principle

A small editor is a few thousand lines of code; a scalable editor is a few well-chosen abstractions. The goal is to design the latter.

Two reference points: Lattner's MLIR (dialects as composable extensions of one `Op` concept) and UIKit (everything is a `UIView`). Both work because **two or three core abstractions** carry the entire system; new features extend existing concepts instead of adding new top-level ones. A custom MLIR dialect is just an `Op` impl. A custom UIKit control is just a `UIView` subclass. The architecture's job is to make sure the same is true here: a custom file tree, a settings page, a terminal, a plugin panel — none of these should need a new top-level concept. They are all instances of the same abstraction.

## The four concepts

The whole editor is built from these. Anything that can't be expressed as an instance of one of them is a sign the design is wrong.

### 1. `Pane` — the universal display unit

Like `UIView`. A rect-bounded thing that can render itself, handle events, and own children.

```rust
pub trait Pane {
    fn render(&self, area: Rect, ctx: &mut RenderCtx);
    fn handle(&mut self, ev: &Event, ctx: &mut HandleCtx) -> Outcome;
    fn layout(&self, area: Rect) -> Vec<(PaneId, Rect)> { vec![] }
}
```

`render` takes `&self` — the compiler enforces "render is pure." Mutation only happens in `handle`. The current PLAN.md rule becomes a type signature, not a comment.

Concrete Panes (today's features, re-expressed):

- `EditorPane` — buffer view; selection, scroll, cursor live here.
- `SplitPane { axis, children, ratios }` — recursive layout. Replaces the `Node::Split` enum variant.
- `TabbedPane` — tab strip + active child. Replaces `Frame`.
- `SidebarPane` — pinned to an edge.
- `PalettePane`, `SymbolPickerPane` — modal overlays.
- `HoverPane`, `CompletionPane` — anchored popups.
- Future: `FileTreePane`, `TerminalPane`, `SettingsPane`, plugin Panes.

There is no `Node::Split | Pane | Panel` enum. There is just `Box<dyn Pane>`. A sidebar is a Pane positioned at an edge by its parent. An overlay is a Pane with a higher z-index. A popup is a Pane with a screen anchor. Composition is the framework's job; what makes a feature is just its Pane impl.

### 2. `Action` — invocable behavior, first-class

The current `Action` is a 50-variant enum the dispatcher matches on. Lattner's principle: make each behavior a type, not a tag.

```rust
pub trait Action: 'static {
    fn invoke(&self, ctx: &mut ActionCtx);
}
```

Actions are values. Keymaps map chords to `Box<dyn Action>`. Palette commands are actions with a label. Plugins contribute actions by implementing the trait. Adding a new action does not grow a central enum — it adds a struct.

### 3. `Document` — text data, decoupled from any view

Text + path + per-doc services (file watcher, tree-sitter highlighter, LSP wire). The key invariant: a Document has no notion of "the view onto it." Multiple `EditorPane`s can share one Document (split view, same file). View state (selection, scroll, hover, completion) lives in the Pane, not the Document.

### 4. `Surface` — the editor itself

The root: a Pane tree + a document registry + the LSP coordinator handle + the focus chain. Replaces `Workspace`. The name says what it is — the surface on which the editor renders. It does not replace `UIView`'s role as a container; it is the root container.

That is the entire design. Everything else is an instance.

## Composition

### Editor view in a tab in a split

```
SplitPane { axis: Horizontal, children: [
    TabbedPane { tabs: [EditorPane(doc1), EditorPane(doc2)], active: 0 },
    TabbedPane { tabs: [EditorPane(doc3)], active: 0 },
]}
```

### With sidebars

```
SplitPane { axis: Horizontal, children: [
    SidebarPane(slot: Left, content: FileTreePane),
    SplitPane { ... editor stuff ... },
    SidebarPane(slot: Right, content: nil),
]}
```

### Palette open over the editor

The root tree paints first. The responder chain head holds a `PalettePane`. It paints last (z-order) and is first in the event chain (modal). When dismissed, it is removed from the chain. There is no `Surface::overlay: Option<Overlay>` field — modals are Panes.

### Hover popup on cursor

When LSP hover lands, `EditorPane` adds a `HoverPane` as a child anchored at the cursor's screen position. When the cursor moves, the HoverPane is removed. State that today lives in `View::hover` lives inside the HoverPane.

### Completion popup

Same pattern as HoverPane. State that today lives in `View::completion` lives inside `CompletionPane`. The CompletionPane handles its own arrow-key navigation in `handle()` before bubbling to EditorPane.

### Plugin panel

A plugin returns a `Box<dyn Pane>`. The host inserts it into a sidebar. There is no special "plugin panel" type.

### Settings UI, file tree, terminal

Each is a Pane. No new architectural concept.

## Composability — the UIScrollView lesson

`UICollectionView` extends `UIScrollView`; `UITextView` extends `UIScrollView`; every scrollable thing in iOS uses one implementation. The same lesson applies here.

```rust
pub struct ScrollPane<P: Pane> {
    inner: P,
    scroll: (u32, u32),
    content_size: (u32, u32),
}
impl<P: Pane> Pane for ScrollPane<P> { ... }
```

`EditorPane` does not call `ensure_visible` itself; it wraps in `ScrollPane` and the framework handles wheel events, clamping, and viewport math. The tab strip wraps its content in a horizontal `ScrollPane`. The palette result list wraps its rows in a vertical `ScrollPane`. **One** scroll implementation, used by every scrollable thing.

The same composability applies to overlays:

```rust
pub struct OverlayPane<P: Pane> { inner: P, anchor: Anchor, z: u8 }
```

A popup, a tooltip, a dropdown — all `OverlayPane<...>`. Write the behavior once; compose it everywhere.

## Crate decomposition

Five framework crates plus the binary, six total. The earlier vision
was eight; the realized layout drops the cycle-breaking carve-outs
(`view`, `workspace`, `commands`, `surface`) that existed only to
satisfy the borrow checker, and collapses the framework's
trait-vs-impl split — `core` and `ui` — into a single `panes` crate
where the boundary lives at the module level instead of the crate
level (UIKit-style).

| Crate | Role | Deps |
|---|---|---|
| `text` | `Buffer`, `Selection`, `Transaction`. Leaf. | `ropey` |
| `syntax` | tree-sitter wrapper. Leaf. | `tree-sitter` |
| `panes` | The framework. `Pane`/`Action`/`Event`/`Outcome` traits, `RenderCtx`/`HandleCtx`, geometry primitives (`Rect`, `Anchor`), layout primitives (`Axis`, `Direction`, `SidebarSlot`, `split_rects`), walk helpers, `Theme`, layout composites (`TabbedPane`, `SidebarSlotPane`), chrome widgets (popup, palette, sidebar, tabstrip). | `ratatui`, `crossterm` |
| `editor` | The editor. `Document` (rope buffer + tree-sitter highlighter + filesystem watcher), `Cursor` (per-tab edit state), `EditorPane` (buffer rendering), `Editor` (root: layout tree, focus chain, modal slot, `Document`/`Cursor` SlotMaps), `LayoutFrame`/`LayoutSplit`/`LayoutSidebar`, commands, keymap, palette logic. | `text`, `syntax`, `panes` |
| `plugin` | Lua plugin host. | `panes`, `editor` |
| `devix` (bin) | Terminal lifecycle, tokio runtime, root `Editor` construction, event loop. | `text`, `panes`, `editor`, `plugin` |

Plugins depend on `panes` (for the trait surface and layout primitives) plus `editor` (the host needs `Editor` to register actions and panes against). The architecture's older "plugins depend only on core" framing was an aspiration — the impls plugins need to compose against (`OverlayPane`, `TabbedPane`, the chrome widgets) live in the same crate as the traits, by design.

The four-concept design maps 1:1 to four type names with no synonyms:

| Concept | Type | Crate |
|---|---|---|
| Universal display unit | `Pane` | `panes` |
| Invocable behavior | `Action` | `panes` |
| Text model | `Document` | `editor` |
| Editor root | `Editor` | `editor` |

## Migration path

This is not a rename pass. It is an architectural rewrite. It can ship incrementally — every phase compiles, tests pass, the editor works end-to-end.

**Phase 0 — Define `core`.** New crate with trait definitions and primitive types. No impls yet. Nothing breaks.

**Phase 1 — `Pane` trait + adapter.** Add an adapter that wraps current `render_*` functions as `Pane` impls. The existing dispatcher still drives the old code; the new `Pane` trait coexists. Validates the abstraction without committing to it.

**Phase 2 — Migrate one feature: `EditorPane`.** Move editor render + scroll + cursor handling out of free functions and into `EditorPane: Pane`. Hover and completion become child Panes. The dispatcher learns to walk Pane children for events.

**Phase 3 — Migrate the layout tree.** Replace `Node::Split | Pane | Panel` with `SplitPane`, `TabbedPane`, `SidebarPane`. The `Workspace` god-object shrinks because most state moved into its Panes.

**Phase 4 — Migrate overlays to modal Panes.** `Overlay::Palette` and `Overlay::Symbols` become `PalettePane` and `SymbolPickerPane` in the responder chain head. The `Overlay` enum disappears.

**Phase 5 — Migrate `Action` to a trait.** Convert the enum's variants into structs. Keymap holds `Box<dyn Action>`. Palette stores them too. The dispatcher becomes ~10 lines: walk the responder chain, hand the event to the first Pane that claims it.

**Phase 6 — Rename and re-organize.** `Workspace` → `Surface`, `devix-buffer` → `devix-text`, etc. By now naming is the easy part.

**Phase 7 — Drop dead crates.** `views`, `ui` (merged into `core` and `editor`), `config`, `document`, `plugin` (until needed).

Each phase is small, reviewable, and ends green.

## What this gives us

- **One way to do anything visual.** Implement `Pane`. Today there are seven render functions, four overlay variants, three view-state enums.
- **Compiler-enforced render purity** via `&self` instead of a comment in `PLAN.md`.
- **Real plugin path.** Plugins implement `Pane` / `Action` against `core`. The editor does not grow special-cased plugin support — plugins are just more Panes.
- **Real settings / file-tree / terminal path.** Each is a Pane. No new architectural concept needed for any of them.
- **Composable scroll, overlay, etc.** `ScrollPane<P>`, `OverlayPane<P>`. Write once, use everywhere.
- **Smaller surface to learn.** Four traits and the type tree they form. Today there are dozens of structs to track.
- **Progressive disclosure.** Hello-world is one Pane. Advanced usage composes them. Plugin authors learn `core` and nothing else.

## What this costs

- **Substantial rewrite**, not a refactor. The current architecture mostly works; this is investment in long-run scale.
- **Trait objects** (`Box<dyn Pane>`, `Box<dyn Action>`) introduce indirection. Acceptable for an editor; UIKit ships fine with the equivalent.
- **Possible over-genericization.** The `Pane` trait may force features into shapes that do not fit (e.g., the editor's selection model is rich; do we want it accessible from a generic `Pane`?). Escape hatches are needed: downcasting, or feature-specific traits like `TextPane: Pane` for things that own text.
- **API churn for callers.** Currently working code in `app/` is rewritten substantially. `CLAUDE.md` and `PLAN.md` get updated.

## Non-goals

- **No DSL for layout.** Pane trees are just Rust values. SwiftUI-style declarative layout is out of scope; if it earns its keep later, build it on top.
- **No reactive runtime.** Render reads state; events mutate it; the loop redraws. No observable, no diffing layer, no virtual DOM. This is a TUI, not a SPA framework.
- **No plugin host in `core`.** `core` defines the trait surface; the plugin host lives in `editor` (or a future `plugin-host` crate) and is the editor's concern. Plugins do not depend on a host.

## Deferred items

Things explicitly pushed to a later phase than the migration path above suggests, with the reason and the new home. Update as work progresses.

- **Dispatcher walks Pane children for events.** Originally part of Phase 2 ("the dispatcher learns to walk Pane children for events"). Pushed to **Phase 5** because rewriting dispatch is most coherent when `Action` becomes a trait at the same time — doing it twice (once for Pane routing, once for Action dispatch) doubles the dispatcher rewrite. Phase 2's `EditorPane::handle` returns `Outcome::Ignored`; the legacy `dispatch::dispatch` still routes click/drag/scroll.
- **Pane-owned view state.** Originally implied by Phase 2 ("most state moved into its Panes"). Pushed to **Phase 3** because `EditorPane` borrowing from `Workspace.views` keeps Phase 2 a render-only move. The god-object shrink that lets `EditorPane` own its `View` outright is Phase 3's actual deliverable.
- **Scroll math (`layout_pass` in `app/render.rs`) inside the Pane.** Pre-paint scroll-into-view + clamp still runs in the binary's render module, not on the Pane. Pushed to **Phase 3** alongside the layout-tree migration — the scroll pass walks the leaf list, which is exactly what `SplitPane`/`TabbedPane` will own.
- **`Theme` in `core`.** ~~Deferred from Phase 0.~~ **Done** (Phase 6 partial). Lives at `crates/core/src/theme.rs`; `devix-config` was the only host and got dropped along with it (Phase 7 partial). `devix-plugin` (1-line "Phase 8" stub) also dropped — zero dependents.
- **Phase 3 sub-phases.** Resolved Q1/Q2/Q3 from `PHASE-3-DESIGN.md` in favor of the Lattner-shaped answer: no role enum, no `LayoutSink`, generic `Pane::children(area)` walker. Status:
  - **3a (trait-surface refactor) — done.** `Pane::children(area)` and `is_focusable()` added; `PaneId` and the old `layout()` removed. Generic walkers (`pane_at`, `focusable_at`, `pane_at_path`, `focusable_leaves`) live in `devix-core::walk`. Tab-strip hits, frame-body rects, and sidebar rects all become `pane_at` queries against a live tree once 3b/3c land. No closed enum of "kinds" anywhere — the framework only knows about `&dyn Pane`.
  - **3b (composite Panes alongside legacy) — done.** `crates/views/src/layout.rs` defines `SplitPane`, `TabbedPane`, `SidebarSlotPane` as concrete `Pane` impls. `EditorPane.highlights`, `TabStripPane.tabs`, and `SidebarPane.title` flipped from borrowed to owned to break self-referential composite layouts. `app/src/render.rs::paint` is now two passes: `populate_cache` (read-only layout math; writes `RenderCache`) and `paint_leaves` (builds a `TabbedPane`/`SidebarSlotPane` per leaf and renders). Tab-strip hit extraction landed as `devix_ui::tab_strip_layout`. `SplitPane` is defined and unit-tested but not yet driving the live render path — that lands in 3c when `Node` is replaced.
  - **3c (replace `Node` with the Pane tree) — done, including state ownership.** `Node` enum deleted. `Workspace.layout` field removed. `sync_root` removed. `Workspace.root: Box<dyn Pane>` is the sole layout source of truth. `Frame` struct deleted. `Workspace.frames: SlotMap<FrameId, Frame>` deleted. **`LayoutFrame` owns its `tabs`/`active_tab`/`tab_strip_scroll`/`recenter_active` directly** — each frame in the layout tree is its own self-contained Pane, no indirection. `FrameId` survives as a stable cache key (`render_cache.frame_rects`/`tab_strips`); minted via the monotonic counter in `frame.rs`. Lookups by `FrameId` go through `tree::find_frame` / `find_frame_mut` (tree walks; O(tree-size), fine at TUI scales). All ops (`split_active`/`close_active_frame`/`toggle_sidebar`) mutate `root` in place via `tree::mutate` helpers. All read-side queries walk `root` via `core::walk` + `as_any` downcasts. The framework-side endgame Lattner described — *one open primitive, state on the Pane, no closed enum at the layout root* — is fully delivered.
- **Phase 4 (modal Panes in responder chain) — done.** The `Overlay` enum is deleted; `Surface.modal: Option<Box<dyn Pane>>` is the head-of-tree responder slot. `PalettePane` and `SymbolPickerPane` live in `crates/surface/src/modal.rs` (Phase 7's flat editor crate would be the natural long-term home, but they share the modal slot's data) and own their state outright. Input drives through `Pane::handle`; close / accept / LSP-refetch surface as `ModalOutcome` flags the host drains via `as_any_mut().downcast_mut::<T>()`. Render goes through host downcast in `app/render.rs` (palette needs the `CommandRegistry` + `Keymap` for chord hints; that's not in `core::RenderCtx`'s remit). Plugin-contributed modals can rely on the fall-through `Pane::render` for self-contained paint.
- **Phase 5 (Action as trait) — done.** `Action` enum deleted; dispatcher enum match deleted. All 50+ commands are struct impls in `crates/surface/src/cmd.rs`. `Keymap` and `CommandRegistry` store `Arc<dyn EditorCommand>` (HRTB-aliased to `for<'a> core::Action<Context<'a>>`). Live input flow: `KeyEvent → Chord → keymap.lookup → action.invoke(&mut Context)`. The Lattner endgame — "each behavior is a type, not a tag" — is delivered: adding a new action is a struct + `impl Action<Context<'a>>`, no central enum to grow. Plugins contribute commands the same way, against the same trait. `crates/surface/src/dispatch.rs` is now just shared helpers (motion math, clipboard, completion ops, LSP-position request) used by the struct impls.
- **Phase 6 (renames) — done.** `Workspace` → `Surface` (struct, crate dir `crates/workspace/` → `crates/surface/`, package name `devix-workspace` → `devix-surface`, every `&Surface`/`&mut Surface` parameter, every `app.surface.foo` accessor). `devix-buffer` → `devix-text` (`Buffer`/`Selection`/`Transaction` lived there; `text` describes the role better than `buffer`). LSP-side `WorkspaceSymbols`/`WorkspaceFolder` and Cargo's `workspace = true` inheritance are unrelated terms and stay untouched.
- **Phase 7 (final crate consolidation) — done.** `crates/views` and `crates/document` collapsed into a single `crates/editor` crate; the prior `devix-document` crate is gone. The dep edge is one-directional: `surface -> editor` (Surface holds `SlotMap<DocId, Document>` and `SlotMap<ViewId, View>`; both `Document` and per-view popup state — `HoverState`, `CompletionState` — live in editor). Editor depends only on `text`, `syntax`, `lsp`, `core`, `ui`. To keep the dep graph acyclic the popup-state types moved out of `surface/view.rs` into `editor/src/popup_state.rs`; `View` itself stays in `surface` because Surface's slotmap owns it. `crates/ui` (chrome widgets — popup, status, tab strip, sidebar) stays as its own crate per the architecture's "stable plugin surface" framing of `core` (ui has no surface awareness; merging it into core would drag ratatui-widget specifics into the framework's trait surface). Final crate inventory: `app`, `text`, `core`, `editor`, `lsp`, `surface`, `syntax`, `ui` — eight crates, plus the binary.
- **Vocabulary reorg (post-Phase 7) — done.** A follow-up reorg pass collapsed the cycle-breaking carve-outs and the residual conceptual collisions; the Phase 7 inventory is superseded. (1) `devix-workspace` folded into `devix-editor` (it was a single-file crate for `Document`; the indirection wasn't paying rent). (2) `devix-surface` folded into `devix-editor`, with the root struct renamed `Surface` → `Editor` (`devix_editor::Editor`, the standard Rust `crate::Type` pattern). The buffer-pane file was renamed `editor.rs` → `buffer.rs` to free `editor.rs` for the root struct; the render-helper struct `EditorView` → `BufferRender` to drop the `view` overload. (3) The per-tab edit-state struct `View` → `Cursor` (it's a cursor against a document, with scroll-tracking; `Selection` already represents multi-cursor). (4) `devix-core` and `devix-ui` collapsed into a single `devix-panes` crate. The trait-vs-impl distinction lives at the *module* level inside `panes`, not the crate level — UIKit ships `UIView` and the chrome widgets in one framework, not two. The aspirational "plugins depend only on `core`" boundary didn't survive contact with reality (any non-trivial plugin needs `OverlayPane`, composites, chrome — they pull in the impl side too); a publishable plugin SDK can be a re-export crate created at that moment if it ever matters. (5) `TabbedPane` made generic over its body (`TabbedPane<B: Pane>`) so `panes` doesn't need to know about `EditorPane`. Final crate inventory: `text`, `syntax`, `panes`, `editor`, `plugin`, `app` — six crates, plus the binary. The four-concept vocabulary now maps 1:1 to four type names with no synonyms (`Pane`, `Action`, `Document`, `Editor`).
