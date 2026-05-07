# devix — Namespace spec

Status: working draft. Stage-0 foundation T-00.

## Purpose

The unified addressing surface for every resource in devix. Replaces the eleven
ad-hoc registries identified in the principles audit (documents, cursors,
frames, sidebars, commands, chords, theme scopes, plugin callbacks, etc.) with
one path-shaped naming convention and one `Lookup` trait that every registry
implements.

This spec answers Plan-9's principle: *every resource is a node in a
hierarchical namespace, accessed through one interface.* The namespace is
local-process today; if/when we add an out-of-process transport, the same path
syntax extends to URIs (`devix:///buf/42`).

## Scope

This spec covers:
- The path grammar.
- Canonical resource roots (`/buf/...`, `/cmd/...`, etc.).
- The `Lookup` trait every registry implements.
- The `Path` type and its API.
- Migration mapping from today's typed-id registries to paths.

This spec does **not** cover:
- Capability negotiation (lives in `protocol.md`).
- Pulse subjects or subscriptions (lives in `pulse-bus.md`).
- Resource-specific semantics (each resource type's own spec).

## Path grammar

```
Path     := "/" Segment ("/" Segment)*
Segment  := SegChar+
SegChar  := ALPHA | DIGIT | "-" | "_" | "."
```

Rules:
- Separator is `/`.
- Segments use ASCII word chars, digits, `-`, `_`, `.`. Other chars (whitespace,
  `:`, `*`, `?`, non-ASCII) are reserved.
- Empty segments are not allowed (no `//`).
- Leading `/` is required (paths are absolute).
- Trailing `/` is not allowed.
- `.` and `..` are not interpreted as relative-path tokens — they are valid
  characters inside a segment but never given filesystem-style meaning.

Dotted forms within a segment are preserved deliberately. They let
VS-Code-style command ids (`edit.copy`) and tree-sitter scope dots
(`keyword.control`) embed into one segment without confusing the segment
boundary.

### Examples

```
/buf/42
/cur/3
/pane
/pane/0/1
/cmd/edit.copy
/keymap/ctrl-s
/keymap/ctrl-shift-p
/theme/keyword.control
/plugin/file-tree
/plugin/file-tree/cmd/refresh
/plugin/file-tree/pane/main
```

## Canonical resource roots

| Root | Resource | Examples |
|---|---|---|
| `/buf/<DocId>` | Document | `/buf/42` |
| `/cur/<CursorId>` | Cursor | `/cur/7` |
| `/pane` | Layout root | `/pane` |
| `/pane/<i>(/<j>)*` | Pane at child path | `/pane/0`, `/pane/0/1` |
| `/cmd/<dotted-id>` | Command | `/cmd/edit.copy` |
| `/keymap/<chord>` | Keymap binding | `/keymap/ctrl-s`, `/keymap/ctrl-shift-p` |
| `/theme/<scope>` | Theme style entry | `/theme/keyword.control` |
| `/plugin/<name>` | Plugin namespace root | `/plugin/file-tree` |
| `/plugin/<name>/cmd/<id>` | Plugin-contributed command | `/plugin/file-tree/cmd/refresh` |
| `/plugin/<name>/pane/<id>` | Plugin-contributed pane | `/plugin/file-tree/pane/main` |
| `/plugin/<name>/cb/<handle>` | Plugin Lua callback | `/plugin/file-tree/cb/12` |
| `/synthetic/<kind>/<id>` | Synthetic view-node id (no resource backing) | `/synthetic/stack/42`, `/synthetic/modal/7` |

These roots are *conventions*, not enforced by the namespace machinery. A
custom registry can mount under its own root if the audit allows it (e.g.,
`/lsp/...` once LSP lands).

## Segment encoding rules

### Resource ids (`/buf/<id>`, `/cur/<id>`, `/pane/.../<id>`, plugin handles)

Path-facing ids are minted from a **process-monotonic counter** — a global
`AtomicU64` that increments per resource creation across the session. The
internal storage stays a `SlotMap` (or its equivalent); the slot key is *not*
exposed in paths.

The current `FrameId` (`crates/editor/src/frame.rs:18-25`) already follows
this shape and is the model. `DocId` and `CursorId` will be reshaped during
T-30/T-31 to mint path-facing ids the same way; the underlying
`slotmap::DefaultKey` becomes a private detail of each store.

Property: a path like `/buf/42` *never names two different buffers in one
session*, even if buffer 42 is closed and a new buffer is opened. The new
buffer gets `/buf/43`. Paths are stable references; stale paths return `None`
from `Lookup::lookup`, never a different resource.

### Chord segments (`/keymap/<chord>`)

Canonical form is **kebab-case** lowercase modifiers + key, hyphen-separated:

```
ctrl-s
ctrl-shift-p
alt-left
ctrl-alt-down
shift-tab
```

Modifier order is fixed (when present): `ctrl`, `alt`, `shift`, `super`. The
key segment is last. Letter keys are lowercased. Named keys use lowercase
words: `enter`, `tab`, `esc`, `backspace`, `delete`, `home`, `end`, `pageup`,
`pagedown`, `left`, `right`, `up`, `down`, `f1`...`f12`.

Plugins write the same form in JSON manifests:

```json
{
  "command": "/plugin/file-tree/cmd/refresh",
  "key": "ctrl-shift-r"
}
```

The chord parser (today: `crates/plugin/src/lib.rs:1180-1235`) is updated to
accept this canonical form. The display renderer (today:
`format_chord` → `Ctrl+Shift+P`) is unaffected — display is a TUI concern.

### Dotted segments (`/cmd/edit.copy`, `/theme/keyword.control`)

Dots are valid inside a segment (see grammar). They carry namespace
information internal to the segment without affecting path traversal. So
`/theme/keyword.control` is a single-segment path under `/theme`, not a
two-level walk.

## The `Lookup` trait

```rust
pub trait Lookup {
    /// The resource type this registry serves.
    type Resource: ?Sized;

    /// Resolve `path` to a borrow of the resource it names, or `None` if no
    /// such resource exists at this path inside this registry.
    fn lookup(&self, path: &Path) -> Option<&Self::Resource>;

    /// Mutable variant.
    fn lookup_mut(&mut self, path: &Path) -> Option<&mut Self::Resource>;

    /// Iterate every path this registry currently holds. Order is
    /// implementation-defined; consumers use this to enumerate resources of a
    /// kind (e.g., the palette listing every `/cmd/...`).
    fn paths(&self) -> Box<dyn Iterator<Item = Path> + '_>;
}
```

The trait is **local to a registry**: there is no global
`lookup_anything(path)` that walks across resource kinds. A `BufferStore` is
`Lookup<Resource = Document>`; a `CommandRegistry` is
`Lookup<Resource = Command>`. Consumers know which registry they're addressing.

This is intentional: a global multi-resource lookup would force every consumer
to handle "this path could resolve to a Document, or a Cursor, or a Theme
entry" with type erasure. Local lookups keep the type information.

A future `Workspace` aggregator can compose lookups (`workspace.buffer(path)`,
`workspace.command(path)`) without the namespace itself needing to know about
the aggregation.

## The `Path` type

```rust
#[derive(Clone, Eq, PartialEq, Hash)]
pub struct Path(Arc<str>);

impl Path {
    /// Parse a string into a Path. Returns Err on grammar violations.
    pub fn parse(s: &str) -> Result<Self, PathError>;

    /// Iterate segments (without the leading slash).
    pub fn segments(&self) -> impl Iterator<Item = &str>;

    /// First segment (e.g., "buf" for "/buf/42"). Always present — every Path
    /// has at least one segment by grammar.
    pub fn root(&self) -> &str;

    /// Parent path, or None if this is a single-segment path
    /// (single-segment paths have no parent because `/` is not a valid path).
    pub fn parent(&self) -> Option<Path>;

    /// Append a segment. Returns Err if `segment` violates the segment grammar.
    pub fn join(&self, segment: &str) -> Result<Path, PathError>;

    /// True if `self`'s segment sequence starts with `other`'s. Used by
    /// `PulseFilter::path_prefix` to test "is this path under that
    /// prefix?" The check is segment-aware, not byte-level — `/buf/4`
    /// does *not* start with `/buf/42`.
    pub fn starts_with(&self, other: &Path) -> bool;

    /// Borrow as &str (canonical form, leading slash, no trailing slash).
    pub fn as_str(&self) -> &str;
}

#[derive(Debug, thiserror::Error)]
pub enum PathError {
    /// Input string was empty (`""`).
    #[error("path is empty")]
    Empty,
    /// Input did not start with `/` (e.g., `"buf/42"`).
    #[error("path must start with `/`")]
    MissingLeadingSlash,
    /// Input ended with `/` (e.g., `"/buf/42/"`).
    #[error("path must not end with `/`")]
    TrailingSlash,
    /// At least one segment between separators was empty
    /// (e.g., `"/"` has no segment after the leading slash;
    /// `"/buf//42"` has an empty segment between two slashes).
    #[error("empty segment in path")]
    EmptySegment,
    /// A segment contained a reserved character (whitespace, `:`,
    /// `*`, `?`, non-ASCII, etc.).
    #[error("segment `{0}` contains reserved character")]
    InvalidSegment(String),
}
```

The single-slash path `"/"` is rejected as `EmptySegment` (the segment
after the leading slash is empty); see grammar.

`Arc<str>` makes cloning cheap (two atomic ops); paths are passed around a
lot, especially in pulse payloads and plugin callbacks. The `Hash` impl uses
the canonical string form, so `Path` is usable as a `HashMap` key.

`Path` is `Serialize + Deserialize` (serde from day one, locked). Both go
through the canonical string form.

## Migration table

The audit listed eleven ad-hoc registries. Here's how each maps to the new
namespace.

| Today (lookup expression) | Path root | Notes |
|---|---|---|
| `documents.get(DocId)` | `/buf/<id>` | Path-facing id is a process-monotonic u64; slotmap is internal storage only |
| `cursors.get(CursorId)` | `/cur/<id>` | Same pattern |
| `doc_index.get(&PathBuf)` | (kept as resolver) | Filesystem path → DocId; not a path-keyed registry itself |
| `LayoutNode::at_path(&[usize])` | `/pane(/<i>)*` | Index list → string segments |
| `frame_rects.get(&FrameId)` | (moves to `devix-tui`) | TUI hit-test cache, not core state |
| `sidebar_rects.get(&SidebarSlot)` | (moves to `devix-tui`) | TUI hit-test cache |
| `tab_strips.get(&FrameId)` | (moves to `devix-tui`) | TUI render cache |
| `CommandRegistry::by_id(CommandId)` | `/cmd/<dotted-id>` | CommandId becomes a typed wrapper around a Path |
| `Keymap::bindings(Chord)` | `/keymap/<chord>` | Chord serialized as kebab-case (`ctrl-s`, `ctrl-shift-p`) |
| `Theme::scopes(&str)` | `/theme/<scope>` | Existing dotted scope syntax preserved |
| Plugin `callbacks(u64)` | `/plugin/<name>/cb/<u64>` | Per-plugin namespace |

The render cache in `RenderCache` (frame_rects / sidebar_rects / tab_strips)
moves out of core entirely — it's a TUI concern. Core exposes the layout tree
via `/pane/...`; the TUI client maintains its own rect cache, keyed by `Path`,
not by `FrameId` / `SidebarSlot`.

## Versioning

Paths are version-free. A path always names the *current* resource at that
location. Resource-content versioning (e.g., `Document::revision`) is carried
by the resource type itself, not the path.

If versioned addressing is ever needed, the segment grammar will be extended
to allow `@<key>/<value>` suffixes (e.g., `/buf/42@rev/100`). Not in v1.

## Interaction with other Stage-0 specs

- **`pulse-bus.md`**: pulses carry paths to identify which resource changed.
  `Pulse::BufferChanged { path: Path, .. }` rather than
  `Pulse::BufferChanged { doc_id: DocId, .. }`. Plugins subscribe on
  paths/path-prefixes; the pulse bus relies on `Path::Hash`.
- **`protocol.md`**: paths cross the wire as their canonical string form.
  Capability negotiation may include "this side understands these path
  prefixes" but that lives in the protocol spec, not here.
- **`manifest.md`**: contribution declarations name commands and panes by
  path. A plugin contributing a command writes
  `"command": "/plugin/file-tree/cmd/refresh"` in its manifest, or just
  `"command": "refresh"` and the loader prepends `/plugin/<name>/cmd/`.
- **`frontend.md`**: View-IR nodes carry stable ids; ids are paths for any
  resource-bound node (a tab strip's tab cells use `/pane/0/tab/3` paths so
  reorders are diffable).
- **`crates.md`**: `Path`, `PathError`, `Lookup` live in `devix-protocol` (so
  it's the lowest layer above `devix-text`).

## Open questions

1. **`lookup_mut` and the borrow checker.** Two simultaneous `lookup_mut`
   calls on the same store conflict. Multi-resource ops (split frames,
   open-path-replace-current) need disjoint borrows. Options:
   - `lookup_two_mut(p1, p2) -> Option<(&mut R, &mut R)>` for the two-paths
     case.
   - Per-registry split-borrow helpers (`buffer_store.split(p1, p2)`).
   - Defer: keep direct slotmap access on the store types for ops that
     genuinely need disjoint mutable borrows; `Lookup` is for single-resource
     access.
   Decide during T-30.

2. **Globbing / patterns.** Does `/plugin/file-tree/*` enumerate resources
   under the prefix? Useful for "list all panes contributed by plugin X" or
   "drop every callback when plugin unloads." Options:
   - Add `Lookup::children(&Path) -> impl Iterator<Item = Path>`.
   - Defer; `paths()` + caller-side prefix filter is enough until a real use
     case appears.
   Lean: defer.

3. **Path → typed-id round trip.** `Path::parse("/buf/42")` should give back
   a `DocId`. Per-root parsers (`Document::id_from_path(&Path)`) are the
   shape; `Path` itself stays untyped. Confirm in T-30.

4. **Serde canonical form.** `Path` derives serde. `Serialize` writes the
   canonical string form; `Deserialize` calls `Path::parse`. No alternative
   wire form. Confirm — anyone want a structured (segment-array) wire form?
   Lean: no.

## Resolved during initial review

- Path-facing id encoding → process-monotonic counter (slotmap stays
  internal); see *Segment encoding rules → Resource ids*.
- Chord encoding → kebab-case lowercase (`ctrl-shift-p`); see
  *Segment encoding rules → Chord segments*.
- Empty path `/` → forbidden; layout root is `/pane`. Grammar requires
  ≥1 segment.
