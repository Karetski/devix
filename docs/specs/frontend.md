# devix — Frontend protocol & View IR spec

Status: working draft. Stage-0 foundation T-05.

## Purpose

Define the View IR and the input event normalization the frontend
protocol uses. This is the contract a frontend (TUI today; GUI / mobile /
web later) implements to render core's output and dispatch input back.

This spec answers two principles:
- **MLIR**: one render abstraction (View IR), not parallel paint
  hierarchies per frontend kind.
- **LSP**: a narrow versioned protocol with negotiated capabilities — same
  spec, applied to the rendering surface.

## Scope

This spec covers:
- The `View` closed enum (the IR).
- `ViewNodeId` semantics for diffing / animation / focus continuity.
- `InputEvent`, `Chord`, modifiers, mouse, scroll.
- `FrontendHandle` and `CoreHandle` traits.
- Animation hints (gated on `Capability::Animations`).
- Theme integration (resolved styles vs scope names).
- Frontend lifecycle around a session.

This spec does **not** cover:
- TUI-specific paint code. The ratatui interpreter lives in `devix-tui`.
- GUI-specific paint code. A future `devix-gui` crate gets its own.
- Layout / virtualization. Per the locked decision, `LinearLayout` /
  `UniformLayout` / scroll cell math live in `devix-tui`. Core emits
  logical positions; the frontend chooses how to virtualize.
- The wire transport (in-process today; future transport spec).

## View IR

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum View {
    /// Empty placeholder; renders nothing.
    Empty,

    /// Styled text run. Single line; explicit newlines are not embedded —
    /// multi-line text uses Stack of Text nodes or List.
    Text {
        id: ViewNodeId,
        spans: Vec<TextSpan>,
        wrap: WrapMode,
        transition: Option<TransitionHint>,
    },

    /// Vertical or horizontal stack of children with proportional weights.
    Stack {
        id: ViewNodeId,
        axis: Axis,
        weights: Vec<u16>,
        children: Vec<View>,
        spacing: u32,
        transition: Option<TransitionHint>,
    },

    /// Top-to-bottom list of items; one item per logical line.
    /// Frontends virtualize (paint only visible items). Each item is a
    /// View, so styling and structure are arbitrary.
    List {
        id: ViewNodeId,
        items: Vec<View>,
        item_keys: Vec<ViewNodeId>,
        selected: Option<u32>,
        transition: Option<TransitionHint>,
    },

    /// Document body (a text buffer). Frontend handles virtualization,
    /// horizontal scroll, gutter rendering. Core publishes
    /// ViewportChanged in response to the frontend's scroll.
    Buffer {
        id: ViewNodeId,
        path: Path,
        scroll_top_line: u32,
        cursor: Option<CursorMark>,
        selection: Vec<SelectionMark>,
        highlights: Vec<HighlightSpan>,
        gutter: GutterMode,
        active: bool,
        transition: Option<TransitionHint>,
    },

    /// Tab strip. Frontend lays out + scrolls + hit-tests; just receives
    /// the tab data and the active index. (Tab-cell transitions are
    /// internal to the strip's renderer; no per-strip TransitionHint.)
    TabStrip {
        id: ViewNodeId,
        tabs: Vec<TabItem>,
        active: u32,
    },

    /// Sidebar with title + content. Content is itself a View — built-ins
    /// give a List (file tree, outline); plugins give whatever.
    Sidebar {
        id: ViewNodeId,
        slot: SidebarSlot,
        title: String,
        focused: bool,
        content: Box<View>,
        transition: Option<TransitionHint>,
    },

    /// Layout split. Children rendered side-by-side along the axis.
    Split {
        id: ViewNodeId,
        axis: Axis,
        weights: Vec<u16>,
        children: Vec<View>,
        transition: Option<TransitionHint>,
    },

    /// Floating overlay anchored to a cell (popup, hover, completion).
    Popup {
        id: ViewNodeId,
        anchor: Anchor,
        content: Box<View>,
        max_size: Option<(u16, u16)>,
        chrome: PopupChrome,
        transition: Option<TransitionHint>,
    },

    /// Centered modal (palette, picker). Z-top in its frame.
    Modal {
        id: ViewNodeId,
        title: String,
        content: Box<View>,
        transition: Option<TransitionHint>,
    },
}
```

### Supporting types

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextSpan {
    pub text: String,
    pub style: Style,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WrapMode { Wrap, NoWrap, Truncate }

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis { Horizontal, Vertical }

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarSlot { Left, Right }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TabItem {
    pub id: ViewNodeId,
    pub label: String,
    pub dirty: bool,
    pub doc: Path,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Anchor { pub col: u16, pub row: u16, pub edge: AnchorEdge }

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorEdge { Above, Below, Left, Right }

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PopupChrome { Bordered, Borderless }

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GutterMode { LineNumbers, None }

/// Position of the primary caret (the one the OS-level cursor /
/// terminal caret is drawn at). `View::Buffer` carries one of these
/// via `cursor: Option<CursorMark>`; secondary multicursor carets are
/// rendered as zero-extent `SelectionMark`s in `selection`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct CursorMark { pub line: u32, pub col: u32 }

/// One selection range. Point cursors (multicursor secondaries) appear
/// as zero-extent marks where start == end.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SelectionMark {
    pub start_line: u32, pub start_col: u32,
    pub end_line: u32,   pub end_col: u32,
}

// HighlightSpan re-exported from devix-syntax through devix-protocol.
```

