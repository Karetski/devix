# Modularization Refactor — Design Spec

**Date:** 2026-05-01
**Scope:** Split `crates/app/src/main.rs` (588 lines) into focused modules; populate the
`teditor-workspace` and `teditor-config` crates. No behavior change.

---

## Goal

Phases 1 and 2 of `PLAN.md` shipped as a single 588-line `main.rs` plus a small `ui` crate.
Before Phase 3 (tabs, splits, panels) lands, extract a stable seam between
"editor state + commands" and "the binary that wires terminal I/O." This refactor:

- moves per-buffer editor state into `teditor-workspace`
- introduces a closed `Action` enum + dispatcher as the single mutation entry point
- introduces a `Keymap` in `teditor-config`
- splits `main.rs` into focused submodules (`app`, `events`, `clipboard`, `watcher`, `render`)
- splits `crates/ui/src/lib.rs` into `editor.rs` + `status.rs`

The other stub crates (`syntax`, `lsp`, `plugin`) are not touched.

## Non-goals

- Tabs, splits, panels, layout tree (Phase 3).
- Command palette, command registry, string→Action lookup (Phase 5).
- Themes, settings file loading, `serde`/TOML deps on `teditor-config`.
- Multi-cursor *commands* (selection model already supports multi-region).
- Plugin host, plugin-contributed actions, open `Action` enum (Phase 8).
- Buffer API additions. `current_line_span` stays in workspace for this refactor.
- Decoupling `teditor-config` from `crossterm`. The keymap takes `crossterm` types directly.
- Tests for the new modules. Behavior is preserved; tests arrive when units have
  non-trivial logic of their own.

## Behavior preserved exactly

Every existing keybinding produces the same effect. The disk-reconcile prompt has the same
trigger, same status text, and same Ctrl+R/Ctrl+K behavior. Clipboard copy/cut on an empty
selection still operate in line-mode with the same status messages. Mouse click/drag/scroll
behavior is identical. `Tab` inserts four spaces; `Enter` inserts `"\n"`. The scroll-on-
cursor-move logic in `render` is unchanged.

---

## Architecture overview

```
┌─ crates/app ───────────────────────────────────────────────┐
│  main.rs   terminal setup/teardown, calls run()           │
│  app.rs    App struct, App::new, run() loop                │
│  events.rs KeyEvent→Chord→Action; MouseEvent→Action;       │
│            disk-pending input gate; calls dispatch()       │
│  render.rs frame layout, scroll adjust, builds             │
│            EditorView + StatusInfo, calls into ui::*       │
│  watcher.rs spawn_watcher, drain_disk_events               │
│  clipboard.rs arboard::Clipboard::new() init wrapper       │
└─────────────┬──────────────────────────────────────────────┘
              │ Context { editor, clipboard, status, quit,
              │           disk_changed_pending, viewport }
              ▼
┌─ crates/workspace ─────────────────────────────────────────┐
│  state.rs    EditorState { buffer, selection,              │
│              target_col, scroll_top } + motion/apply_tx    │
│  action.rs   Action enum (closed)                          │
│  context.rs  Context, StatusLine, Viewport                 │
│  dispatch.rs dispatch(action, &mut cx) — single match      │
└────────────────────────────────────────────────────────────┘
              ▲
              │ Action
┌─ crates/config ────────────────────────────────────────────┐
│  keymap.rs Chord, Keymap, default_keymap()                 │
└────────────────────────────────────────────────────────────┘
```

Dependency direction: `app` → `config` → `workspace` → `buffer`. `ui` depends on `buffer`
only. `workspace` does **not** depend on `ratatui` or `crossterm`.

---

## Component: `teditor-workspace`

### `EditorState` (state.rs)

```rust
pub struct EditorState {
    pub buffer: Buffer,
    pub selection: Selection,
    pub target_col: Option<usize>,   // sticky column for vertical motion
    pub scroll_top: usize,
}

impl EditorState {
    pub fn primary(&self) -> Range;
    pub fn move_to(&mut self, idx: usize, extend: bool, sticky_col: bool);
    pub fn move_vertical(&mut self, down: bool, extend: bool);
    pub fn apply_tx(&mut self, tx: Transaction);  // resets target_col, sets selection
}
```

Migrated verbatim from current free functions in `main.rs` (`move_to`, `move_vertical`,
`apply_tx`) and from `App::primary`. No logic change.

### `Context` (context.rs)

```rust
pub struct Context<'a> {
    pub editor: &'a mut EditorState,
    pub clipboard: &'a mut Option<arboard::Clipboard>,
    pub status: &'a mut StatusLine,
    pub quit: &'a mut bool,
    pub disk_changed_pending: &'a mut bool,
    pub viewport: Viewport,           // Copy snapshot, taken per-frame
}

pub struct StatusLine(Option<String>);
impl StatusLine {
    pub fn set(&mut self, s: impl Into<String>);
    pub fn clear(&mut self);
    pub fn get(&self) -> Option<&str>;
}

#[derive(Copy, Clone)]
pub struct Viewport {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub gutter_width: u16,
}
```

