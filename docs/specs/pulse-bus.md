# devix — Pulse bus spec

Status: working draft. Stage-0 foundation T-01.

## Purpose

Closed-enum versioned message bus for in-thread and cross-thread event
publication. Replaces the current closure-as-message mechanism
(`EventSink::pulse`) plus the various ad-hoc push callbacks (`DiskSink`,
`MsgSink`, `Wakeup`) with one typed surface every component publishes to and
subscribes from.

This spec answers Smalltalk's principle: *messaging as the kernel; default to
late binding.* Pulses are named, typed, and serializable; subscribers bind by
filter, not by direct method call.

## Scope

This spec covers:
- The `Pulse` closed enum and the v0 catalog.
- The `PulseBus` API (publish, subscribe, unsubscribe).
- `PulseFilter` matching shape.
- Delivery semantics in-thread and cross-thread.
- Versioning rules.

This spec does **not** cover:
- View IR rendering (lives in `frontend.md`). Pulses signal *that* something
  changed; the View IR carries *what* to draw.
- The wire transport (lives in `protocol.md`). Today the bus is in-process;
  a future transport carries the same `Pulse` enum serialized.
- Concrete subscription site code (each consumer registers in its own crate).

## Catalog (v0)

The v0 catalog. Variants are *additive between minor versions*; renames or
removals require a major bump. Every payload is `Path`-keyed (per
`namespace.md`); no raw `DocId`/`CursorId`/`FrameId` shows up here.

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Pulse {
    // ---- Buffer / document ----
    BufferOpened     { path: Path, fs_path: Option<PathBuf> },
    BufferChanged    { path: Path, revision: u64 },
    BufferSaved      { path: Path, fs_path: PathBuf },
    BufferReloaded   { path: Path },
    BufferClosed     { path: Path },
    DiskChanged      { path: Path, fs_path: PathBuf },

    // ---- Cursor / selection ----
    CursorMoved      { cursor: Path, doc: Path, head: u64 },
    SelectionChanged { cursor: Path, doc: Path },

    // ---- Layout / focus ----
    TabOpened        { frame: Path, doc: Path },
    TabActivated     { frame: Path, doc: Path },
    TabClosed        { frame: Path, doc: Path },
    FrameSplit       { source: Path, new: Path, axis: Axis },
    FrameClosed      { frame: Path },
    SidebarToggled   { slot: Path, open: bool },
    FocusChanged     { from: Option<Path>, to: Option<Path> },

    // ---- Modal ----
    ModalOpened      { modal: ModalKind, frame: Option<Path> },
    ModalDismissed   { modal: ModalKind },

    // ---- Commands ----
    CommandInvoked   { command: Path, source: InvocationSource },

    // ---- Plugin lifecycle ----
    PluginLoaded     { plugin: Path, version: String },
    PluginUnloaded   { plugin: Path },
    PluginError      { plugin: Path, message: String },

    // ---- Theme ----
    ThemeChanged     { theme: Path, palette: ThemePalette },

    // ---- Settings ----
    SettingChanged   { setting: Path, value: SettingValue },

    // ---- Highlighter (T-80) ----
    HighlightsReady  { doc: Path, highlights: Vec<HighlightSpan> },

    // ---- Render coordination ----
    RenderDirty      { reason: DirtyReason },

    // ---- Process lifecycle ----
    StartupFinished,
    ShutdownRequested,

    // ---- Frontend-originated (inbound) ----
    ViewportChanged  { frame: Path, top_line: u32, visible_rows: u32 },
    InputReceived    { event: InputEvent },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationSource { Keymap, Palette, Plugin, Programmatic }

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModalKind { Palette, Picker, Custom }

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirtyReason { Buffer, Layout, Theme, Modal, Frontend }

/// Resolved style table for the active theme. Sent to subscribers
/// (typically frontends) on `ThemeChanged` so they can interpret
/// highlight scope names from `View::Buffer` against the new palette
/// without a separate request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThemePalette {
    pub text: Style,
    pub selection: Style,
    pub scopes: HashMap<String, Style>,
}
```

`Axis`, `InputEvent`, and `Style` are defined in `frontend.md` and
re-exported here.

Properties of every variant:
- Carries `Path` for resource identity (no raw ids).
- `Clone` so the bus fans out to multiple subscribers cheaply.
- `Serialize + Deserialize` from day one (locked decision).
- Payload is data-only — no closures, no `Box<dyn ...>`, nothing
  non-serializable.

## Discriminant enum

The pulse-kind discriminant is exposed as its own enum so filters can match
without instantiating a payload.

```rust
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum PulseKind {
    BufferOpened, BufferChanged, BufferSaved, BufferReloaded, BufferClosed,
    DiskChanged,
    CursorMoved, SelectionChanged,
    TabOpened, TabActivated, TabClosed,
    FrameSplit, FrameClosed,
    SidebarToggled, FocusChanged,
    ModalOpened, ModalDismissed,
    CommandInvoked,
    PluginLoaded, PluginUnloaded, PluginError,
    RenderDirty,
    StartupFinished, ShutdownRequested,
    ViewportChanged, InputReceived,
}

