# devix — Manifest spec

Status: working draft. Stage-0 foundation T-03.

## Purpose

JSON schema for plugin manifests **and** built-in contributions. The
manifest is the declarative source of truth for what a plugin or built-in
subsystem contributes — commands, keymaps, themes, panes, settings, pulse
subscriptions.

This spec answers VS Code's principle: *extensions describe what they add
declaratively in a manifest; the host wires and lazily activates them.* And
it unifies built-ins with plugins under the same schema, so the palette,
settings UI, and keymap configuration all read from one shape.

## Scope

This spec covers:
- Manifest JSON schema (top-level fields).
- The `contributes` shape (commands, keymaps, themes, panes, settings).
- The `subscribe` shape (PulseFilter list).
- The `engines` block (subsystem version requirements).
- How built-ins use the same manifest format.
- Manifest discovery, loading, and validation.

This spec does **not** cover:
- Activation events. Deferred per locked decision; v0 plugins always load
  eagerly. The schema reserves the field name for future use.
- Concrete `Pulse` variants (`pulse-bus.md`).
- View IR types (`frontend.md`).
- Plugin runtime / Lua sandbox details — that's a separate runtime spec.

## Rust types

The schema deserializes into the following Rust structs (definitions
abbreviated for clarity; full impls in `devix-protocol::manifest`):

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    pub version: String,           // semver
    pub engines: Engines,
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(default)]
    pub contributes: Contributes,
    #[serde(default)]
    pub subscribe: Vec<SubscriptionSpec>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Engines {
    pub devix: ProtocolVersion,        // protocol_version
    pub pulse_bus: ProtocolVersion,    // pulse_bus_version
    pub manifest: ProtocolVersion,     // manifest_version
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contributes {
    #[serde(default)]
    pub commands: Vec<CommandSpec>,
    #[serde(default)]
    pub keymaps: Vec<KeymapSpec>,
    #[serde(default)]
    pub panes: Vec<PaneSpec>,
    #[serde(default)]
    pub themes: Vec<ThemeSpec>,
    #[serde(default)]
    pub settings: HashMap<String, SettingSpec>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandSpec {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub lua_handle: Option<String>,    // None for built-ins
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeymapSpec {
    pub key: Chord,                          // serializes as kebab-case string
    pub command: String,                     // bare id or full /cmd/... Path
    /// Reserved for v1 conditional bindings. Accepted at v0 only as
    /// `null`; any other value is parsed but its semantics are
    /// ignored (host emits a one-time warning).
    #[serde(default)]
    pub when: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaneSpec {
    pub id: String,
    pub slot: SidebarSlot,             // "left" | "right"
    #[serde(default)]
    pub lua_handle: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThemeSpec {
    pub id: String,
    pub label: String,
    pub scopes: HashMap<String, Style>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SettingSpec {
    Boolean { default: bool, label: String },
    String  { default: String, label: String },
    Number  { default: f64, label: String },
    Enum    { default: String, values: Vec<String>, label: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubscriptionSpec {
    #[serde(flatten)]
    pub filter: PulseFilter,           // kinds, field, path_prefix
    pub lua_handle: String,
}
```

Concrete JSON examples follow each section below. The Rust types are
the implementation contract; the JSON shape is the user-facing
contract — they serialize round-trip without surprise via
`#[derive(Serialize, Deserialize)]` plus the custom Chord and Color
serializers defined in `frontend.md`.

## Top-level schema

```json
{
  "name": "file-tree",
  "version": "0.1.0",
  "engines": {
    "devix": "0.1",
    "pulse_bus": "0.1",
    "manifest": "0.1"
  },
  "entry": "main.lua",
  "contributes": {
    "commands": [],
    "keymaps":  [],
    "panes":    [],
    "themes":   [],
    "settings": {}
  },
  "subscribe": []
}
```

### Top-level fields

| Field | Type | Required | Notes |
|---|---|---|---|
| `name` | string | yes | Unique plugin name; becomes `/plugin/<name>` |
| `version` | semver | yes | Plugin's own version |
| `engines` | object | yes | Required versions of devix subsystems |
| `entry` | string | optional | Lua script (relative path). Absent for declarative-only plugins and for built-ins |
| `contributes` | object | optional | Declarative contributions |
| `subscribe` | array | optional | List of PulseFilter entries |

`name` must match `^[a-z0-9][a-z0-9-]*$`: kebab-case, lowercase, ASCII. No
dots in names — dots inside command-id strings would conflict with the
namespace's intra-segment dot convention.

### `engines`

```json
{
  "engines": {
    "devix": "0.1",
    "pulse_bus": "0.1",
    "manifest": "0.1"
  }
}
```

| Key | Source spec | Maps to handshake field |
|---|---|---|
| `devix` | `protocol.md` | `protocol_version` in `Hello` / `Welcome` |
| `pulse_bus` | `pulse-bus.md` | `pulse_bus_version` in `Hello` / `Welcome` |
| `manifest` | this spec | `manifest_version` in `Hello` / `Welcome` |

The manifest uses `engines.devix` as the user-facing name (matches the
project name in user-edited config) while the protocol's Rust struct
field is `protocol_version`. Same value, two presentations: serde
deserializes `engines.devix` into the `protocol_version` field at load
time. No semantic difference; the alias is a pure ergonomics choice.

A plugin loads only if every declared engine version is satisfiable: same
major, host minor ≥ declared minor. Unsatisfied: the plugin is rejected
at load time with `Pulse::PluginError` and a `ProtocolError::Incompatible*`
on its lane.

Future subsystems (LSP, debug adapter) add their own keys.

## `contributes`

### `contributes.commands`

```json
{
  "commands": [
    {
      "id": "refresh",
      "label": "Refresh File Tree",
      "category": "File Tree",
      "lua_handle": "on_refresh"
    }
  ]
}
```

| Field | Required | Notes |
|---|---|---|
| `id` | yes | Bare id; resolves to `/plugin/<name>/cmd/<id>` (plugin) or `/cmd/<id>` (built-in) |
| `label` | yes | Display label in palette, settings UI, etc. |
| `category` | optional | Defaults to plugin `name` (or `"Built-in"`) |
| `lua_handle` | yes for plugins | Lua function name to invoke. Must not be set on built-ins |

Built-in commands resolve to Rust handlers by id at registry-load time
(`/cmd/edit.copy` → built-in registry's `cmd::Copy` action). Plugin
commands resolve to `LuaAction { handle }` where the handle is registered
via `lua_handle` lookup.

### `contributes.keymaps`

```json
{
  "keymaps": [
    { "key": "ctrl-shift-r", "command": "refresh", "when": null }
  ]
}
```

| Field | Required | Notes |
|---|---|---|
| `key` | yes | Chord in canonical kebab-case form (per `namespace.md`) |
| `command` | yes | Bare id (resolves to this manifest's own `commands[]` entries) or absolute `Path` (`/cmd/<dotted-id>` for built-ins, `/plugin/<other>/cmd/<id>` for cross-plugin bindings) |
| `when` | optional | Reserved for v1 conditional binding. Today must be `null` |

Multiple keymaps can bind the same command (alias chords). Two manifests
binding the same chord to *different* commands is a conflict; see
*Keymap conflicts* below.

### Keymap conflicts

When two manifests bind the same chord to different commands, the
**second binding is refused** with a warning. The loader emits
`Pulse::PluginError` describing the conflict and the second manifest's
binding does not take effect.

The user resolves the conflict via an explicit override list in user
config (`$XDG_CONFIG_HOME/devix/keymap-overrides.json`):

```json
{
  "ctrl-shift-r": "/plugin/file-tree/cmd/refresh"
}
```

The override list is applied **after** all manifests are loaded; an entry
binds the chord unconditionally to the named command, displacing whatever
manifest bound it first. Built-in bindings can be overridden the same way.

Loader order matters for determinism: plugins are loaded alphabetically
by directory name (the first one to bind a chord wins by default). Users
who want a different default reach for the override list rather than
renaming directories.

Built-in keymap bindings always load first, so a plugin chord conflicting
with a built-in is the *plugin's* binding that gets refused — built-ins
cannot be silently overridden, only via the explicit override list.

### `contributes.panes`

```json
{
  "panes": [
    { "id": "main", "slot": "left", "lua_handle": "render_main" }
  ]
}
```

| Field | Required | Notes |
|---|---|---|
| `id` | yes | Becomes `/plugin/<name>/pane/<id>` |
| `slot` | yes | `"left"` \| `"right"` (v0). `"floating"` reserved for v1 overlay panes |
| `lua_handle` | yes for plugins | Lua function returning a `View` |

A plugin pane is a Pane (per Stage-9 collapse of LayoutNode into Pane); its
`render` calls into Lua via the lua_handle, gets a serialized View tree
back, and emits it. Stable view-node ids on pane content require the
`StableViewIds` capability.

### `contributes.themes`

```json
{
  "themes": [
    {
      "id": "dim-monochrome",
      "label": "Dim Monochrome",
      "scopes": {
        "keyword":  { "fg": "#aaaaaa", "bold": true },
        "comment":  { "fg": "#666666", "italic": true },
        "function": { "fg": "#cccccc" }
      }
    }
  ]
}
```

Themes are fully declarative — no Lua code touches them. Style fields:

| Field | Type | Notes |
|---|---|---|
| `fg` | hex `#rrggbb` or named color | Foreground |
| `bg` | hex `#rrggbb` or named color | Background |
| `bold` | bool | |
| `italic` | bool | |
| `underline` | bool | |
| `dim` | bool | |
| `reverse` | bool | |

Active theme is selected by id (per session, persisted in user settings).
Switching themes publishes `Pulse::ThemeChanged { id }`; on receipt the
frontend re-requests view immediately and paints under the new theme on
its next frame. The transition is bounded by one round-trip (the
`Request::View` triggered by the pulse), not by the next dirty cycle —
users see the new theme essentially the moment they pick it.

### `contributes.settings`

```json
{
  "settings": {
    "file-tree.show_hidden": {
      "type": "boolean",
      "default": false,
      "label": "Show hidden files"
    },
    "file-tree.glob_ignore": {
      "type": "string",
      "default": "*.tmp",
      "label": "Glob pattern to ignore"
    }
  }
}
```

Each setting key is dotted-namespaced (per the same convention as command
ids). Plugins read settings via the `devix.setting(key)` Lua API; built-ins
read directly from the resolved settings table.

Setting types (v0): `boolean`, `string`, `number`, `enum`. `enum` carries
an additional `values: [...]` list. Nested object schemas are deferred —
see Open Q2.

## `subscribe`

Plugin's pulse subscriptions. Each entry deserializes into the
`SubscriptionSpec` defined above (in *Rust types*) — a `PulseFilter`
(per `pulse-bus.md`) extended with a `lua_handle`. JSON form (filter
fields flattened alongside `lua_handle`):

```json
{
  "subscribe": [
    {
      "kinds": ["buffer_opened", "buffer_changed"],
      "lua_handle": "on_buffer_event"
    },
    {
      "kinds": ["focus_changed"],
      "field": "focus_to",
      "path_prefix": "/pane/0",
      "lua_handle": "on_focus_in_main"
    }
  ]
}
```

| Field | Required | Notes |
|---|---|---|
| `kinds` | optional | Array of `PulseKind` snake-case names; absent matches all |
| `field` | optional | `PulseField` snake-case name; absent uses the variant's default `path` field |
| `path_prefix` | optional | Path string the chosen field must start with |
| `lua_handle` | yes for plugins | Lua function called with the Pulse payload |

The loader registers each filter at plugin load. On match, the bus calls
the plugin's invoke channel with `(handle, pulse_as_lua_table)`.

Built-ins subscribe via Rust code, not the manifest, because the handlers
are typed Rust closures with shared state — declarative subscribe doesn't
fit. The schema's `subscribe` field is plugin-only; built-in manifests
omit it.

## Built-in manifests

Built-ins use the same schema. The built-in commands, keymap, and themes
live in `crates/devix-core/manifests/builtin.json` (loaded via
`include_str!` so it's available without a filesystem read at startup).

```json
{
  "name": "devix-builtin",
  "version": "0.1.0",
  "engines": { "devix": "0.1", "pulse_bus": "0.1", "manifest": "0.1" },
  "contributes": {
    "commands": [
      { "id": "edit.copy",       "label": "Copy",       "category": "Edit" },
      { "id": "edit.paste",      "label": "Paste",      "category": "Edit" },
      { "id": "edit.undo",       "label": "Undo",       "category": "Edit" },
      { "id": "tab.new",         "label": "New Tab",    "category": "Tab" },
      { "id": "split.vertical",  "label": "Split Vertical",   "category": "Split" },
      { "id": "palette.open",    "label": "Open Palette",     "category": "Palette" },
      { "id": "app.quit",        "label": "Quit",       "category": "App" }
      /* ... full set per crates/editor/src/commands/builtins.rs ... */
    ],
    "keymaps": [
      { "key": "ctrl-c",       "command": "edit.copy" },
      { "key": "ctrl-v",       "command": "edit.paste" },
      { "key": "ctrl-z",       "command": "edit.undo" },
      { "key": "ctrl-shift-z", "command": "edit.redo" },
      { "key": "ctrl-y",       "command": "edit.redo" },
      { "key": "ctrl-t",       "command": "tab.new" },
      { "key": "ctrl-shift-p", "command": "palette.open" },
      { "key": "ctrl-q",       "command": "app.quit" }
      /* ... full set per crates/editor/src/commands/keymap.rs ... */
    ],
    "themes": [
      {
        "id": "default",
        "label": "Default (One-Dark-adjacent)",
        "scopes": { /* ports the Theme::default() table from panes/theme.rs */ }
      }
    ]
  }
}
```

Built-ins have no `entry` field; their command handlers are Rust functions
identified by id. The registration step is symmetric otherwise: the loader
reads the JSON, registers each command into `CommandRegistry`, each
keymap into `Keymap`, each theme into the theme registry.

This is the principle's payoff: **palette listing, settings UI,
keymap-config UI all read from manifests**; they don't have separate code
paths for built-in vs plugin contributions.

## Manifest discovery

| Source | Location |
|---|---|
| Built-in | `crates/devix-core/manifests/builtin.json`, embedded via `include_str!` |
| Plugin | `<plugin-dir>/manifest.json` |

Plugin directory resolution:
1. `DEVIX_PLUGIN_DIR` env var (overrides default; primarily for tests).
2. `$XDG_CONFIG_HOME/devix/plugins/` if set.
3. `~/.config/devix/plugins/` otherwise.

Multi-plugin support (T-54 in the master plan): every directory under the
plugin root with a `manifest.json` is loaded. Order is alphabetical by
directory name; first-loaded wins on conflicts (see
*contributes.keymaps → Keymap conflicts*).

The current `DEVIX_PLUGIN` env var (single Lua file) is removed during
T-50 / T-54; replaced by the directory-based scheme above.

## Validation

The loader validates each manifest:

| Check | Failure mode |
|---|---|
| JSON well-formed | Refuse plugin; emit `Pulse::PluginError` |
| Schema match (serde, deny_unknown_fields) | Refuse |
| `name` matches `^[a-z0-9][a-z0-9-]*$` | Refuse |
| `version` is valid semver | Refuse |
| `engines.devix.major` matches host major | Refuse |
| `engines.devix.minor` ≤ host minor | Refuse |
| Same for `engines.pulse_bus` and `engines.manifest` | Refuse |
| Each command `id` is a valid path segment | Refuse |
| Each pane `id` is a valid path segment | Refuse |
| Each theme `id` is a valid path segment | Refuse |
| Each setting key has dotted-namespace form (`<prefix>.<name>`) | Refuse |
| Each keymap `key` parses as a Chord | Refuse |
| Each keymap `command` is a valid bare segment or absolute Path | Refuse |
| Each pane `slot` is a known value | Refuse |
| Each theme color string parses (`"#rrggbb"` / `"@<n>"` / named / `"default"`) | Refuse |
| Each subscribe `kinds` are real `PulseKind` variants | Refuse |
| Each subscribe `field` is a real `PulseField` variant | Refuse |
| Each subscribe `path_prefix` parses as a `Path` | Refuse |
| `entry` file (if set) exists and is readable | Refuse |
| Third-party plugin `name` does **not** start with `devix-` | Refuse |

Validation failure: the plugin is not loaded, no contributions are
registered, and the user sees a `Pulse::PluginError` (typically rendered
to a status line by a future built-in subscriber).

## Versioning

The manifest schema follows semver:

- Adding an optional field: minor.
- Adding a new value to an existing string-enum (`slot`, setting `type`):
  minor; old loaders silently fail validation on the new value.
- Adding a new top-level section (e.g., `commands_async`): minor.
- Renaming or removing a field: major.

Plugins declare `engines.manifest` to assert which schema they were
written against.

## Interaction with other Stage-0 specs

- **`namespace.md`**: command ids and pane ids in manifests are bare
  segments; the loader prepends `/plugin/<name>/cmd/` or `/cmd/`. Theme
  scope keys are dotted segments under `/theme/`.
- **`pulse-bus.md`**: `subscribe` entries deserialize into `PulseFilter`.
  Manifest-declared kind / field names are validated against the `PulseKind`
  / `PulseField` enums at load time.
- **`protocol.md`**: `engines` in the manifest match the versions reported
  in `Hello`/`Welcome`. Manifest validation runs before the plugin's
  Hello handshake.
- **`frontend.md`**: pane `lua_handle`s return `View` trees. Theme styling
  is consumed by the frontend when interpreting `View::Text` nodes.
- **`crates.md`**: manifest types live in `devix-protocol`. The reader and
  validator live in `devix-core`. Built-in `manifests/builtin.json` ships
  with `devix-core`.

## Open questions

1. **Activation events.** Per locked decision, deferred to post-v0. The
   schema does not reserve a field name today — `deny_unknown_fields`
   would reject any forward-looking `activate_on: [...]` placeholder
   because we haven't designed its shape. When activation events land,
   they get a field defined at that time. Plugins written for future
   `manifest_version` declare their required version in `engines`;
   older hosts reject them rather than load with the field silently
   ignored.

2. **Settings type system.** Today's draft has flat `boolean | string |
   number | enum`. VS Code has nested object schemas with validation.
   Lean: flat for v0; revisit when Settings UI lands and we have
   real-world plugin needs.

3. **Plugin entry script type.** Lua only today (mlua). Future: WASM,
   native dylibs, JS via QuickJS. Manifest's `entry` is `<filename>.lua`
   for now; later add an `entry_type: "lua" | "wasm" | "dylib"` field
   with `lua` as default. Lean: keep Lua-only in v0; add `entry_type`
   when a second runtime ships.

4. **Manifest hot-reload.** Editing `manifest.json` while running — does
   the plugin reload? Lean: no in v0; restart-required. Hot reload is
   future; needs a `Pulse::PluginManifestChanged` and a careful unload
   path.

5. **JSON Schema document.** Should we ship a JSON Schema
   (`.schema.json`) so editors / VS Code / IDEs can validate
   user-written manifests? Lean: yes, generated from the serde structs
   via `schemars` once T-23 (manifest reader skeleton) lands. Adds
   polish; not blocking.

## Resolved during initial review

- Keymap conflict resolution → refuse-second-with-warning + explicit
  user override list at `$XDG_CONFIG_HOME/devix/keymap-overrides.json`.
  Built-in bindings load first; plugins cannot silently override them.
  See *Keymap conflicts* under `contributes.keymaps`.
- Theme switching semantics → immediate re-render. `Pulse::ThemeChanged`
  carries a resolved `ThemePalette`; on receipt the frontend
  re-requests view and paints under the new theme. Bounded by one
  round-trip, not by the next dirty cycle.
- `devix-` prefix reserved for first-party manifests. The loader
  refuses any third-party manifest whose `name` starts with `devix-`.
  See validation table.