`Viewport` deliberately uses primitives, not `ratatui::Rect`, so workspace stays free of
the `ratatui` dependency. The `app` layer translates `Rect → Viewport` once per frame
inside `render.rs`.

### `Action` (action.rs)

Closed enum. `Clone`, `Debug`. Not `PartialEq` (not needed).

```rust
pub enum Action {
    // motion
    MoveLeft { extend: bool },
    MoveRight { extend: bool },
    MoveUp { extend: bool },
    MoveDown { extend: bool },
    MoveWordLeft { extend: bool },
    MoveWordRight { extend: bool },
    MoveLineStart { extend: bool },
    MoveLineEnd { extend: bool },
    MoveDocStart { extend: bool },
    MoveDocEnd { extend: bool },
    PageUp { extend: bool },
    PageDown { extend: bool },

    // edits
    InsertChar(char),
    InsertNewline,
    InsertTab,
    DeleteBack { word: bool },
    DeleteForward { word: bool },

    // history
    Undo,
    Redo,

    // selection
    SelectAll,

    // clipboard
    Copy,
    Cut,
    Paste,

    // file / disk
    Save,
    ReloadFromDisk,
    KeepBufferIgnoreDisk,

    // app
    Quit,

    // mouse
    ClickAt { col: u16, row: u16, extend: bool },
    DragAt { col: u16, row: u16 },
    ScrollUp,
    ScrollDown,
}
```

Notes:

- `extend` is a struct field on motion variants, not a separate `Select*` variant. Halves
  variant count and matches how Shift composes with motion at the keymap layer.
- `InsertChar(char)` is the typing path. `InsertNewline` and `InsertTab` are explicit
  because they currently insert hardcoded strings (`"\n"`, `"    "`).
- Mouse events route through Actions to keep a single mutation entry point. Mouse is not
  rebindable.
- No `Action::Compose`, no noop, no plugin variants. Phase 8 will revisit.

### `dispatch` (dispatch.rs)

```rust
pub fn dispatch(action: Action, cx: &mut Context<'_>);
```

Single flat `match` on `Action`. Each arm calls a method on `cx.editor` or a small free
helper. The dispatcher is the only place that reads/writes `cx.quit`.

Helpers in this file:

- `current_line_span(buf: &Buffer, head: usize) -> (usize, usize)` — moved from
  `main.rs`. Used by `Copy` and `Cut`.
- `page_step(viewport: Viewport) -> usize` — `viewport.height.saturating_sub(1).max(1)`.

Clipboard handlers (`do_copy`, `do_cut`, `do_paste`) live in `dispatch.rs` because they
mutate `EditorState` (cut and paste) and read/write status. The `app/clipboard.rs` file
owns *only* `arboard::Clipboard::new()` initialization and any error-mapping wrapper.

Disk-pending interaction:

- `ReloadFromDisk` calls `editor.buffer.reload_from_disk()`. On `Ok`, clamps the
  selection, sets `*cx.disk_changed_pending = false`, and sets status `"reloaded from
  disk"`. On `Err(e)`, sets status `"reload failed: {e}"` and **leaves the flag set**
  (matches today). Always callable; the events layer is responsible for only dispatching
  it when meaningful (see "Disk-pending input gate").
- `KeepBufferIgnoreDisk` sets `*cx.disk_changed_pending = false` and sets status
  `"kept buffer; disk change ignored"`. Always callable.

### `lib.rs`

Re-exports: `Action`, `Context`, `EditorState`, `StatusLine`, `Viewport`, `dispatch`.

### Cargo deps

`teditor-buffer`, `arboard` (for the `Clipboard` type in `Context`).

---

## Component: `teditor-config`

### `Chord` and `Keymap` (keymap.rs)

```rust
#[derive(Copy, Clone, Hash, Eq, PartialEq)]
pub struct Chord {
    pub code: KeyCode,
    pub mods: KeyModifiers,    // CONTROL | SHIFT | ALT, normalized
}

pub struct Keymap {
    bindings: HashMap<Chord, Action>,
}

impl Keymap {
    pub fn lookup(&self, chord: Chord) -> Option<Action>;
}

pub fn default_keymap() -> Keymap;
```

`KeyCode` and `KeyModifiers` already implement `Hash + Eq + Copy` in crossterm, so the
derives compose.

#### Chord normalization (applied at chord-build time in `events.rs`)

- If `code` is `KeyCode::Char(c)` and `c.is_ascii_alphabetic()`, lowercase it. This
  matches today's `'s' | 'S'` arms exactly and is independent of how a given terminal
  reports Shift+letter.
