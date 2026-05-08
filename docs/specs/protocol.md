# devix — Protocol spec

Status: working draft. Stage-0 foundation T-02.

## Purpose

Define the message-passing contract between separable subsystems of devix.
Three lanes:

1. **Client ↔ Core** — TUI today; GUI / mobile / web later.
2. **Plugin ↔ Core** — Lua plugins today; other plugin runtimes possible later.
3. **Core ↔ Core internal** — cross-crate boundaries inside `devix-core` we
   want kept loose (supervised actors, future LSP client).

Today every lane is in-process. Tomorrow's transports (stdio, Unix socket,
TCP, WebSocket) carry the same message types serialized.

This spec answers LSP's principle: *a narrow versioned protocol with
capability negotiation.* It also extends VS Code's principle: contributions
are declared, not executed-into-existence (manifest.md owns the declaration
shape; this spec covers how those contributions are reported across lanes).

## Scope

This spec covers:
- Message envelope shape and versioning.
- The three lane vocabularies (high-level — concrete Pulse and Manifest
  shapes belong to their own specs).
- Version + capability negotiation handshake.
- Request/response correlation.
- Error model.

This spec does **not** cover:
- Concrete Pulse variants (`pulse-bus.md`).
- Manifest schema (`manifest.md`).
- View IR types (`frontend.md`).
- Transport implementation. Today the in-process bus + direct calls are the
  transport. Future transport specs (stdio framing, etc.) live separately.

## Message envelope

Every message that crosses a lane boundary is wrapped in a versioned
envelope:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub protocol_version: ProtocolVersion,
    pub seq: u64,
    pub payload: T,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ProtocolVersion { pub major: u16, pub minor: u16 }
// Custom Serialize / Deserialize: wire form is the canonical
// "<major>.<minor>" string (e.g., "0.1"), matching how
// `engines.<name>` appears in plugin manifests. Round-trips through
// the same parser the manifest reader uses.
```

`seq` is monotonic per producer. Used for:
- Request/response correlation (response carries the request's seq).
- Ordered delivery semantics for transports that need it.
- Debugging / log correlation.

`protocol_version` is the version of the *envelope and lane vocabulary*,
**not** the pulse-bus catalog version (which is separate, see
`pulse-bus.md`). Both versions are reported in the handshake; both can
evolve independently.

## Lanes

### Lane 1: Client ↔ Core

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClientToCore {
    /// Initial handshake. Client declares capabilities; core responds with
    /// the negotiated set (intersection).
    Hello(ClientHello),
    /// Hand off a pulse the frontend originated (input, viewport).
    Pulse(Pulse),
    /// Subscribe to a pulse stream. Same shape as the plugin lane;
    /// pulses matching `filter` are delivered as `CoreToClient::Pulse`.
    /// Fire-and-forget; errors arrive as `CoreToClient::Error`.
    Subscribe { id: SubscriptionId, filter: PulseFilter },
    /// Drop a previously-installed subscription.
    Unsubscribe { id: SubscriptionId },
    /// Typed request expecting a Response. Envelope's `seq` correlates.
    Request(Request),
    /// Save a buffer (fire-and-forget; result flows as pulses).
    Save { buffer: Path },
    /// Open a path from the OS file picker / drag-drop.
    OpenPath { fs_path: PathBuf },
    /// Graceful shutdown.
    Goodbye,
}

/// Typed requests carrying a response. Each variant pairs with a
/// `Response` variant of the same name; the response is correlated by
/// `Envelope.seq`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request {
    /// Request the current view tree for `root` (a Path).
    View { root: Path },
    /// Invoke a command by path. Fire-and-forget if the command does not
    /// produce a typed result; the response then carries `Result::Ok(())`.
    /// Side effects (BufferChanged etc.) still flow as pulses regardless.
    InvokeCommand { command: Path, args: Option<serde_json::Value> },
    /// Enumerate paths under a prefix from a registry.
    ListPaths { prefix: Path, kinds: Option<Vec<PathKind>> },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoreToClient {
    /// Handshake response. Core declares its negotiated capability set.
    Welcome(ServerWelcome),
    /// A pulse the client subscribed to (or that affects view rendering).
    Pulse(Pulse),
    /// Typed response to a Request. `seq` matches the request's seq.
    Response(Response),
    /// Non-fatal protocol error; client logs and continues.
    Error(ProtocolError),
}

/// Typed responses paired with `Request` variants. Each response carries a
/// `Result` so the request can fail without escalating to a protocol-level
/// error.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    View(Result<ViewResponse, RequestError>),
    InvokeCommand(Result<serde_json::Value, RequestError>),
    ListPaths(Result<Vec<Path>, RequestError>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ViewResponse {
    pub root: Path,
    pub view: View,
    /// Monotonic counter so the client can detect stale views (out-of-order
    /// responses on a future async transport).
    pub version: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RequestError {
    UnknownPath(Path),
    UnknownCommand(Path),
    InvalidArgs(String),
    Cancelled,
    Internal(String),
}
```