## ViewNodeId

```rust
#[derive(Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
pub struct ViewNodeId(pub Path);
```

Every View node carries a stable id. Identity is a `Path`:

- **Resource-bound** nodes use the resource's canonical path:
  - `View::Buffer` → `/buf/42`
  - `View::TabStrip` for frame 0 → `/pane/0/tabstrip`
  - `View::Sidebar` left → `/pane/0/sidebar/left`
- **Synthetic** nodes (a Stack with no resource backing, an internal
  Modal wrapper) use `/synthetic/<kind>/<minted-id>` where `<minted-id>`
  is a process-monotonic counter. Synthetic ids must be **stable across
  renders for the same logical node**. Two implementation strategies
  satisfy that contract:
  1. *Mint-and-cache*: when core decides to emit a synthetic node, it
     consults a per-parent cache keyed by the node's structural
     position (e.g., "child-index 2 of /pane/0/sidebar/left/content")
     and reuses the previously-minted id if one exists. Mints fresh
     and caches when the position is unseen.
  2. *Deterministic derivation*: the synthetic id is derived from the
     parent's id + the child's structural slot
     (`/synthetic/stack/<parent-path>/<child-index>`). No state needed,
     but the path leaks structure.
  The contract is implementation-agnostic; T-71 picks one. The
  invariant is: same logical node across two renders ⇒ same id.

The frontend uses ids for:

- **Diffing across renders**: same id → "same logical node, possibly with
  changed style or position." Different id → "different node."
- **Animation continuity**: an id that persists across renders is the
  same animatable element; transition hints reference it implicitly.
- **Focus continuity**: a node that holds focus retains it on the next
  render if its id is still present in the tree.
- **Accessibility**: stable identity for the a11y tree the frontend
  exposes to screen readers.

The diff algorithm itself is **frontend-defined**. Core makes one
guarantee: same `ViewNodeId` across renders means "same logical node."
Different id means "different node, free to animate as enter/exit."

## Animation hints