impl Pulse {
    pub fn kind(&self) -> PulseKind { ... }
}
```

The discriminant is what plugins reference in their JSON manifests:

```json
{ "subscribe": [{ "kind": "buffer_changed", "path_prefix": "/buf" }] }
```

## The `PulseBus` API

```rust
pub struct PulseBus { /* internal: Arc<Inner> with mutex on subscribers + queue */ }

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct SubscriptionId(u64);

impl PulseBus {
    pub fn new() -> Self;

    /// Publish a pulse synchronously. Every matching subscriber's handler
    /// runs before publish() returns. See "Delivery semantics" below.
    pub fn publish(&self, pulse: Pulse);

    /// Queue a pulse from a background thread. **Non-blocking**: on a
    /// full queue the pulse is dropped, the overflow counter is
    /// bumped, and `PublishError::Full(pulse)` is returned. The main
    /// loop calls `drain()` once per tick; each queued pulse is then
    /// `publish`-ed synchronously on the main thread.
    pub fn publish_async(&self, pulse: Pulse) -> Result<(), PublishError>;

    /// Snapshot of overflow diagnostics: total dropped count plus
    /// the most-recent dropped `PulseKind`s in arrival order.
    pub fn overflow_snapshot(&self) -> (u64, Vec<PulseKind>);

    /// Drain async-queued pulses. Called by the main loop between ticks.
    /// Returns the count drained, mostly for tests.
    pub fn drain(&self) -> usize;

    /// Drain into `out` *without* dispatching to bus subscribers.
    /// Used by the main loop's typed dispatch path when a handler
    /// needs `&mut` state subscribers can't reach through the spec's
    /// `Fn(&Pulse) + Send + Sync` shape (e.g., `&mut Editor`).
    /// Coexists with `drain` (which dispatches to subscribers); the
    /// loop calls `drain_into` then matches on pulse variants and
    /// invokes typed handlers with its own state. Added during T-61
    /// (see foundations-review log 2026-05-07).
    pub fn drain_into(&self, out: &mut Vec<Pulse>) -> usize;

    /// Register a handler. Returns an id usable for `unsubscribe`.
    pub fn subscribe<F>(&self, filter: PulseFilter, handler: F) -> SubscriptionId
    where F: Fn(&Pulse) + Send + Sync + 'static;

    pub fn unsubscribe(&self, id: SubscriptionId);
}
```

## Filter matching

```rust
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct PulseFilter {
    /// If set, pulse `kind()` must be in this list.
    pub kinds: Option<Vec<PulseKind>>,
    /// If set, the pulse payload's `field` value (a Path) must start with
    /// this prefix. Defaults to `PulseField::Path` when None — matches the
    /// `path` field on BufferChanged, BufferOpened, etc.
    pub path_prefix: Option<Path>,
    /// Which Path field on the payload to test against `path_prefix`.
    /// Typed (not stringly-typed) so a variant rename is a compile error
    /// at every subscriber, not a silent miss at runtime.
    pub field: Option<PulseField>,
}

/// Identifies a *role* a Path field plays on a Pulse payload — not the
/// literal field name. The same role can map to differently-named fields
/// across variants (e.g., `Frame` covers both `frame: Path` and
/// `FrameSplit.source: Path`). The Pulse↔role mapping is a property of
/// the Pulse enum, not a field-name string match. Hand-maintained at v0;
/// macro-derived once the catalog grows — see Open Q4.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PulseField {
    /// `BufferOpened.path`, `BufferChanged.path`, `BufferClosed.path`,
    /// `DiskChanged.path`, `BufferReloaded.path`, `BufferSaved.path`.
    Path,
    /// `CursorMoved.cursor`, `SelectionChanged.cursor`.
    Cursor,
    /// `CursorMoved.doc`, `SelectionChanged.doc`, `TabOpened.doc`,
    /// `TabActivated.doc`, `TabClosed.doc`.
    Doc,
    /// `TabOpened.frame`, `TabActivated.frame`, `TabClosed.frame`,
    /// `FrameSplit.source`, `FrameClosed.frame`, `ModalOpened.frame`,
    /// `ViewportChanged.frame`.
    Frame,
    /// `FrameSplit.new`.
    NewFrame,
    /// `SidebarToggled.slot`.
    Slot,
    /// `FocusChanged.from`.
    FocusFrom,
    /// `FocusChanged.to`.
    FocusTo,
    /// `CommandInvoked.command`.
    Command,
    /// `PluginLoaded.plugin`, `PluginUnloaded.plugin`, `PluginError.plugin`.
    /// Always shaped `/plugin/<name>`.
    Plugin,
    /// `ThemeChanged.theme`. Always shaped `/theme/<id>`.
    Theme,
}

