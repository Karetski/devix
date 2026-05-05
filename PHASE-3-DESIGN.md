# Phase 3 — Layout-tree migration: design proposal

**Status:** draft, awaiting decisions on the three open questions in §3.

The goal of Phase 3 is to replace the `Node::Split | Frame | Sidebar` enum
in `devix-workspace::layout` with three concrete `Pane` impls
(`SplitPane`, `TabbedPane`, `SidebarPane`) that own their state. After
Phase 3, the layout tree *is* the Pane tree; there's no parallel data
structure.

Phases 0–2 validated that the `Pane` trait surface fits leaf renders
(`StatusPane`, `EditorPane`, `HoverPane`, `CompletionPane`). Phase 3 is
where the trait has to support **composition** (parents that own
children) and **state ownership** (per-pane data that today lives on
`Workspace.frames` / `Workspace.layout`).

## 1. What's in scope

- Replace `Node` enum with `Box<dyn Pane>` as the layout source-of-truth.
- Move per-frame state (`tabs: Vec<ViewId>`, `active_tab`, `tab_strip_scroll`,
  `recenter_active`) onto `TabbedPane`.
- Move per-sidebar state (`SidebarSlot`, focused flag, future content) onto
  `SidebarPane`.
- Recursive composition via `SplitPane { axis, children: Vec<(Box<dyn Pane>, weight)> }`.
- Migrate `focus`, `hittest`, and `ops` modules to walk the Pane tree.
- Migrate the pre-paint `layout_pass` (scroll-into-view + clamp) and the
  `render_cache` writes into the framework's layout pass — paint becomes
  truly pure.

## 2. What's out of scope (still deferred)

- **Pane-owned `View` state.** `EditorPane` keeps borrowing from
  `Workspace.views` for now. Moving views onto `EditorPane` is a separate
  refactor that touches the dispatcher and is best done in Phase 5.
- **Dispatcher walks Pane children for events.** Still Phase 5 — needs
  Action-as-trait first so the dispatcher rewrite happens once.
- **`Theme` to `core`.** Phase 6.

## 3. Three open questions (need decisions before writing code)

### Q1. How does the framework move state out of `&self` paint?

`Pane::render` is `&self`. Today's paint writes the render cache
(`frame_rects`, `sidebar_rects`, `tab_strips`). Two options:

**Option A: Two-pass (`layout` then `render`).** The framework walks the
Pane tree's `layout()` first; that pass computes rects, hits, and any
other layout-derived data, writing into a side-channel. Then `render()`
runs purely.

```rust
pub trait Pane {
    fn layout(&self, area: Rect, out: &mut LayoutSink);
    fn render(&self, area: Rect, ctx: &mut RenderCtx<'_, '_>);
    fn handle(&mut self, ev: &Event, ctx: &mut HandleCtx<'_>) -> Outcome;
}
```

`LayoutSink` carries the cache references the parent supplies. Composite
Panes recursively call children's `layout` with their assigned rects.

Pros: render stays pure; the layout pass is exactly where focus traversal
and hit-test caches *should* be populated. Matches UIKit's
`viewWillLayoutSubviews` / `draw` separation, which is already cited in
`app/src/render.rs`.