Gated on `Capability::Animations`. When a frontend advertises it, core
populates the `transition` field on supporting View variants:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransitionHint {
    pub kind: TransitionKind,
    pub duration_ms: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    /// Node is appearing in this render (was absent before).
    Enter,
    /// Node is exiting (was present before; the frontend should keep
    /// rendering it during the transition window).
    Exit,
    /// Node persists; its position or style changed; the frontend may
    /// animate the transition.
    Move,
}
```

When the frontend doesn't advertise `Animations`, core sets every
`transition` to `None`. The frontend never sees a hint it can't handle.

`Exit` hints are special: they describe a node that is *no longer
present in the tree*. Core synthesizes a "phantom" View for one frame
with `Exit` set so the frontend can animate the disappearance, then
omits it on subsequent frames. (Implementation detail; spec just
guarantees the contract.)

## Style

Themes are resolved by core; the View IR carries concrete colors and
modifiers, not scope names. A frontend with `Capability::TruecolorStyles`
gets RGB; a frontend without may receive RGB anyway and quantize on
receipt (TUI's responsibility, not core's).

```rust
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub dim: bool,
    pub reverse: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum Color {
    /// Use the terminal/desktop default (caller decides).
    Default,
    /// 24-bit RGB. Quantized by the frontend if it lacks truecolor.
    Rgb(u8, u8, u8),
    /// 256-color indexed; for terminals.
    Indexed(u8),
    /// Named: black, white, red, green, blue, yellow, magenta, cyan,
    /// dark_gray, light_red, ... — VT100-equivalents.
    Named(NamedColor),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamedColor {
    Black, Red, Green, Yellow, Blue, Magenta, Cyan, White,
    DarkGray, LightRed, LightGreen, LightYellow, LightBlue,
    LightMagenta, LightCyan,
}
```

### Color serialization

`Color` has a custom `Serialize` / `Deserialize` impl (not derived) so
hand-written JSON manifests stay ergonomic. The wire form is **a
single string**:

| String form | Variant |
|---|---|
| `"default"` | `Color::Default` |
| `"#rrggbb"` (lowercase or uppercase hex) | `Color::Rgb(r, g, b)` |
| `"@<n>"` where `0 ≤ n ≤ 255` (e.g., `"@42"`) | `Color::Indexed(n)` |
| `"red"`, `"green"`, `"black"`, ... (snake_case `NamedColor`) | `Color::Named(...)` |

So a manifest theme writes:

```json
{ "fg": "#aaaaaa", "bold": true }
{ "fg": "red", "italic": true }
{ "fg": "@8", "bg": "default" }
```

Deserialization rejects anything that doesn't fit one of the four
patterns. There's no structured `{"kind": "rgb", ...}` form on the
wire — the string is canonical. The Rust enum is what consumers match
against in code; the string form is what crosses serde.

The rationale: theme manifests are hand-edited often; structured forms
are tedious. Programmatic consumers (the future Settings UI writing
back a customized theme) emit the same string form via the custom
`Serialize` impl.

## InputEvent

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputEvent {
    Key {
        chord: Chord,
        text: Option<char>,
    },
    Mouse {
        x: u16,
        y: u16,
        button: Option<MouseButton>,
        press: MouseKind,
        modifiers: Modifiers,
    },
    Scroll {
        x: u16,
        y: u16,
        delta_x: i32,
        delta_y: i32,
        modifiers: Modifiers,
    },
    Paste(String),
    FocusGained,
    FocusLost,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseKind { Down, Up, Drag, Move }

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton { Left, Right, Middle, Back, Forward }

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    #[serde(rename = "super")]
    pub super_key: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Chord {
    pub key: KeyCode,
    pub modifiers: Modifiers,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum KeyCode {
    Char(char),
    Enter, Tab, BackTab, Esc, Backspace, Delete, Insert,
    Left, Right, Up, Down, Home, End, PageUp, PageDown,
    F(u8),
}
```

### Chord serialization

`Chord` and `KeyCode` have **custom `Serialize` / `Deserialize` impls**
(not derived). The wire form is the **canonical kebab-case string**
defined in `namespace.md`:

```
ctrl-s
ctrl-shift-p
alt-left
shift-tab
f12
```

So a manifest keymap entry writes:

```json
{ "key": "ctrl-shift-p", "command": "palette.open" }
```

A pulse payload carrying a chord (`InputEvent::Key { chord, .. }`)
serializes the same way — the chord is always a string on the wire,
whether it appears in a JSON manifest or a pulse payload. The `KeyCode`
enum exists so Rust code matches on it ergonomically; outside Rust it's
the chord-string segment after the modifier prefix.

Deserialization rejects strings that don't parse against the chord
grammar (`<modifier>-...-<key>` with modifier order ctrl-alt-shift-super
when present, lowercase). Validation happens at deserialize time, not
later — a malformed chord in a manifest fails the manifest validator;
a malformed chord in a pulse fails `DeserializationFailure`.

Coordinates:
- TUI: `x` / `y` are cell column / row (`u16` matches ratatui's `Rect`).
  `delta_y` for scroll is in lines; `delta_x` in columns.
- GUI: `x` / `y` are pixel coordinates within the window; the frontend
  reports its viewport in `Pulse::ViewportChanged` so core can translate
  back to logical positions when needed (e.g., click → buffer position).
  Scroll deltas are in pixels.

The frontend chooses; core's interpretation depends on the frontend's
reported viewport semantics in `ViewportChanged`. There is no "TUI mode"
flag — coordinate space is a function of what the frontend publishes.

`Chord` matches the canonical kebab-case form defined in
`namespace.md` when serialized to a `/keymap/<chord>` path. The struct
form here is what the bus and the manifest deserialize into.

## Handles

```rust
pub trait FrontendHandle: Send + Sync {
    fn deliver(&self, msg: CoreToClient);
}

pub trait CoreHandle: Send + Sync {
    fn submit(&self, msg: ClientToCore);
}
```

`devix-tui` implements `FrontendHandle` (receives `CoreToClient` and
renders) and holds a `CoreHandle` clone (submits inputs and requests
views). Core implements `CoreHandle` and holds the attached
`FrontendHandle`.

Future transports get adapter implementations: a stdio `CoreHandle`
wraps stdout writes; a stdio `FrontendHandle` wraps stdin reads. Same
core code, different shipping.

## Buffer rendering specifics

`View::Buffer` is the most complex variant; it deserves its own
contract:

- `path` identifies the document being rendered.
- `scroll_top_line` is the logical line index at the top of the
  viewport. Core reports it; the frontend honors it (no fractional
  pixel scroll on TUI; GUI may interpolate).
- `cursor` is the primary cursor's position; `selection` is the
  selection ranges. The frontend renders these as it sees fit (block,
  vertical bar, underline; highlighted region or animated marker).
- `highlights` is the tree-sitter highlight span list for the visible
  byte range. Core has already applied theme resolution to
  HighlightSpan's scope name (mapping it through the active theme); see
  *Style* — actually, **highlights still carry scope names**, not
  resolved styles, because the same `View::Buffer` is rendered
  identically across themes and we'd repeatedly re-emit the same view
  on theme changes if styles were inlined. The frontend interprets scope
  names against the active palette delivered via
  `Pulse::ThemeChanged { palette }`. (See *Resolved during initial
  review*.)
- `gutter` is `LineNumbers | None`. The frontend's interpreter knows
  how to render line numbers; core just signals whether to show them.
- `active` indicates whether this buffer view should claim the visible
  cursor (multiple buffers in a split — only one shows the caret).

## Lifecycle

Frontend lifecycle around a session:

1. Frontend dials core (in-process: instantiate `Core`; out-of-process:
   open a transport).
2. Frontend sends `ClientToCore::Hello(ClientHello)` carrying its
   capabilities.
3. Core responds `CoreToClient::Welcome(ServerWelcome)` with the
   negotiated capability set.
4. Frontend subscribes to pulses it cares about (typically
   `RenderDirty`, `ThemeChanged`, `BufferChanged` for active buffers,
   `FocusChanged`, `PluginError`) by sending
   `ClientToCore::Subscribe { id, filter }` — same mechanism plugins
   use, applied to the client lane. Core delivers matching pulses as
   `CoreToClient::Pulse`. Frontend can later
   `Unsubscribe { id }` to drop a subscription.
5. Frontend sends `Request::View { root: "/pane" }`; core responds with
   the initial View.
6. Frontend paints; loops on input.
7. On input: frontend sends `Pulse::InputReceived { event }`; core
   processes; pulses fire; if any are `RenderDirty`, frontend
   re-requests view.
8. On viewport change (resize, scroll): frontend sends
   `Pulse::ViewportChanged`.
9. Shutdown: frontend sends `ClientToCore::Goodbye`; core closes the
   session and releases per-frontend state.

## Interaction with other Stage-0 specs

- **`namespace.md`**: every `ViewNodeId` is a `Path`. Resource-bound
  nodes use the resource's canonical path; synthetic nodes use
  `/synthetic/...`. Frontend never invents paths — it only echoes ones
  core sent.
- **`pulse-bus.md`**: defines `Axis`, the InputEvent payload of
  `Pulse::InputReceived`, and the `ViewportChanged` payload. All three
  shapes are defined here and re-exported through pulse-bus.md.
- **`protocol.md`**: `Request::View` returns `ViewResponse { view, version, root }`.
  `Capability::ViewTree`, `StableViewIds`, `UnicodeFull`,
  `TruecolorStyles`, `Animations` gate features described above.
- **`manifest.md`**: themes deserialize into the `Style` palette. Plugin
  panes' `lua_handle` returns a Lua-shaped View tree; the host
  marshals it into `View`.
- **`crates.md`**: every type in this spec lives in `devix-protocol`.

## Open questions

1. **`SidebarSlot::Floating`?** Today the slot enum is `Left | Right`.
   Adding overlay-pane support (`Capability::ContributeOverlayPane`)
   may want a `Floating { anchor }` slot — or overlay panes use
   `View::Popup` directly without going through Sidebar at all.
   Lean: overlay panes are top-level `View::Popup` siblings, not
   sidebar slots. Confirm during T-71.

2. **List virtualization windowing.** Huge lists (workspace search,
   global symbols) shouldn't ship 50,000 items in one View. Either
   add `View::List { items_window: Option<(start, end)>, total_count }`
   or a separate `Request::ListPage { id, offset, limit }`. Lean: add
   a windowed variant when the first big-list use case ships;
   defer for v0.

3. **Z-order.** `Popup` and `Modal` paint above their sibling tree by
   construction (frontend renders them last). Multiple modals: explicit
   z-index field, or order in the parent's children? Lean: order by
   tree position; modals rarely stack.

4. **Drag-and-drop, file-drop, IME composition.** TUI doesn't need
   them; GUI does. Defer to a v1 InputEvent expansion.

5. **`Chord::text` ambiguity.** A keypress like `shift-a` produces text
   `'A'`. Today's draft has both `chord` and `text` on `Key`. Should
   `text` *always* be set when the press produces a printable char, or
   only when no chord matched? Lean: always set when printable; the
   keymap layer can ignore `text` when it dispatches via `chord`.

## Resolved during initial review

- Frontend pulse subscription → explicit `ClientToCore::Subscribe` /
  `Unsubscribe` (same shape as the plugin lane). Frontend opts into the
  pulse kinds it cares about; core delivers as `CoreToClient::Pulse`.
- Buffer highlights → scope names (not pre-resolved styles). Theme is
  shipped separately as a palette via `Pulse::ThemeChanged { palette }`;
  the frontend interprets scope names against the active palette.
  ThemeChanged does not invalidate cached `View::Buffer` trees.