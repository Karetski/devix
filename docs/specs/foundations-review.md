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