# devix — Foundations cross-spec review

Status: working draft. Stage-0 foundation T-04. **Approval gate** for the
five foundation specs that precede it.

## Purpose

Final consistency check across the six Stage-0 specs:

1. `namespace.md` — `Path`, `Lookup`.
2. `pulse-bus.md` — `Pulse`, `PulseFilter`, `PulseBus`.
3. `protocol.md` — `Envelope`, lanes, `Capability`, `Request` / `Response`.
4. `manifest.md` — JSON manifest schema for plugins and built-ins.
5. `frontend.md` — View IR, `ViewNodeId`, `InputEvent`.
6. `crates.md` — five-crate layout, file migration, dependency graph.

This doc verifies the six compose without overlap or contradiction. It
also aggregates the remaining open questions from each spec, sorted by
which Stage-1+ task gates them.

When this doc is signed off, Stage 0 is complete and Stage 1 (crate
split) starts.

## Spec overview (one paragraph each)

**`namespace.md`** establishes the unified path-shaped naming surface.
Every resource (buffers, cursors, frames, sidebars, commands, chords,
themes, plugin handles) is reachable as `/<root>/<segments>...`. The
`Lookup` trait is the one interface registries implement; it's local
per-registry rather than a global multi-resource lookup. Path-facing
ids are process-monotonic counters; chord segments are kebab-case;
`/` (empty path) is forbidden.

**`pulse-bus.md`** defines the closed-enum versioned message bus that
replaces today's closure-as-message + per-callback ad-hoc surfaces.
27 v0 variants cover buffer, cursor, layout, focus, modal, command,
plugin, theme, render, lifecycle, and frontend-originated events. Every
payload uses `Path` for resource identity. `PulseBus` exposes synchronous
`publish` + cross-thread `publish_async` (bounded 1024, block-on-full)
+ `drain` for the main loop. Subscribers register typed `PulseFilter`s;
field selectors are typed `PulseField` enum (not stringly-typed).

**`protocol.md`** defines the message-passing contract between
separable subsystems. Three lanes: client↔core, plugin↔core, internal
core↔core. Versioned `Envelope` with `seq` for request/response
correlation. Capability negotiation in handshake; capabilities are
fine-grained (16 bits across rendering, contributions, runtime API).
Pulses are one kind of protocol message; the bus is the in-process
implementation of the in-process lanes. Frontends and plugins both
subscribe via `Subscribe`/`Unsubscribe` lane messages.

**`manifest.md`** specifies the JSON schema for plugin manifests and the
shape built-in subsystems use to declare their commands, keymaps, panes,
themes, settings, and pulse subscriptions. Activation events are
deferred. Keymap conflicts refuse the second binding with an
explicit user override list at
`$XDG_CONFIG_HOME/devix/keymap-overrides.json`. Built-ins load before
plugins; plugins cannot silently override built-in chords. The `devix-`
name prefix is reserved for first-party manifests.

**`frontend.md`** defines the View IR (closed enum: `Empty`, `Text`,
`Stack`, `List`, `Buffer`, `TabStrip`, `Sidebar`, `Split`, `Popup`,
`Modal`), `ViewNodeId` (stable ids on every node, paths for
resource-bound nodes), animation hints (gated on `Capability::Animations`),
input event normalization, and the `FrontendHandle` / `CoreHandle`
traits. Layout / virtualization is *not* in core — `LinearLayout` /
`UniformLayout` / scroll cell math live in `devix-tui`. Buffer
highlights carry scope names (not pre-resolved styles); themes ship as a
palette via `Pulse::ThemeChanged`.

**`crates.md`** is the source of truth for the Stage-1 crate split.
Five crates: `devix-text` and `devix-syntax` (existing, unchanged);
`devix-protocol` (NEW, pure data + serde); `devix-core` (NEW, engine,
absorbs editor + plugin + most of panes); `devix-tui` (renamed from
`devix-app`, absorbs widget code + layout primitives). No cycles. `mlua`
is an unconditional `devix-core` dep. File-level migration table is
explicit.

## Cross-cutting alignment

### Are pulses, protocol messages, and View IR disjoint?

Yes — the three concepts each own a distinct slice:

| Concept | Owns | Lifecycle |
|---|---|---|
| **Pulse** | Events / state changes ("something happened") | Published once per event; subscribers receive copies |
| **Protocol message** | Wire-level wrapper around any cross-lane payload | One per lane interaction; envelope adds version + seq |
| **View IR** | A snapshot of the renderable visual state | Produced on demand by `Request::View`; consumed by frontend |

Their relationship:
- A pulse becomes a protocol message when it crosses a lane boundary
  (`ClientToCore::Pulse(p)`, `PluginToCore::Pulse(p)`,
  `CoreToClient::Pulse(p)`). In-process today, no envelope is
  constructed; the bus is the lane.
- A View is the typed payload of `Response::View(ViewResponse { view, ... })`,
  the response to `Request::View`. It never flows over the bus.
- Pulses signal *that* something changed (e.g., `RenderDirty`); the
  frontend reacts by issuing `Request::View`; the response carries the
  new View.

No overlap. No type lives in two of the three.

### String-canonical serialization pattern

Five types use **custom `Serialize` / `Deserialize` impls** (not derived)
because their canonical wire form is a single string. Plugin manifests
and pulse payloads both expect the string shape; the Rust struct is
what consumers match on internally.