- All modifier bits (`CONTROL`, `SHIFT`, `ALT`) are kept as crossterm reports them. The
  keymap stores the concrete `extend` value on motion variants, so SHIFT is meaningful;
  `Ctrl+Shift+Z` is distinct from `Ctrl+Z`.

#### Action coverage

Motion appears twice in the keymap — once with `extend: false`, once with `extend: true`.
Verbose but explicit; avoids a "modifier transform" layer.

#### Char input is the fallback, not a keymap entry

Resolution order in `events.rs`:

1. Build `Chord` from the `KeyEvent`.
2. `keymap.lookup(chord)` — if `Some`, dispatch that Action.
3. Else, if the event is `KeyCode::Char(c)` with no Ctrl and no Alt, dispatch
   `Action::InsertChar(c)`.
4. Else, drop.

#### Default keymap content

Built from current handlers in `main.rs`:

| Chord                  | Action                          |
|------------------------|---------------------------------|
| `Ctrl+Q`               | `Quit`                          |
| `Ctrl+S`               | `Save`                          |
| `Ctrl+Z`               | `Undo`                          |
| `Ctrl+Shift+Z`         | `Redo`                          |
| `Ctrl+Y`               | `Redo`                          |
| `Ctrl+A`               | `SelectAll`                     |
| `Ctrl+C`               | `Copy`                          |
| `Ctrl+X`               | `Cut`                           |
| `Ctrl+V`               | `Paste`                         |
| `Ctrl+Left/Right`      | `MoveLineStart/End { extend: shift }` |
| `Ctrl+Up/Down`         | `MoveDocStart/End { extend: shift }`  |
| `Alt+Left/Right`       | `MoveWordLeft/Right { extend: shift }`|
| `Left/Right/Up/Down`   | `MoveLeft/Right/Up/Down { extend: shift }` |
| `Home/End`             | `MoveLineStart/End { extend: shift }` |
| `PageUp/PageDown`      | `PageUp/PageDown { extend: shift }`   |
| `Backspace`            | `DeleteBack { word: false }`    |
| `Alt+Backspace`        | `DeleteBack { word: true }`     |
| `Delete`               | `DeleteForward { word: false }` |
| `Alt+Delete`           | `DeleteForward { word: true }`  |
| `Enter`                | `InsertNewline`                 |
| `Tab`                  | `InsertTab`                     |

For motion entries with `extend: shift`, both variants are present in the table (one
chord per `extend` value).