The `View` type is defined in `frontend.md`; `version` is a monotonic
counter so the client can detect stale views (out-of-order responses on a
future async transport).

### Lane 2: Plugin ↔ Core

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginToCore {
    Hello(PluginHello),
    Pulse(Pulse),
    InvokeCommand { command: Path, args: Option<serde_json::Value> },
    /// Subscribe to pulses; `id` is plugin-chosen so unsubscribe is easy.
    Subscribe { id: SubscriptionId, filter: PulseFilter },
    Unsubscribe { id: SubscriptionId },
    /// Ask core to open a path (existing devix.open_path semantics).
    OpenPath { fs_path: PathBuf },
    Goodbye,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoreToPlugin {
    Welcome(PluginWelcome),
    /// A pulse matched the plugin's filter; deliver to the plugin's
    /// registered Lua handle for this subscription.
    Deliver { subscription: SubscriptionId, pulse: Pulse },
    /// Invoke a Lua callback registered as part of a contribution
    /// (a command's `lua_handle`, a pane's `lua_handle`).
    InvokeCallback { handle: u64, args: serde_json::Value },
    Error(ProtocolError),
}
```

Note `Deliver` is "core delivering a pulse to a plugin subscriber" — the
core is the source-of-truth bus, plugins receive copies. `Pulse` going the
other direction (plugin → core) is the plugin publishing its own pulse
into the bus (e.g., a linter publishing `LintReady` once that variant
exists).

### Lane 3: Core ↔ Core internal

For now, the in-process Pulse bus is the primary cross-crate vocabulary
inside `devix-core`. There is no separate envelope here. Cross-crate
boundaries that matter for late binding (editor ↔ plugin host) communicate
exclusively via Pulse subscriptions; boundaries with clear ownership and
synchronous semantics (editor ↔ syntax) are direct Rust calls.

The "internal lane" exists only insofar as supervised actors (the future
tree-sitter worker, future LSP client) interact with the rest of core via
Pulse + SubscriptionId, not raw method calls. Their behaviour is
documented per-actor in their own specs (out of scope here).

## Capability negotiation

Each lane's first message is a Hello carrying the producer's capability
set. The peer responds with Welcome carrying the **negotiated** set
(intersection of both sides).

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientHello {
    pub protocol_version: ProtocolVersion,
    pub pulse_bus_version: ProtocolVersion,
    pub manifest_version: ProtocolVersion,
    pub capabilities: HashSet<Capability>,
    pub client_name: String,    // "devix-tui", "devix-gui", etc.
    pub client_version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerWelcome {
    pub protocol_version: ProtocolVersion,
    pub pulse_bus_version: ProtocolVersion,
    pub manifest_version: ProtocolVersion,
    pub capabilities: HashSet<Capability>,
    pub session_id: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginHello { /* same fields as ClientHello */ }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginWelcome { /* same fields as ServerWelcome */ }

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    // ---- Frontend rendering ----
    /// Frontend understands the View IR base vocabulary.
    ViewTree,
    /// Frontend supports stable view-node ids for diffing / animation /
    /// focus continuity. Without this, views are repainted from scratch.
    StableViewIds,
    /// Frontend renders Unicode beyond the Basic Multilingual Plane
    /// (combining marks, emoji, CJK, etc.).
    UnicodeFull,
    /// Frontend renders 24-bit RGB colors. Without it, theme colors are
    /// quantized to 256/16-color palettes by the frontend.
    TruecolorStyles,
    /// Frontend handles transition / animation hints embedded in View IR
    /// (enter/exit phases, smooth scroll). Without it, hints are ignored.
    Animations,

    // ---- Plugin contributions (declarative) ----
    /// Plugin manifest may declare commands.
    ContributeCommands,
    /// Plugin manifest may declare keymap bindings.
    ContributeKeymaps,
    /// Plugin manifest may declare sidebar panes.
    ContributeSidebarPane,
    /// Plugin manifest may declare floating overlay panes (v1+).
    ContributeOverlayPane,
    /// Plugin manifest may declare status-line items (v1+).
    ContributeStatusItem,
    /// Plugin manifest may declare themes.
    ContributeThemes,
    /// Plugin manifest may declare settings keys.
    ContributeSettings,

    // ---- Plugin runtime API ----
    /// Plugin may subscribe to pulses.
    SubscribePulses,
    /// Plugin may invoke commands programmatically.
    InvokeCommands,
    /// Plugin may request file-open through the host.
    OpenPath,
    /// Plugin may enumerate the workspace filesystem (read_dir API).
    ReadDir,
}
```

The fine-grained shape lets a host advertise exactly what it supports. A
TUI client that doesn't yet implement overlay panes simply omits
`ContributeOverlayPane`; plugins requesting it see the negotiated set
miss it and degrade (or refuse to load, their choice). When overlay
panes ship, the bit gets advertised and existing plugins start working
without protocol or manifest version bumps — capabilities are the
forward-compat dimension.

### Negotiation rules

- **Major version mismatch** (`protocol_version.major != peer.major`) is a
  hard error. Peer responds with `ProtocolError::IncompatibleProtocol` and
  closes the lane.
- **Minor version mismatch** with same major: the lower of the two becomes
  the effective minor; both peers behave as if running that minor.
- **Capability subsetting**: the `capabilities` field on `Welcome` is the
  intersection. A plugin built against `ContributeCommands +
  ContributeSidebarPane` running on a host that only advertises
  `ContributeCommands` sees `Welcome.capabilities = {ContributeCommands}`.
  Pane registration calls become no-ops with a warning. The plugin can
  inspect the negotiated set after Welcome and decide to refuse-to-run
  rather than degrade — its choice.
- **`pulse_bus_version` and `manifest_version` mismatches** follow the same
  major-major / minor-min rule. A plugin compiled against pulse bus 0.3
  fails to load on pulse bus 0.2 (its required minor exceeds the host's).

### When negotiation happens

- Client ↔ Core: at TUI startup, before any other message.
- Plugin ↔ Core: at plugin load time, before any contribution message.
- Core ↔ Core internal: not at all (no envelope; same binary, same versions
  by construction).

## Protocol vs pulses

A pulse **is** a protocol message — `ClientToCore::Pulse` and
`PluginToCore::Pulse` carry the same `Pulse` enum that flows through the
in-process bus. The protocol layer adds an envelope so the message is
correlatable, versioned, and serializable.

This is the unification: there's no separate "protocol message type" and
"pulse type" hierarchy. The pulse bus is the in-process implementation of
the in-process lanes; the protocol envelope is what wraps a pulse when it
crosses a lane boundary that needs serialization.

In-process today, the pulse bus *is* the lane. There's no envelope at all
— components publish to the bus and the right thing happens. Envelopes
appear only when a transport is added; until then, the in-process bus is
sufficient.

## Request / response correlation

`Envelope.seq` correlates request to response. Every `Request` variant
pairs with the same-named `Response` variant; the response envelope's
`seq` matches the request envelope's `seq`.

```
seq=42 → ClientToCore::Request(View { root: "/pane/0" })
seq=42 ← CoreToClient::Response(View(Ok(ViewResponse { ... })))
```

For pulse-only messages (no expected response), `seq` is just for
ordering and log correlation.

`Request::InvokeCommand` carries a typed return value. Most commands
return `Ok(Value::Null)` — they're invoked for their side effects, which
flow back as separate pulses (`BufferChanged`, `CommandInvoked`, etc.).
Commands that produce a useful return (a search-result list, a
workspace-symbol set) put it in the `Response::InvokeCommand`'s
`Result::Ok(Value)`.

The pulse stream and the response stream are independent: a command may
publish pulses *before* its response arrives, and subscribers see them
in publish order without waiting on the response.

## Error model

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProtocolError {
    IncompatibleProtocol { ours: ProtocolVersion, theirs: ProtocolVersion },
    IncompatiblePulseBus { ours: ProtocolVersion, theirs: ProtocolVersion },
    IncompatibleManifest { ours: ProtocolVersion, theirs: ProtocolVersion },
    UnknownPath(Path),
    UnknownCommand(Path),
    UnknownSubscription(SubscriptionId),
    DeserializationFailure { what: String, detail: String },
    InternalError(String),
}
```

Most errors are non-fatal: receiver logs and continues.
`IncompatibleProtocol` and `IncompatiblePulseBus` are fatal — peer
disconnects from the lane after sending the error.

## In-process implementation today

Before any transport lands, the protocol is implemented as direct Rust
calls between crates plus the in-process bus:

```rust
pub trait FrontendHandle: Send + Sync {
    /// Core delivers a CoreToClient message to the attached frontend.
    fn deliver(&self, msg: CoreToClient);
}

pub trait PluginHandle: Send + Sync {
    /// Core delivers a CoreToPlugin message to the attached plugin.
    /// Mirrors `FrontendHandle::deliver` for the plugin lane.
    fn deliver(&self, msg: CoreToPlugin);
}

pub struct Core { /* ... */ }

impl Core {
    pub fn handle_client_message(&mut self, msg: ClientToCore);
    pub fn handle_plugin_message(&mut self, msg: PluginToCore);
    pub fn attach_frontend(&mut self, handle: Box<dyn FrontendHandle>);
    pub fn attach_plugin(&mut self, handle: Box<dyn PluginHandle>);
}
```

`CoreHandle` (in `frontend.md`) is the frontend-side handle for
submitting `ClientToCore` to core. There's no symmetric
plugin-side-handle trait: plugins submit `PluginToCore` via Lua-friendly
APIs (`devix.subscribe`, `devix.invoke_command`) that the host
translates into messages internally.

No serialization happens; envelopes are constructed but not turned into
bytes. The structure exists so that:

- Capability negotiation works the same regardless of transport.
- Adding a transport later is "wire up serde + a byte stream + a framing
  layer," not "redesign the message types."
- Tests drive Core through the same message types a real client would use.

Today's `devix-tui` binary creates a `Core`, attaches itself as a
`FrontendHandle`, optionally loads a plugin (which attaches as a
`PluginHandle`), then runs the input/render loop sending and receiving
messages.

## Versioning

`protocol_version` follows semver:

- Add a new variant to a lane enum: minor.
- Add a new field with `#[serde(default)]` to a struct: minor.
- Add a new `Capability`: minor (subsetting handles forward-compat).
- Rename / remove a variant or field: major.
- Change a field's type: major.

The `Pulse` enum and `Manifest` schema have their own versions (per
`pulse-bus.md` and `manifest.md`); all three are reported in Hello/Welcome.

## Interaction with other Stage-0 specs

- **`namespace.md`**: every protocol message uses `Path` for resource
  identity. No raw ids cross any lane.
- **`pulse-bus.md`**: pulses are wrapped in lane envelopes when they cross
  a lane boundary. In-process, the bus is the boundary; envelopes are
  conceptual.
- **`manifest.md`**: plugin manifests advertise the same versions and
  capabilities the protocol negotiates over.
- **`frontend.md`**: defines `View`, `InputEvent`, and the
  `FrontendHandle` trait. Lane 1 messages reference `View` directly.
- **`crates.md`**: every type in this spec lives in `devix-protocol`.
  `FrontendHandle` and `PluginHandle` traits also live there so consumers
  in `devix-core` and `devix-tui` reach for the same surface.

## Open questions

1. **Streaming responses.** Some commands (search results, file
   enumeration, workspace symbol lookup) return many items. Single batch
   payload vs streaming chunks. Lean: defer until streaming use case
   appears; v0 batches into one `Response::InvokeCommand(Ok(...))`. Add
   a `Stream` variant later if needed.

2. ~~**Plugin capability mismatches.**~~ *Resolved during T-32
   (2026-05-07): warn-and-degrade with plugin opt-out (VS Code
   style). Unsupported contributions silently no-op with a
   `Pulse::PluginError` warning; the plugin inspects the negotiated
   capability set on `Welcome` and decides for itself whether to
   keep running. See amendment log.*

3. **Internal lane formalization.** Today the "internal lane" is just
   "use the bus + direct calls." Should supervised actors (tree-sitter
   worker, future LSP) get their own envelope-bound mailbox so they can
   OOB-control the supervisor? Lean: only when an actor needs control
   messages beyond what pulses carry. Tree-sitter worker can probably
   make do with pulses plus a direct call into the supervisor for
   restart.

4. **Transport framing.** Out of scope for this spec (in-process only),
   but when the wire transport ships, what framing? JSON-RPC-style
   `Content-Length: N\r\n\r\n<json>` (LSP), length-prefixed binary,
   msgpack-framed serde. Lean: punt to transport-spec; design today
   doesn't constrain.

5. **Session lifecycle pulses.** Should `Hello` / `Welcome` / `Goodbye`
   emit pulses? `PluginLoaded` / `PluginUnloaded` are already in the v0
   catalog; `ClientConnected` is not. Lean: add `ClientConnected` /
   `ClientDisconnected` to the pulse catalog so subscribers can react.
   Folds in during T-21 when the bus and the protocol skeleton land
   together.

6. ~~**`PathKind` for `Request::ListPaths`.**~~ *Resolved during T-32
   (2026-05-07): seven variants matching every Stage-5 namespace
   migration target — `Buffer`, `Cursor`, `Pane`, `Command`,
   `Keymap`, `Theme`, `Plugin`. (Note: `Pane`, not `Frame`, per the
   post-Stage-9 layout vocabulary; `Sidebar` is dropped because it's
   a sub-path of `/pane`, not a registry root.) See amendment log.*

## Resolved during initial review

- Typed Response messages → adopted. `Request` and `Response` enums pair
  by variant; `Envelope.seq` correlates. Pulses still flow for events;
  Response is for typed return values.
- Capability granularity → fine-grained from day one. The catalog covers
  frontend rendering bits, plugin contribution bits, and plugin runtime
  API bits as separate capabilities. New features ship by adding bits
  without bumping protocol or manifest versions.