| Type | Canonical string | Defined in |
|---|---|---|
| `Path` | `/buf/42`, `/cmd/edit.copy`, `/keymap/ctrl-s` | `namespace.md` |
| `Chord` | `ctrl-s`, `ctrl-shift-p`, `alt-left` | `frontend.md` |
| `KeyCode` | the `<key>` segment of a chord (`p`, `enter`, `f12`) | `frontend.md` |
| `Color` | `#rrggbb`, `red`, `@42`, `default` | `frontend.md` |
| `ProtocolVersion` | `0.1`, `1.42` | `protocol.md` |

Pattern rule: when a type has a *user-edited canonical form*
(manifests, settings files, log lines) **and** also flows through
serde-derived containers (pulse payloads, protocol envelopes), use
custom string-form serde so both surfaces see the same shape.
Structured serde forms (`{"major": 0, "minor": 1}`) are reserved for
types that exist only in code.

### Vocabulary alignment

One canonical name per concept; specs cross-reference rather than
re-define:

| Concept | Type | Defined in | Re-exported from |
|---|---|---|---|
| Path | `Path` | `namespace.md` | (used everywhere; no re-export) |
| Resource registry | `Lookup` trait | `namespace.md` | impls in `devix-core` |
| Event | `Pulse` | `pulse-bus.md` | (carried by protocol lanes) |
| Subscription | `SubscriptionId`, `PulseFilter`, `PulseField` | `pulse-bus.md` | manifest schema references field/kind names |
| Lane envelope | `Envelope<T>` | `protocol.md` | wraps any lane payload |
| Lane payload | `ClientToCore` / `CoreToClient` / `PluginToCore` / `CoreToPlugin` | `protocol.md` | |
| Versioning | `ProtocolVersion`, `Capability` | `protocol.md` | manifest's `engines` reports the same versions |
| Manifest | `Manifest`, `Contributes`, `*Spec` | `manifest.md` | |
| Render IR | `View`, `ViewNodeId` | `frontend.md` | View shipped via `Response::View` |
| Style table | `ThemePalette` | `pulse-bus.md` | `ThemeChanged` payload |
| Resolved style | `Style`, `Color` | `frontend.md` | Theme manifest deserializes into these |
| Input | `InputEvent`, `Chord`, `KeyCode`, `Modifiers` | `frontend.md` | `Pulse::InputReceived` payload |
| Crate location | (per type) | `crates.md` | source-of-truth for where each type lives |

`Axis` and `SidebarSlot` are defined once in `frontend.md` and used by
`pulse-bus.md` (in `FrameSplit`, `SidebarToggled`).

`HighlightSpan` is defined in `devix-syntax`, re-exported from
`devix-protocol::view`.

### Versioning alignment

Three independently-versioned surfaces report through one handshake:

| Surface | Version | Reported in |
|---|---|---|
| Protocol (envelopes, lane vocabulary, capabilities) | `protocol_version` | `Hello`/`Welcome` |
| Pulse catalog | `pulse_bus_version` | `Hello`/`Welcome` |
| Manifest schema | `manifest_version` | `Hello`/`Welcome`, plus `engines.manifest` in the manifest itself |

Plugins declare required minor versions in `engines`. Hosts negotiate
the lower of declared/host minor for each. Major mismatches are fatal
in any of the three.

A frontend doesn't ship a manifest; it reports `manifest_version` only
to indicate it understands manifest-derived contribution data flowing
back (e.g., a settings UI rendering manifest-declared settings).

### Capability alignment

Every capability bit gates a feature visible across multiple specs:

| Capability | Touches | Effect when missing |
|---|---|---|
| `ViewTree` | `frontend.md`, `protocol.md` | Frontend can't consume View IR; the lane is incompatible |
| `StableViewIds` | `frontend.md` | Frontend ignores `ViewNodeId`; no diffing/animation |
| `UnicodeFull` | `frontend.md` | Core may pre-strip combining chars; `view` IR is unchanged in shape |
| `TruecolorStyles` | `frontend.md` | Frontend quantizes Color::Rgb to indexed at paint time |
| `Animations` | `frontend.md` | Core sets every `transition` field to None |
| `ContributeCommands` | `manifest.md`, `protocol.md` | Plugin manifest's `contributes.commands` is rejected with warning |
| `ContributeKeymaps` | `manifest.md`, `protocol.md` | Same |
| `ContributeSidebarPane` | `manifest.md`, `protocol.md` | Same |
| `ContributeOverlayPane` | `manifest.md`, `protocol.md`, `frontend.md` | Floating popups via plugins refused |
| `ContributeStatusItem` | `manifest.md`, `protocol.md`, `frontend.md` | (v1+) status items refused |
| `ContributeThemes` | `manifest.md` | Theme contributions refused |
| `ContributeSettings` | `manifest.md` | Settings contributions refused |
| `SubscribePulses` | `protocol.md`, `pulse-bus.md`, `manifest.md` | `Subscribe` messages and manifest `subscribe` rejected |
| `InvokeCommands` | `protocol.md` | Plugin can't call `InvokeCommand`; Lua API hides it |
| `OpenPath` | `protocol.md` | `OpenPath` requests refused |
| `ReadDir` | `protocol.md` | Plugin's `devix.read_dir` Lua call returns an error |

Adding a new feature later is "advertise the bit, ship the code on both
sides." Plugins that don't know about the bit are unaffected.

### Layering check