impl PulseFilter {
    pub fn any() -> Self;
    pub fn kind(k: PulseKind) -> Self;
    pub fn kinds<I: IntoIterator<Item = PulseKind>>(ks: I) -> Self;
    pub fn under(prefix: Path) -> Self;
    pub fn under_field(field: PulseField, prefix: Path) -> Self;
}
```

Plugin manifests serialize `PulseField` as snake_case (`"doc"`, `"cursor"`,
`"frame"`, ...). A typo in a manifest like `"field": "document"` is a
deserialize error at plugin-load time, not a silent miss at runtime.

Common patterns:

```rust
// Every BufferChanged pulse, on every buffer.
bus.subscribe(PulseFilter::kind(PulseKind::BufferChanged), |p| { ... });

// Anything happening to one specific document.
bus.subscribe(PulseFilter::under(Path::parse("/buf/42")?), |p| { ... });

// CursorMoved on cursors of one document.
bus.subscribe(
    PulseFilter {
        kinds: Some(vec![PulseKind::CursorMoved]),
        path_prefix: Some(Path::parse("/buf/42")?),
        field: Some(PulseField::Doc),
    },
    |p| { ... },
);

// Catch-all (debugging, logging).
bus.subscribe(PulseFilter::any(), |p| { ... });
```

The bus dispatches in O(subscribers-matching-kind) per publish, indexing
internally by `PulseKind` so a `BufferChanged` publish doesn't visit
`InputReceived` subscribers.

## Delivery semantics

### In-thread `publish(pulse)`

Synchronous. The bus walks subscribers whose `PulseFilter` matches the
pulse, invokes each handler with `&pulse`, returns when all have finished.

Handlers are `Fn + Send + Sync` (not `FnMut`). Interior mutation goes
through `Mutex` / atomic fields the closure captures. This rules out
"subscribe with `&mut self` accumulator" patterns; consumers that need them
hold their state in an `Arc<Mutex<...>>` they clone into the closure.

No queueing on the in-thread path: `publish` is a stack-direct call into
each handler.

### Re-entrancy

A handler may call `publish` again. The nested publish runs to completion
before the outer publish returns. The bus tracks recursion depth and
**panics on overflow** at a configurable max (default 16) — accidental
cycles are caller bugs, and panicking surfaces them at test time rather
than corrupting state in production.

### Cross-thread `publish_async(pulse)`

Pushes the pulse onto an MPSC queue inside the bus. Background threads call
`publish_async` only — never `publish` — because subscribers run on the
main thread.

The queue is **bounded at 1024 pulses with drop-newest on full**
(F-1 follow-up, 2026-05-12; supersedes the original block-on-full
design). When the queue is full, `publish_async` returns
`PublishError::Full(pulse)` immediately, bumps an internal
`overflow_count`, and stashes the dropped `PulseKind` in a 16-slot
diagnostics ring. Producers that care can inspect the result and
retry/coalesce; most ignore it.

The block-on-full design deadlocked: the input thread publishes a
typed pulse *before* it sends the wake/input message that's supposed
to drain the queue. A full queue therefore blocked the only producer
that could move the loop forward. Drop-newest sheds load instead.

The capacity is configurable on `PulseBus::with_capacity(usize)` for
tests that want a tighter bound to provoke overflow deterministically.
`bus.overflow_snapshot()` exposes the counter + recent-kinds ring
for tests and future `/dev/pulses` diagnostics.

Once per main-loop tick, the runtime calls `bus.drain()`, which pops every
queued pulse and calls `publish` for each. Subscribers see them in queue
order, on the main thread, with normal in-thread synchronous semantics.

This matches today's `EventSink::pulse(closure)` flow: a typed payload
takes the place of the opaque closure, the queueing + drain shape is the
same, and the cap is the same.

### Frontend-originated pulses

The frontend publishes `ViewportChanged` and `InputReceived` via
`publish_async` (frontend may run on a different thread; even if not, the
async path is uniform with everything else cross-thread). Today this is an
in-process clone of the bus; under a future transport, the frontend pushes
a serialized Pulse onto the wire and the core's in-process side does
`publish_async`.

## Plugin subscription

Plugins subscribe from Lua. The host translates a Lua table to a
`PulseFilter` and registers a handler that wraps a Lua callback handle.

```lua
local sub = devix.subscribe(
    { kind = "buffer_changed", path_prefix = "/buf" },
    function(pulse)
        devix.status("buf changed: " .. pulse.path)
    end
)