Cons: every Pane impl now has two methods that each need a sensible
default. Most leaf Panes would have a one-line `layout()` ("I occupy this
whole rect, no children"); the boilerplate is real.

**Option B: Mutable cache on `RenderCtx`.** Keep `render` as the only pass,
add `cache: &mut RenderCache` (or a more abstract `LayoutSink`) to
`RenderCtx`. Panes mutate the cache during paint.

Pros: one pass; no method-count growth.

Cons: violates "render is pure" — the rule that's literally encoded in
`Pane::render(&self)`. We'd be admitting the rule is aspirational. Also,
`RenderCache` lives in `devix-workspace`, so putting it on `RenderCtx`
forces `devix-core` to depend on workspace types or to define a generic
sink trait.

**Recommendation: A.** The two-pass design is what the trait already
hints at (Phase 0's `layout(&self, area: Rect) -> Vec<(PaneId, Rect)>`).
We refine the signature to take a sink rather than return a `Vec`, but
the spirit is unchanged. The boilerplate in leaf Panes is real but tiny
(one default method on the trait covers it).

### Q2. How do parents identify children's roles?

Today's `paint()` does:

```rust
match leaf {
    LeafRef::Frame(id)    => paint_frame(*id, *rect, ...),
    LeafRef::Sidebar(slot) => paint_sidebar(*slot, *rect, ...),
}
```

The cache writes (`frame_rects.insert(id, rect)`, `sidebar_rects.insert(slot, rect)`)
require knowing which leaf is which. With a homogeneous `Box<dyn Pane>`
tree, that information is gone.

**Option A: Each Pane writes its own cache slot.** `TabbedPane::layout`
writes to `LayoutSink::frame_rects(self.id, area)`; `SidebarPane::layout`
writes `sidebar_rects(self.slot, area)`. The framework just walks; each
Pane knows its own role.

Pros: single dispatch point per role; no centralized match-on-type. Adding
a new Pane kind (file tree, terminal) doesn't need framework changes.

Cons: `LayoutSink` ends up with workspace-specific methods
(`frame_rects`, `sidebar_rects`, `tab_strips`). That couples `core` to
workspace concepts, violating the "core is plugin-stable" goal.

**Option B: `PaneId` carries kind information.** Make `PaneId` an enum:

```rust
pub enum PaneId {
    Editor(u64),
    TabStrip(u64),
    Sidebar(SidebarSlot),  // or u64
    Custom(TypeId, u64),   // for plugins
}
```

`layout()` returns `(PaneId, Rect)` per child; the framework dispatches
cache writes off the `PaneId` discriminant.

Pros: `core` stays workspace-agnostic; cache writes are framework
business, not Pane business.

Cons: `PaneId` becomes a closed-ish enum, which fights the open-extension
principle of the architecture (anything can be a Pane). The `Custom`
variant helps but is awkward.

**Option C: Type-erased "ports".** Drop the centralized cache entirely.
Each consumer (focus traversal, hit-test, dispatch) walks the live Pane
tree and uses downcasts to find the panes it cares about.

```rust
pub trait Pane: Any { ... }
fn find_pane<T: Pane>(root: &dyn Pane) -> Option<&T> { ... }
```

Pros: zero coupling; matches the architecture's "downcasting as escape
hatch" note (line 178 of ARCHITECTURE-REFACTOR.md).

Cons: every focus/hit-test/dispatch path now does a tree walk. Today's
`O(1)` cache lookups become `O(tree-size)`. For tree sizes <20, fine; for
plugin-heavy futures, possibly hot.

**Recommendation: A for now, with the LayoutSink trait abstracted in
`core` and the workspace-flavored impl living in `devix-workspace`.** The
sink trait stays open (anyone can define one), and the workspace's impl
knows about frames / sidebars / tab strips. Plugins that need their own
cache provide their own sink. C is the "right" long-term answer but the
overhead doesn't matter at today's tree sizes; revisit if profiling
shows the cache walks dominate.

### Q3. Tab-strip hits — where does layout end and paint begin?

`render_tabstrip` returns `TabStripRender { hits, content_width }` as a
side-effect of painting. To populate `tab_strips` in the layout pass, hit
generation has to be factor-able from rendering.

The good news: it already mostly is. `layout_tabstrip` exists and runs
the scroll math. The remaining painted-only logic in `render_tabstrip` is
the visible hits — every tab whose `geom.screen` was returned by
`CollectionPass::visible_items()`.

Plan:

1. Add `layout_tabstrip_hits(tabs, active, scroll, area) -> Vec<TabHit>`
   that runs `CollectionPass::new(...).visible_items()` and collects rects
   without drawing.
2. `TabbedPane::layout` calls it and writes hits into `LayoutSink`.
3. `render_tabstrip` keeps returning `TabStripRender` for backward-compat
   during the migration; once `TabbedPane::render` is the only caller, the
   return type can drop the hits field.

No new tension here — just a one-function extraction.

## 4. Proposed Pane shapes

```rust
// devix-views or devix-editor (post-Phase-6 rename)

pub struct SplitPane {
    pub axis: Axis,
    pub children: Vec<(Box<dyn Pane>, u16)>, // weight per child
}

pub struct TabbedPane {
    pub tabs: Vec<ViewId>,         // moved off Workspace.frames
    pub active_tab: usize,
    pub tab_strip_scroll: (u32, u32),
    pub recenter_active: bool,
    // The active tab's editor lives here too once `View` ownership migrates
    // (Phase 5). For Phase 3, it's still indexed via ViewId into Workspace.
}

pub struct SidebarPane {
    pub slot: SidebarSlot,
    pub focused: bool,
    pub content: Option<Box<dyn Pane>>, // future: plugin-contributed
}
```

`Workspace` keeps `documents` and `views` slot-maps; `frames` slot-map
disappears (state moved onto `TabbedPane`). The layout root becomes
`pub root: Box<dyn Pane>` rather than `pub layout: Node`.

## 5. Migration sub-phases

To keep the diff reviewable, Phase 3 splits into three:

- **3a. Trait surface refactor.** Refine `Pane::layout` to take a sink,
  add `LayoutSink` trait to `core`, define a `WorkspaceLayoutSink` in
  `devix-workspace` that wraps `RenderCache`. Update Phase-1/2 Panes to
  carry the new (default-impl) `layout` method. No behavior change yet.
  **End state:** workspace builds, tests green, cache is still populated
  by the legacy paint-time writes.

- **3b. Composite Panes alongside the legacy tree.** Define `SplitPane`,
  `TabbedPane`, `SidebarPane`. Build a Pane tree from the legacy `Node`
  tree on each render and drive paint through the Pane tree. Cache writes
  switch to the new layout pass. Legacy `Node` is now dead but still
  exists. **End state:** two parallel structures; Pane tree drives paint
  and cache; `Node` only used by `ops`/`focus`/`hittest`.

- **3c. Replace `Node` with the Pane tree.** Migrate `ops` (split_active,
  close_active_frame, toggle_sidebar, new_tab, etc.), `focus` (focus_dir,
  walk_into), and `hittest` (focus_at_screen, etc.) to walk
  `Box<dyn Pane>` instead of `Node`. Delete `Node`, `LeafRef`, the old
  `layout_tabstrip`/cache wiring. **End state:** Pane tree is the only
  layout structure.

3a is the trait-surface change that needs the decisions above; 3b is
mostly straightforward; 3c is the largest diff but most mechanical (it's
a code-walking exercise once the Pane tree is the source of truth).

## 6. Things to confirm before starting 3a

- [ ] Decision on **Q1** (layout pass): A vs. B.
- [ ] Decision on **Q2** (parent-child role identification): A, B, or C.
- [ ] Decision on **Q3**: confirm the `layout_tabstrip_hits` extraction is
      uncontroversial. (It probably is.)
- [ ] Sub-phase commit cadence: one PR per sub-phase, or three sub-phase
      branches merged together?

Once these are settled, 3a is a 1–2 hour change; 3b is a half-day; 3c is
a day or two depending on how many `Node`-walking sites turn up in
`dispatch.rs`.