For each crate, what specs contribute types?

| Crate | Specs |
|---|---|
| `devix-text` | (substrate; no spec touches it directly) |
| `devix-syntax` | (substrate; reused as `HighlightSpan` from frontend's `View::Buffer`) |
| `devix-protocol` | namespace, pulse-bus *types*, protocol, manifest, frontend, view |
| `devix-core` | pulse-bus *implementation*, namespace registries (`Lookup` impls), manifest loader, theme registry, plugin host, core engine |
| `devix-tui` | View IR interpreter, layout primitives, ratatui adapters, input thread |

Dependency graph (from `crates.md`):

```
text ─┐
      ├─→ protocol ─┐
syntax┤             ├─→ core ─┐
      └─────────────┤         ├─→ tui
                    └─────────┘
```

No cycles; verified by inspection of each crate's declared deps.

## Migration consistency

The audit identified eleven ad-hoc registries. After Stage 5 (registries
onto namespace), every entry maps to a path:

| Today's lookup | Path | Owning crate |
|---|---|---|
| `documents.get(DocId)` | `/buf/<id>` | `devix-core` |
| `cursors.get(CursorId)` | `/cur/<id>` | `devix-core` |
| `LayoutNode::at_path(&[usize])` | `/pane(/<i>)*` | `devix-core` |
| `frame_rects.get(&FrameId)` | (TUI-internal cache; no path) | `devix-tui` |
| `sidebar_rects.get(&SidebarSlot)` | (same) | `devix-tui` |
| `tab_strips.get(&FrameId)` | (same) | `devix-tui` |
| `CommandRegistry::by_id(CommandId)` | `/cmd/<dotted-id>` | `devix-core` |
| `Keymap::bindings(Chord)` | `/keymap/<chord>` | `devix-core` |
| `Theme::scopes(&str)` | `/theme/<scope>` | `devix-core` |
| Plugin `callbacks(u64)` | `/plugin/<name>/cb/<u64>` | `devix-core` |
| Plugin `pane_callbacks(u64)` | `/plugin/<name>/cb/<u64>` (same registry) | `devix-core` |

All eleven addressable through one syntax. No registry remains
private-typed-key-only.

## Aggregated open questions

Carry-forward open items from the six specs, sorted by the Stage-1+
task that gates them. Each item retains its source spec and number for
backtracking.

### Gate: T-22 (protocol skeleton)

- **`PathKind` for `Request::ListPaths`** (protocol.md Q6): what enum
  values? `Buffer` / `Cursor` / `Frame` / `Sidebar` / `Command` /
  `Theme` / `Plugin`? Decide when implementing the request handler.

- **Streaming responses** (protocol.md Q1): single-batch vs streaming
  for big-result commands. Defer until first big-result use case
  (likely workspace symbol search).

- **Plugin capability mismatch policy** (protocol.md Q2): warn-degrade
  with plugin opt-out (lean) vs strict refuse-to-load. Confirm during
  T-22.

- **Internal lane formalization** (protocol.md Q3): supervised actors
  use pulses + direct calls; envelope-bound mailbox not needed for v0.
  Confirm.

- **Session lifecycle pulses** (protocol.md Q5): add
  `ClientConnected` / `ClientDisconnected` to the v0 pulse catalog so
  subscribers can react to lane attach/detach. Lean: yes; add during
  T-21 (pulse bus skeleton) so it lands together.

### Gate: T-21 (pulse bus skeleton)

- **Reentrancy depth limit default** (pulse-bus.md Q1): default 16
  picked arbitrarily. Confirm; add `with_depth_limit` builder.

- **Per-pulse priority** (pulse-bus.md Q3): some pulses
  (`ShutdownRequested`) might want to jump the queue. Defer; FIFO for
  v0.

- **Macro-derived `PulseKind` / `PulseField`** (pulse-bus.md Q4):
  hand-maintain at v0 size; switch to derive macro when the catalog
  doubles. Decide during T-21.

- **Wall-clock timestamps on pulses** (pulse-bus.md Q2): no in v0.
  Confirm.

### Gate: T-23 (manifest reader skeleton)

- **Settings type system** (manifest.md Q2): flat for v0; revisit when
  Settings UI ships.

- **Plugin entry script type** (manifest.md Q3): Lua-only in v0;
  add `entry_type` field when a second runtime ships.

- **Manifest hot-reload** (manifest.md Q4): no in v0; restart-required.
  Specify `Pulse::PluginManifestChanged` and unload path when the need
  appears.

- **JSON Schema document** (manifest.md Q5): generate via `schemars`
  during T-23 so user-written manifests get IDE validation.

### Gate: T-30 (migrate documents onto namespace)

- **`lookup_mut` and the borrow checker** (namespace.md Q1):
  decide on `lookup_two_mut` helper vs split-borrow on the store types
  vs direct slotmap access for ops that need disjoint borrows.

- **Globbing / patterns** (namespace.md Q2): defer; `paths()` +
  caller-side filter is enough until a use case appears.

- **Path → typed-id round trip** (namespace.md Q3): per-root parsers
  (`Document::id_from_path(&Path)`). Confirm shape.

- **Serde canonical form for `Path`** (namespace.md Q4): canonical
  string (lean). Confirm; rule out segment-array wire form.

### Gate: T-71 (LayoutNode → Pane collapse)

- **`SidebarSlot::Floating`** (frontend.md Q1): overlay panes are
  top-level `View::Popup` siblings, not sidebar slots (lean). Confirm
  during T-71.

- **List virtualization windowing** (frontend.md Q2): defer until
  first big-list use case ships.

- **Z-order for stacked Popup/Modal** (frontend.md Q3): tree-position
  order. Confirm.

- **Drag-drop / file-drop / IME** (frontend.md Q4): defer to v1.

- **`Chord::text` ambiguity** (frontend.md Q5): always set when
  printable. Confirm.

- **Pane / Action trait location** (crates.md Q2): keep in `devix-core`
  (lean). Confirm.

- **Theme location split** (crates.md Q3): `ThemeSpec` in protocol,
  `Theme` (active state) in core. Confirm.

### Gate: post-v0

- **Activation events** (manifest.md Q1): deferred per locked
  decision.

- **`devix-protocol` on crates.io** (crates.md Q4): future;
  publishable contract for plugin authors.

### Gate: T-12 (rename app → tui)

- **Workspace member naming** (crates.md Q5): match crate name to
  directory name. Confirm.

- **`devix-builtin` JSON Schema** (manifest.md Q5): same as plugin
  schema; one schema doc.

## Verification: principles vs specs

The principles audit identified twelve major findings (across MLIR,
Plan 9, Smalltalk, Acton, Erlang, LSP, VS Code, Hickey). Each Stage-0
spec addresses a subset:

| Principle | Spec(s) addressing it |
|---|---|
| MLIR (one primitive, dialects extend) | `frontend.md` (View IR is the universal render unit), `crates.md` (Stage-9 collapses LayoutNode into Pane after these foundations land) |
| Plan 9 (uniform addressing) | `namespace.md` |
| Smalltalk (named messaging, late binding) | `pulse-bus.md` |
| Acton (data-oriented, design data first) | `frontend.md` (View IR is data; no closures), `pulse-bus.md` (Pulse payloads are pure data), `crates.md` (devix-protocol is pure data) |
| Erlang (supervised isolation) | Future Stage-8 specs (tree-sitter actor, plugin supervisor) read these foundations |
| LSP (versioned protocol, capabilities) | `protocol.md` |
| VS Code (declarative manifests, lazy activation) | `manifest.md` (declarative; activation deferred) |
| Hickey (simple, un-braided) | `crates.md` (clean dependency graph; pure-data devix-protocol; no god-struct survives the migration) |
| SICP (primitives, combinators, abstraction) | `frontend.md` View combinators; `pulse-bus.md` Pulse + filter combinators; `manifest.md` declarative composition |

Stage-0 specs alone don't *fix* any principle violation — they describe
the foundations that Stages 1–13 implement. Sign-off here is sign-off on
the *target shape*, not on having reached it.

## Sign-off checklist

Stage 0 is complete when:

- [ ] All six foundation specs (T-00 namespace, T-01 pulse-bus, T-02
      protocol, T-03 manifest, T-05 frontend, T-06 crates) are reviewed
      and accepted.
- [ ] All cross-spec consistency checks in this document pass (no
      contradictions, no double-defined types, no missing
      cross-references).
- [ ] All "Resolved during initial review" sections in the six specs
      reflect the user's locked decisions.
- [ ] Aggregated open questions are accepted as deferred-to-task-X with
      explicit gates, not as unanswered show-stoppers.
- [ ] User confirms readiness to proceed to Stage 1 (crate split).

## What happens after sign-off

Stage 1 begins (T-10 through T-13). The crate split executes against
this spec set as the reference. Each subsequent stage reads from the
specs without re-asking the locked decisions.

Spec changes during implementation: any code-level discovery that
requires changing a Stage-0 decision is a *spec amendment* — the
relevant spec gets updated, this review doc gets a delta line, and the
amendment is signed off before the implementation lands. No silent
drift between code and spec.

## Spec-to-implementation feedback loop

**Policy: Strict.** Any code that contradicts a spec is rejected. Spec
changes go through an amendment process:

1. Open a spec amendment: edit the relevant Stage-0 spec doc.
2. Add a delta line in this review doc's *Amendment log* section
   (below) describing what changed and why.
3. Sign-off on the amendment (same gate semantics as the original spec
   review).
4. Then the code lands.

This is heavier than letting code drive the spec, deliberately. The
foundations exist to prevent re-refactoring two stages from now; loose
coupling between code and spec invites that exact failure mode.

Pragmatic shortcuts (`// DEVIATES FROM SPEC` comments, "fix the spec
later") are not allowed — they accumulate into the same drift the
strict policy is meant to prevent.

### Amendment log

- **2026-05-08 — T-91 phase 1 + phase-2 partial progress.**

  Phase 1: `RenderCtx` widens with `layout: Option<&'a
  LayoutCtx<'a>>` — structural render populates Some, chrome / modal
  / plugin panes pass None and ignore. Decision locked over two
  alternatives (parallel render paths; new sub-trait `LayoutPane`
  extending `Pane` with a `paint(area, frame, &LayoutCtx)` method);
  user picked widening for the simplest single-trait surface.
  Trade-off acknowledged: `RenderCtx` now references a non-protocol
  borrow type (`LayoutCtx`) defined in `devix-core`. Acceptable —
  `RenderCtx` itself lives in `devix-core` and never crosses the
  wire. `PaneRegistry.root` shifts from `LayoutNode` to
  `Box<dyn Pane>`; typed methods recover `&LayoutNode` through
  `Pane::as_any` → `downcast_ref`.

  Phase 2 partial: `Pane::children_mut` (default empty) added to the
  trait. `LayoutSplit`, `LayoutFrame`, `LayoutSidebar` each impl
  `Pane` directly (LayoutSplit's `Pane::render` recurses via the
  trait, `Pane::children`/`children_mut` map its existing rect math
  through; LayoutFrame and LayoutSidebar wrap their existing render
  helpers). LayoutNode's `Pane` impl now delegates via match.
  `PaneRegistry::pane_paths` switches to a Pane-trait-driven walk
  generic over the concrete composite. *Still deferred*: change
  `LayoutSplit.children` from `Vec<(LayoutNode, u16)>` to
  `Vec<(Box<dyn Pane>, u16)>` (the breaking restructure that lets
  the enum go), rewrite `mutate::*` as Pane-tree-walk mutations,
  hoist `LayoutNode`'s match-based methods into `PaneRegistry`
  helper functions over `&dyn Pane`, re-wire `editor::{focus,
  hittest, ops, view}` accordingly, and finally delete the enum.
  T-92 / T-94 / T-95 still gate on the completion.

- **2026-05-08 — Stage 11 partial: T-110 / T-111 / T-112 / T-113 all
  ship as partials.**
  - **T-110** widens `CommandId` from tuple-struct `&'static str` to
    `{ plugin: Option<&'static str>, id: &'static str }` with
    `CommandId::builtin(id)` / `CommandId::plugin(plugin, id)`
    constructors; `to_path()` produces `/cmd/<id>` or
    `/plugin/<name>/cmd/<id>` per kind, and `CommandRegistry`'s
    `Lookup` resolves both. `PluginRuntime::install_with_manifest`
    registers manifest's `contributes.commands` at
    `/plugin/<manifest.name>/cmd/<id>`, matched to Lua-registered
    handles by id (orphans publish `PluginError`). Three same-day
    follow-ups closed three deferred items:
    (1) `register_keymap_contributions` / `apply_keymap_overrides`
    resolve `/plugin/<name>/cmd/<id>` as a keymap-binding command;
    (2) new `Keymap::bind_command_if_free` enables first-loaded-wins
    on chord conflicts (plugin-vs-plugin / plugin-vs-builtin); the
    runtime fires `PluginError` describing contested chords with the
    existing binding intact;
    (3) `install_with_manifest` now also reads
    `manifest.contributes.keymaps` and binds them via
    `BindPolicy::IfFree`, so plugins can declare chords in JSON
    rather than the Lua-side `chord` field.
  - **T-111** cross-checks each `manifest.contributes.panes` entry
    against the runtime's Lua-side `register_pane` registrations by
    slot; orphan declarations publish `PluginError`. The declared
    `id` is documented but not yet used as a registry key — panes
    still install onto the editor's structural tree by slot.
  - **T-112** new `theme_store` module: `register_from_manifest`
    seeds defaults, `activate(store, id, bus) -> Option<Theme>`
    builds the in-memory `Theme` and emits `Pulse::ThemeChanged`
    with wire-shape `ThemePalette`. First-loaded-wins on theme-id
    collisions.
  - **T-113** new `settings_store` module:
    `register_from_manifest` seeds typed defaults
    (`Boolean | String | Number | EnumString` per `manifest.md` v0
    lock), `apply_overrides_from_file` reads
    `$XDG_CONFIG_HOME/devix/settings.json` with type-mismatch and
    enum-out-of-range error surfaces, `settings_overrides_path()`
    resolver. Lua bridge (`devix.setting(key)`) deferred — threads
    `Arc<Mutex<SettingsStore>>` through `PluginHost::new` along
    with the T-81-full module reorg.
  - **main.rs** walks `plugin_dir()` for every `manifest.json`
    subdirectory, loads each under the supervisor (T-81 partial),
    and wires its commands via `install_with_manifest`. Legacy
    `DEVIX_PLUGIN` single-file path stays alive at `/cmd/<id>`.
  - *Deferred across Stage 11*: capability negotiation
    (`ContributeCommands` / `ContributeThemes` /
    `ContributeSidebarPane` / `StableViewIds` /
    `ContributeSettings` warn-and-degrade — all need T-81 full);
    runtime theme-switch UI; the `devix.setting` Lua bridge; Lua →
    Rust `View` IR marshaling for plugin panes; plugin-pane
    `/plugin/<name>/pane/<id>` path-based addressing (waits on
    Stage-9 / T-91 Pane-tree unification).

- **2026-05-08 — T-81 partial close (supervised plugin thread; restart
  deferred).** `PluginRuntime::load_supervised(path, sink, bus)` wraps
  the plugin worker thread in `supervise()` with `max_restarts: 0`. A
  Lua-side panic escalates as `Pulse::PluginError` on the editor's
  bus; the editor stays responsive. Successful load publishes
  `Pulse::PluginLoaded`. Custom `Drop` on `PluginRuntime` fires a
  oneshot shutdown so the loop's `tokio::select!` exits even when an
  installed plugin pane keeps an `input_tx` clone alive. *Deferred*
  from T-81 spec: channel-re-acquisition restart (needs
  `Arc<Mutex<Option<Sender>>>` topology so editor-held senders refresh
  across respawn) and the module reorg into
  `host`/`runtime`/`bridge`/`pane_handle` (per `crates.md`). Both go
  together in a future T-81 follow-up sprint.

- **2026-05-08 — Loose-end wire-ups in `main.rs`.** Two helpers
  landed earlier (T-72 / T-73) but were never called: theme now
  loads from the embedded `BUILTIN_MANIFEST` via
  `theme_from_manifest("default")` (with a fallback to
  `Theme::default()`); user keymap overrides apply from
  `$XDG_CONFIG_HOME/devix/keymap-overrides.json` after manifest
  bindings. Errors surface to stderr; missing override file is silent.

- **2026-05-07 — Stage 10 close (T-100 / T-101 / T-102 / T-103 / T-104
  ship).** Editor god-struct decomposed into 8 typed owners:
  `documents: DocStore`, `cursors: CursorStore`, `bus: PulseBus`,
  `panes: PaneRegistry` (T-100 — owns the layout tree, exposes
  `find_frame` / `at_path` / `pane_at(&Path)` / `replace_at` /
  `remove_at` / `collapse_singletons` / `lift_into_horizontal_split`),
  `modal: ModalSlot` (T-103 — at-most-one-modal invariant, paired with
  `Editor::open_modal` / `dismiss_modal` helpers that emit
  `ModalOpened` / `ModalDismissed`), `focus: FocusChain` (T-101 —
  active path + transition diff; `Editor::set_focus` emits
  `FocusChanged` exactly when the path changes), `doc_index:
  HashMap<PathBuf, DocId>` (path-dedup cache), `render_cache:
  RenderCache`. Ops (T-102) remain `impl Editor` methods rather than
  free functions (kept the typed-owner-API spirit; free-fn signatures
  would have pushed 4–5 owner refs per call with no behavioural gain).
  Each layout op publishes its spec'd pulse (`FrameSplit` /
  `FrameClosed` / `SidebarToggled`). Build clean, 260 tests pass;
  manual TUI sanity deferred to local verification.

- **2026-05-07 — Stage 9 partial close (T-90, T-93 ship;
  T-91 / T-92 / T-94 / T-95 deferred).** T-90 locks the
  synthetic-id strategy (see entry below). T-93 confirms
  `Pane` and `Action` trait location in `devix-core` with
  doc-comments on `pane.rs` / `action.rs`.

  T-91 (collapse `LayoutNode` into a unified `Pane` tree),
  T-92 (move rect cache to `devix-tui`), T-94 (fold composites),
  and T-95 (regression gate retiring legacy paint) defer to a
  focused Stage-9 sprint. Reasons:

  - T-91 is the largest single refactor in the foundations plan.
    `LayoutNode` is referenced from `editor::tree`, `editor::ops`,
    `editor::focus`, `editor::hittest`, `editor::editor`, plus
    several command implementations (`split.rs`, `tab.rs`). Each
    walker-style call site needs to switch from match-on-variant
    to either `Pane::children()` walks or `as_any` downcasts.
    Estimated 500-1000 lines touched + new tests covering the
    Pane-tree shape.
  - T-92 / T-94 / T-95 transitively depend on T-91.

  T-90's deterministic-derivation lock and T-93's trait-location
  doc are independent and ship now; the structural collapse is
  its own focused sprint.

- **2026-05-07 — `frontend.md` Q1 (synthetic-id strategy)
  resolved — deterministic derivation.** T-90 locks the
  placeholder strategy T-43 shipped: synthetic ids are
  `/synthetic/<kind>/<encoded-parent-path>[/<suffix>]`, where
  the parent's path slashes are encoded as `_` to fit one segment.
  No per-Editor state, no mint-and-cache. The alternative
  (mint-and-cache keyed by structural position) was considered
  and rejected: it gives the same answer for "child at structural
  position i" — same id across renders if i is stable; different
  id if i changes. Without a stable logical-node identity beyond
  structural position (which is what a resource-bound `Path`
  already provides), the cache buys no extra fidelity. Spec text
  in `frontend.md` § *ViewNodeId* describes both options as
  acceptable; locking the simpler one.

- **2026-05-07 — Stage 8 partial close (T-82 ships;
  T-80 / T-81 deferred).** T-82 lands the supervisor primitive
  (`devix_core::supervise`): one-for-one restart strategy, default
  3 restarts in 30s, escalates via `Pulse::PluginError` on budget
  exhaustion. Tests cover clean exit, panic-then-restart, budget
  exhaustion, and drop-stops-supervisor.

  T-80 (tree-sitter highlighter as supervised actor) and T-81
  (plugin runtime as supervised actor) deferred to a Stage-8
  follow-up sprint. Both require structural ownership shifts:

  - T-80: today the `Highlighter` lives inside `Document` and
    `Document::apply_tx` calls `h.parse` synchronously. Migrating
    to a supervised actor means Document gives up the highlighter,
    a worker thread owns it, and View producers consume
    `Pulse::HighlightsReady` (a new variant — would require a
    pulse-bus.md minor bump). Stage 9's LayoutNode collapse +
    Stage 4's view-producer refinements are the natural moment
    to restructure together.
  - T-81: today the plugin runtime spawns its own thread inside
    `load_with_sink`; channels (`invoke_tx`, `input_tx`,
    `msg_rx`) flow back to the editor. The supervisor primitive
    expects to own the spawn. Wrapping requires either (a)
    refactoring `PluginRuntime` so the supervisor owns the spawn
    and exposes channel handles via shared state, or (b) inlining
    a panic-recovery loop inside the existing tokio::select. Real
    supervised behavior (Lua VM reset, re-registration of
    contributions on restart) is its own design problem — better
    landed alongside T-110 / T-111 when manifest-driven plugin
    loading reshapes the boundaries anyway.

  End state: supervisor primitive is shipping infrastructure;
  consumers wire it up when the surrounding code is ready.

- **2026-05-07 — Stage 6 fully closed (after partial-close
  reversal).** The earlier "Stage 6 partial close" entry below was
  reversed in the same session: subsequent work pushed through
  every remaining migration. Final state:

  - `Pulse::OpenPathRequested { fs_path, source }` added to the v0
    catalog (minor `pulse-bus.md` bump).
  - `Effect` / `EffectFn` / `effect.rs` deleted entirely.
    `AppContext::request_redraw` / `quit` flip
    `dirty_request` / `quit_request` flags; `Application::with_context`
    folds them back. The one `ctx.defer` site (wheel coalescing)
    inlined.
  - `EventSink::pulse(closure)` + `LoopMessage::Pulse` + `PulseFn`
    removed. `EventSink` now carries only `Input` / `Wake` / `Quit`.
  - `Wakeup` type alias / `load_with_wakeup` removed from
    `devix-core::plugin`. The MsgSink itself wakes the main loop
    via `EventSink::wake` (`LoopMessage::Wake`).
  - Input thread publishes `Pulse::InputReceived` (parallel
    observer notification); `crossterm_to_input_event` does the
    conversion. Dispatch keeps using `EventSink::Input(crossterm)`
    because the keymap consults shape info beyond `InputEvent`.

  239 tests passing, build clean, clippy clean. The "partial
  close" entry remains below for the historical record.

- **2026-05-07 — Stage 6 partial close (superseded by full close
  above).** Stage 6 lands the bus + two producer migrations
  (disk-watch, plugin pane-changed); the remaining wholesale
  retirement of `EventSink` / `Effect` / `Wakeup` plus the
  input-thread + plugin OpenPath migrations defer to a Stage-6
  follow-up sprint. End-state Stage-6 acceptance criterion ("no
  producer calls into the legacy event types") is partially met:

  - Disk watcher: bus only ✓
  - Plugin PaneChanged: bus only ✓
  - Plugin Status: no-op (no migration needed) ✓
  - Plugin OpenPath: still on EventSink ✗
  - Frontend input (crossterm → AppContext): still on
    LoopMessage::Input ✗
  - Application loop's LoopMessage::Pulse(closure) path: kept as
    the bridge for OpenPath until the full migration ✗

  T-63's "drop legacy types + full regression" criterion narrowed
  to "regression gate against the partial state" (235 tests pass,
  zero warnings). Bus is structurally proven; remaining producer
  migrations follow the same template as T-61 / T-62.

- **2026-05-07 — `pulse-bus.md` extension: `drain_into`.** Adds a
  drain variant that pops the cross-thread queue into a caller-
  owned `Vec<Pulse>` *without* invoking bus subscribers. Lets the
  main loop dispatch typed pulses with `&mut state` (the editor)
  that the spec's `Fn(&Pulse) + Send + Sync` subscriber shape
  can't reach without wrapping the editor in `Arc<Mutex<>>`. Bus
  subscribers still work for cross-cutting concerns
  (logging, plugins) via the existing `subscribe` /
  `publish` / `drain` shape. Spec doc updated under
  *The `PulseBus` API*; impl in `devix-core::bus::PulseBus::drain_into`.

- **2026-05-07 — Stage 6 scope re-allocation.** Original T-60 spec
  said "Replace every existing EventSink::pulse / DiskSink::push /
  MsgSink::send / Wakeup::request call site with PulseBus.publish*"
  in one task. Doing all four producer paths plus the consumer-side
  subscriber wiring in one atomic commit risks long-lived broken
  state on the branch. Re-allocated:

  - **T-60**: land the bus on `Editor` and drain it per tick in
    `Application::run`. Add a dormant
    `install_bus_watcher_for_doc(bus)` shape ready for the disk
    producer to switch onto. No producer rewires yet — the
    closure-based `EventSink` / `DiskSink` / `MsgSink` / `Wakeup`
    paths still drive the runtime.
  - **T-61**: migrate the disk-watcher producer
    (`Pulse::DiskChanged`) plus its subscriber. Retire the
    `DiskSink` callback type.
  - **T-62**: migrate frontend-originated pulses (input,
    viewport) and the plugin MsgSink (`PluginMsg` →
    `Pulse::Plugin*` / `RenderDirty`).
  - **T-63**: drop `Effect` / `EventSink` / `Wakeup`. Stage-6
    regression gate.

  End-of-Stage-6 state matches the original spec; only the
  per-task boundary moves.

- **2026-05-07 — T-56 plugin-callback Lookup deferred.** T-56
  ships path encoding/decoding helpers for `/plugin/<name>/cb/<u64>`
  but defers the full `Lookup<Resource = LuaCallback>` impl to
  T-110 / T-111 (manifest-driven plugin loading). Reason: the
  plugin host's two callback-related maps are an
  `Arc<Mutex<HashMap<u64, RegistryKey>>>` registry plus a
  per-pane *index* into it (`Arc<Mutex<HashMap<u64,
  PaneCallbackKeys>>>`), not two parallel registries. Implementing
  `Lookup` on the locked registry fights the trait's
  `&Resource`-borrow shape; storage redesign waits for the API to
  become load-bearing in plugin contributions. Path encoding
  alone is enough for T-57's pulse-payload sweep.

- **2026-05-07 — Stage-4 deviations accepted.** Three small
  deviations from the spec text accepted during the post-Stage-4
  review:
  - **T-44 byte-parity criterion deferred to T-95.** The original
    "byte-equivalent ratatui buffers vs. legacy direct-paint"
    acceptance is unachievable while the legacy Pane render path
    drives the App's render loop — both renderers would compete.
    `paint_view` ships at T-44 as a structural library function;
    T-95 wires it in and achieves parity as the legacy path
    retires. T-44 task file records the scope change.
  - **T-43 `View::Buffer.highlights` ships empty.** Tree-sitter
    highlights flow once the Stage-8 supervised actor lands.
    Producer code path is ready to plug in; consumers
    (`paint_view`, downstream renderers) tolerate empty.
  - **T-43 buffer paths use `slotmap::Key::data().as_ffi()`** until
    T-50 swaps for the process-monotonic counter. Slotmap key
    reuse violates the "paths are stable across the session"
    property in `namespace.md` *Segment encoding rules → Resource
    ids* during the Stage-4–Stage-5 window. T-50's task file
    notes the shim explicitly so the fix is tied to that task.

- **2026-05-07 — Inner `kind` field renames to avoid serde tag
  collision.** Locked during the post-Stage-3 interactive review.
  Three pulse / input variants previously declared an inner field
  literally named `kind`, colliding with the outer
  `#[serde(tag = "kind")]` discriminant on the parent enum.
  Resolution: rename the Rust field name itself rather than
  `#[serde(rename)]`-only:
  - `pulse-bus.md` — `Pulse::ModalOpened.kind: ModalKind` →
    `Pulse::ModalOpened.modal: ModalKind`; same on
    `Pulse::ModalDismissed`.
  - `frontend.md` — `InputEvent::Mouse.kind: MouseKind` →
    `InputEvent::Mouse.press: MouseKind`.

  The lane payload variants (`ClientToCore::Request`,
  `ClientToCore::Pulse`, `CoreToClient::Response`,
  `CoreToClient::Error`, `PluginToCore::Pulse`,
  `CoreToPlugin::Error`) were converted from tuple variants to
  struct variants nesting the payload — Rust field names already
  disambiguate (`request`, `pulse`, `response`, `error`); no spec
  text renames needed there. Matching spec text amendments landed
  in `docs/specs/pulse-bus.md` and `docs/specs/frontend.md`.

- **2026-05-07 — Stage-3-gating open questions resolved.**
  Locked during the pre-Stage-3 interactive review:
  - `namespace.md` Q1 (lookup_mut): `Lookup` stays single-resource;
    disjoint-borrow ops use `std::mem::{take, swap, replace}`. No
    helpers, no workarounds. Data layout that fights this is the
    thing to fix, not the trait.
  - `protocol.md` Q2 (capability mismatch): warn-and-degrade with
    plugin opt-out (VS Code style).
  - `protocol.md` Q6 (`PathKind` variants): `Buffer, Cursor, Pane,
    Command, Keymap, Theme, Plugin` (seven, one per Stage-5
    namespace-migration root).
  - `pulse-bus.md` Q1 (reentrancy depth): default 16 confirmed.
    `with_depth_limit` builder available.

  Each spec doc's *Open questions* entry is updated with a
  parenthetical pointer back to this log.

- **2026-05-06 — `crates.md` Q5 resolved (workspace member naming).**
  Q5 closed in the affirmative: directory names match `[package]
  name` everywhere. `crates/text` and `crates/syntax` renamed to
  `crates/devix-text` and `crates/devix-syntax` under T-25 (Stage 2,
  prepended). The other three crates (`devix-protocol`, `devix-core`,
  `devix-tui`) already matched. Reason: keeps `cargo` output and
  filesystem navigation consistent with imports; user confirmed
  during post-Stage-1 review.

- **2026-05-06 — `crates.md` § *Stage-1 sequencing* (widget move
  deferred).** T-11 absorbed widgets into `devix-core` to break a
  transient cycle, and T-12 leaves them there rather than forwarding
  to `devix-tui`. Reason: every chrome widget file (`palette.rs`,
  `popup.rs`, `sidebar.rs`, `tabstrip.rs`) plus the layout
  primitives in `widgets/layout.rs` are used by `devix-core` code
  (`composites.rs::SidebarSlotPane`/`TabbedPane`, `editor/tree.rs`,
  `editor/buffer.rs`, `editor/editor.rs`, `editor/commands/modal.rs`).
  Honoring the T-12 sequencing as written would leave the workspace
  in a `devix-core ↔ devix-tui` cycle until later stages
  (Stage 9) dissolve `LayoutNode` and move rect caches to TUI. The
  migration table's eventual destinations still hold; the physical
  move is now sequenced into T-92 (rect caches → TUI) and T-95
  (Stage-9 regression gate). T-12 is reduced to the
  `devix-app → devix-tui` directory rename, file renames per the
  table (`render.rs → interpreter.rs`, `events.rs → input.rs`,
  `input.rs → input_thread.rs`), and Cargo dep cleanup
  (drop tokio, drop devix-text since core re-exports buffer types).