-- later
devix.unsubscribe(sub)
```

Pulse payloads cross to Lua as tables (the same shape serde would emit).
The Lua handle is invoked on the main thread by the existing
`PluginRuntime` invoke channel — Lua never sees a non-main-thread call.

## Versioning

The `Pulse` enum is versioned with the `devix-protocol` crate's semver.

- Adding a variant: minor bump.
- Adding a field to an existing variant in a backwards-compatible way (new
  field, default at decode): minor bump. `serde(default)` is required on
  any new field.
- Renaming a variant or field, removing a variant or field, changing a
  field type: major bump.

Plugin manifests declare a required pulse-bus version. The plugin loader
rejects plugins whose required minor exceeds the current; this prevents a
plugin built against `0.3` from running on `0.2` and silently missing
variants.

## What does not flow over the bus

- **View IR.** Render output is request/response, not event-driven. Core
  exposes `view(&self, root: Path) -> View`; the frontend asks when it's
  ready to paint. The bus carries `RenderDirty` as a hint, not the view
  itself.
- **Buffer text.** `BufferChanged` carries a revision number, not the new
  text. Subscribers that need the text re-read it from the document store.
- **Tree-sitter parses.** A reparse is internal to the highlighter; the
  pulse that matters externally is `BufferChanged`.

This is deliberate — fanning text or render trees through the bus would
fan-out copies of large data per subscriber. The bus stays small and fast.

## Interaction with other Stage-0 specs

- **`namespace.md`**: every pulse payload uses `Path`, never raw ids.
  `PulseFilter::path_prefix` uses `Path::starts_with`. Resolved id encoding
  (process-monotonic counter) means `/buf/42` is stable across the session,
  so a long-lived subscription keyed on `/buf/42` does not silently move to
  a different buffer.
- **`protocol.md`**: pulses are the dominant message kind on the
  core↔plugin and core↔frontend boundaries. The protocol spec defines how
  the in-process bus maps onto a future wire (serialize Pulse; submit to
  remote bus's queue).
- **`manifest.md`**: plugin manifests carry a `subscribe` section describing
  filters; the loader registers them on plugin activation.
- **`frontend.md`**: defines `InputEvent` and `Axis` (re-exported here);
  consumes `RenderDirty` to know when to ask for a fresh view; produces
  `ViewportChanged` and `InputReceived`.
- **`crates.md`**: `Pulse`, `PulseKind`, `PulseFilter`, `PulseBus`,
  `SubscriptionId` all live in `devix-protocol`.

## Open questions

1. ~~**Reentrancy depth limit.**~~ *Resolved during T-31
   (2026-05-07): default 16 confirmed. Configurable via
   `PulseBus::with_depth_limit(usize)` for tests that want to provoke
   shallower-overflow scenarios deterministically. See amendment log.*

2. **Wall-clock timestamps on every pulse.** Useful for plugin-side
   debounce / throttle. Lean: no — keeps pulses small; plugins use their
   own clock if they need it.

3. **Per-pulse priority.** Some pulses (`ShutdownRequested`) should jump
   the queue. Defer until we have a use case where ordering matters in
   practice; for now, FIFO.

4. **Macro-derived `PulseKind` / `PulseField`.** Either hand-maintain both
   or `derive(EnumKind)` via a proc macro. Hand-maintained is fine at v0
   size (~25 variants, ~9 fields); switch to derived once the catalog
   doubles. Decide during T-21.

## Resolved during initial review

- `PulseFilter` field selector → typed `PulseField` enum (not stringly-typed).
  Variant rename is a compile error at every subscriber; manifest typos are
  deserialize errors at load time.
- `publish_async` backpressure → bounded queue (1024 default) with
  block-on-full. Configurable via `PulseBus::with_capacity`.
- Pulse history / replay → not in v0. Plugins that need it maintain their
  own ring. Door is not closed; can land later as opt-in
  `subscribe_with_replay(n, ...)` if a real pattern needs it.