`Ctrl+R` and `Ctrl+K` are intentionally **not** in the default keymap. They are special-
cased in `events.rs` while the disk-pending prompt is active (see "Disk-pending input
gate" below). Outside that prompt today they are no-ops; the keymap preserves that.

### Mouse

Mouse is not in the keymap. The events layer translates `MouseEvent` directly to
`Action::ClickAt` / `DragAt` / `ScrollUp` / `ScrollDown`. Rebinding mouse is out of scope.

### Disk-pending input gate

The disk-pending state lives on `App`. The events layer special-cases two chords while
`disk_changed_pending` is true:

- `Ctrl+R` → dispatch `Action::ReloadFromDisk`, then `return` from event handling.
- `Ctrl+K` → dispatch `Action::KeepBufferIgnoreDisk`, then `return`.

All other chords fall through to normal keymap lookup (and the `InsertChar` fallback) —
typing while pending is allowed and does not dismiss the prompt. This matches today's
behavior in `main.rs`: only `Ctrl+R` and `Ctrl+K` interact with the prompt; other input
is unaffected. The dispatcher itself stays stateless about the prompt; both Action
handlers do their work unconditionally (reload from disk; clear the flag and set status).

### `lib.rs`

Re-exports: `Chord`, `Keymap`, `default_keymap`.

### Cargo deps

`teditor-workspace` (for `Action`), `crossterm` (for `KeyCode`, `KeyModifiers`).

---

## Component: `teditor-ui` (split-only refactor)

Current `crates/ui/src/lib.rs` (159 lines) splits into:

```
crates/ui/src/
  lib.rs       // re-exports
  editor.rs    // EditorView, EditorRenderResult, render_editor,
               // paint_selection, paint_line_span, paint_zero_width
  status.rs    // StatusInfo, render_status
```

`render_status` today is in `app/main.rs` and reads `&App` directly. Split:

- `ui/status.rs` exposes `pub struct StatusInfo` (path: `Option<String>`, dirty: bool,
  line: usize, col: usize, sel_len: usize, status_msg: Option<String>) and
  `pub fn render_status(info: &StatusInfo, area: Rect, frame: &mut Frame)`.
- `app/render.rs` builds a `StatusInfo` from `App` state and calls `ui::render_status`.

This keeps `ui` independent of `app` and removes the only place where rendering reached
into the App struct.

`editor.rs` content is the current `lib.rs` content moved verbatim.

---

## Component: `teditor` (the binary)

### Files

```
crates/app/src/
  main.rs       // ~50 lines: arg parse, terminal setup/teardown, calls run()
  app.rs        // App struct, App::new, run() loop
  events.rs     // chord build, KeyEvent→Action, MouseEvent→Action,
                // disk-pending input gate, dispatch entry
  clipboard.rs  // arboard::Clipboard::new() + tiny error wrapper
  watcher.rs    // spawn_watcher, drain_disk_events
  render.rs     // render(frame, &mut App), Layout split, scroll adjust,
                // build EditorView + StatusInfo, call ui::*
```

### `App` struct (after refactor)

```rust
struct App {
    editor: EditorState,
    keymap: Keymap,
    clipboard: Option<arboard::Clipboard>,
    status: StatusLine,
    quit: bool,
    viewport: Viewport,
    _watcher: Option<notify::RecommendedWatcher>,
    disk_rx: Option<mpsc::Receiver<()>>,
    disk_changed_pending: bool,
}
```

### Per-event Context construction

```rust
let mut cx = Context {
    editor: &mut app.editor,
    clipboard: &mut app.clipboard,
    status: &mut app.status,
    quit: &mut app.quit,
    disk_changed_pending: &mut app.disk_changed_pending,
    viewport: app.viewport,    // Copy
};
dispatch(action, &mut cx);
```

### Cargo deps

Existing deps plus `teditor-workspace`, `teditor-config`.

---

## Approximate line counts post-split

| File                     | Lines (est.) |
|--------------------------|--------------|
| `app/main.rs`            | 50           |
| `app/app.rs`             | 80           |
| `app/events.rs`          | 120          |
| `app/clipboard.rs`       | 15           |
| `app/watcher.rs`         | 50           |
| `app/render.rs`          | 80           |
| `workspace/state.rs`     | 80           |
| `workspace/action.rs`    | 50           |
| `workspace/context.rs`   | 30           |
| `workspace/dispatch.rs`  | 250          |
| `config/keymap.rs`       | 120          |
| `ui/editor.rs`           | 120          |
| `ui/status.rs`           | 40           |
| **Total**                | **~1085**    |

vs. today's 588-line `main.rs` + 159-line `ui/lib.rs` = 747. The increase is structural
(scaffolding, re-exports, type definitions) and bounded — every file fits in one screen
and has a single concern. Largest is `dispatch.rs` at ~250, a flat match.

---

## Verification

- `cargo build --workspace` succeeds.
- `cargo clippy --workspace` clean (or no new warnings vs. baseline).
- Manual smoke test of every preserved behavior:
  - Open file via CLI arg; type characters; `Enter`; `Tab`.
  - All motion keys with and without Shift: arrows, Ctrl+arrows, Alt+arrows, Home, End,
    PageUp, PageDown.
  - `Backspace`, `Alt+Backspace`, `Delete`, `Alt+Delete`.
  - `Ctrl+Z`, `Ctrl+Shift+Z`, `Ctrl+Y`.
  - `Ctrl+A`, `Ctrl+C`, `Ctrl+X`, `Ctrl+V` with empty selection (line mode) and with a
    selection.
  - `Ctrl+S` save.
  - External file change: prompt appears, `Ctrl+R` reloads, `Ctrl+K` keeps buffer.
  - Mouse: left-click positions cursor, Shift-click extends, drag selects, scroll wheel
    scrolls.
  - `Ctrl+Q` quits.
- No new automated tests in this refactor.

---

## Risks and mitigations

- **Subtle behavior drift in chord normalization.** Mitigation: build the chord
  normalization helper deliberately and compare against current `main.rs` `'s' | 'S'`
  patterns. The default-keymap table is the source of truth.
- **Context borrow patterns.** `dispatch` takes `&mut Context`; the editor field is
  borrowed mutably and the clipboard/status fields concurrently. Confirmed safe — they
  are disjoint fields. If a future Action needs the keymap or watcher, those would be
  added to `Context` then.
- **`Viewport` snapshot staleness.** `viewport` is captured per-frame and used by
  Actions dispatched on input events that arrive *after* the frame. This matches today's
  behavior (`last_editor_area` / `last_gutter_width` are updated in `render` and read in
  `handle_mouse`). Resize between render and input is a pre-existing edge case unchanged
  by this refactor.

---

## Out-of-scope reaffirmed

- No tabs, splits, panels, layout tree.
- No command palette, command registry, or string→Action lookup.
- No theme or settings file loading.
- No multi-cursor commands.
- No plugin host or plugin-contributed actions.
- No new `Buffer` API.
- No decoupling of `teditor-config` from `crossterm`.
- No new automated tests.
