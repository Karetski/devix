# Modularization Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split `crates/app/src/main.rs` (588 lines) into focused modules; populate the
`teditor-workspace` and `teditor-config` crates with `EditorState`, a closed `Action` enum
+ dispatcher, and a `Keymap`. No behavior change.

**Architecture:** Bottom-up. Build out `teditor-workspace` and `teditor-config` as
standalone crates first (each task leaves the workspace crate compiling green even though
it isn't wired into the app yet). Then split `crates/ui/src/lib.rs`. Then wire workspace
+ config into the app in two passes (state first, dispatch second). Finally split
`crates/app/src/main.rs` into focused submodules.

**Tech Stack:** Rust 2021, ratatui 0.29, crossterm 0.28, ropey 1.6, anyhow, arboard,
notify. No new external dependencies introduced.

**Spec:** `docs/superpowers/specs/2026-05-01-modularization-design.md`

**Verification per task:** `cargo build --workspace` and `cargo clippy --workspace
--all-targets -- -D warnings`. Both must succeed before commit.

**Manual smoke test (Task 8 only):**
```
cargo run -- /tmp/refactor-test.txt
```
Type, navigate (all motion combinations), select with Shift, copy/cut/paste, undo/redo,
save (Ctrl+S), trigger external change (`echo new > /tmp/refactor-test.txt` from another
shell) and confirm Ctrl+R / Ctrl+K both work, mouse click/drag/scroll, Ctrl+Q quit.

---

## File Structure (post-refactor)

```
crates/workspace/src/
  lib.rs           # re-exports
  state.rs         # EditorState + impl (primary, move_to, move_vertical, apply_tx)
  action.rs        # Action enum (closed)
  context.rs       # Context<'a>, StatusLine, Viewport
  dispatch.rs      # dispatch(action, &mut cx) + helpers (current_line_span,
                   # page_step, do_copy/cut/paste, replace_selection,
                   # delete_primary_or)

crates/config/src/
  lib.rs           # re-exports
  keymap.rs        # Chord, Keymap, default_keymap()

crates/ui/src/
  lib.rs           # re-exports
  editor.rs        # EditorView, EditorRenderResult, render_editor + paint helpers
  status.rs        # StatusInfo, render_status

crates/app/src/
  main.rs          # ~50 lines: arg parse, terminal setup/teardown, calls run()
  app.rs           # App struct, App::new, run() loop
  events.rs        # chord build, KeyEvent→Action, MouseEvent→Action,
                   # disk-pending input gate, dispatch entry
  clipboard.rs     # arboard::Clipboard::new() helper
  watcher.rs       # spawn_watcher, drain_disk_events
  render.rs        # render(frame, &mut App), Layout split, scroll adjust,
                   # build EditorView + StatusInfo, call ui::*
```

---

## Task 1: Scaffold `teditor-workspace` — types only

**Goal:** Workspace crate exposes `EditorState`, `Action`, `Context`, `StatusLine`,
`Viewport`. No `dispatch` yet. Crate compiles standalone; not yet used by app.

**Files:**
- Modify: `crates/workspace/Cargo.toml`
- Create: `crates/workspace/src/state.rs`
- Create: `crates/workspace/src/action.rs`
- Create: `crates/workspace/src/context.rs`
- Modify: `crates/workspace/src/lib.rs`

- [ ] **Step 1: Add dependencies to `crates/workspace/Cargo.toml`**

Replace the empty `[dependencies]` section with:

```toml
[dependencies]
teditor-buffer.workspace = true
arboard = { version = "3.4", default-features = false }
```

- [ ] **Step 2: Create `crates/workspace/src/state.rs`**

```rust
//! Per-buffer editor state: buffer + selection + sticky column + scroll.
//!
//! Methods migrate the motion/edit helpers that previously lived as free
//! functions in `crates/app/src/main.rs`. No logic change.

use teditor_buffer::{Buffer, Range, Selection, Transaction};

pub struct EditorState {
    pub buffer: Buffer,
    pub selection: Selection,
    /// Sticky column for vertical motion. Reset on horizontal motion or edit.
    pub target_col: Option<usize>,
    pub scroll_top: usize,
}

impl EditorState {
    pub fn new(buffer: Buffer) -> Self {
        Self {
            buffer,
            selection: Selection::point(0),
            target_col: None,
            scroll_top: 0,
        }
    }

    pub fn primary(&self) -> Range {
        self.selection.primary()
    }

    pub fn move_to(&mut self, idx: usize, extend: bool, sticky_col: bool) {
        let r = self.primary().put_head(idx, extend);
        *self.selection.primary_mut() = r;
        if !sticky_col {
            self.target_col = None;
        }
    }

    pub fn move_vertical(&mut self, down: bool, extend: bool) {
        let head = self.primary().head;
        let col = self
            .target_col
            .unwrap_or_else(|| self.buffer.col_of_char(head));
        let new = if down {
            self.buffer.move_down(head, Some(col))
        } else {
            self.buffer.move_up(head, Some(col))
        };
        self.target_col = Some(col);
        self.move_to(new, extend, true);
    }

    /// Apply a transaction; updates selection and clears the sticky column.
    pub fn apply_tx(&mut self, tx: Transaction) {
        let after = tx.selection_after.clone();
        self.buffer.apply(tx);
        self.selection = after;
        self.target_col = None;
    }
}
```

- [ ] **Step 3: Create `crates/workspace/src/action.rs`**

```rust
//! Closed enum of every editor command. The dispatcher's only input.

#[derive(Clone, Debug)]
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

- [ ] **Step 4: Create `crates/workspace/src/context.rs`**

```rust
//! Dispatcher context: bundles every mutable reference an Action handler may
//! touch, plus a per-frame copy of viewport geometry.

use crate::state::EditorState;

#[derive(Default)]
pub struct StatusLine(Option<String>);

impl StatusLine {
    pub fn set(&mut self, s: impl Into<String>) {
        self.0 = Some(s.into());
    }
    pub fn clear(&mut self) {
        self.0 = None;
    }
    pub fn get(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

#[derive(Copy, Clone, Default)]
pub struct Viewport {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub gutter_width: u16,
}

pub struct Context<'a> {
    pub editor: &'a mut EditorState,
    pub clipboard: &'a mut Option<arboard::Clipboard>,
    pub status: &'a mut StatusLine,
    pub quit: &'a mut bool,
    pub disk_changed_pending: &'a mut bool,
    pub viewport: Viewport,
}
```

- [ ] **Step 5: Replace `crates/workspace/src/lib.rs`**

```rust
//! Editor state, action enum, and dispatcher. Phase 3-ready scaffold.

pub mod action;
pub mod context;
pub mod state;

pub use action::Action;
pub use context::{Context, StatusLine, Viewport};
pub use state::EditorState;
```

- [ ] **Step 6: Verify**

Run: `cargo build --workspace`
Expected: success.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: success.

- [ ] **Step 7: Commit**

```bash
git add crates/workspace/
git commit -m "workspace: add EditorState, Action, Context types"
```

---

## Task 2: `teditor-workspace` — `dispatch` and helpers

**Goal:** Add the dispatcher and all command handlers (motion, edit, clipboard,
file/disk, app). Crate is now functionally complete; still not wired into app.

**Files:**
- Create: `crates/workspace/src/dispatch.rs`
- Modify: `crates/workspace/src/lib.rs`

- [ ] **Step 1: Create `crates/workspace/src/dispatch.rs`**

```rust
//! Action dispatcher. Single flat match; the only place that mutates Context
//! state in response to commands.

use teditor_buffer::{Buffer, Range, Selection, delete_range_tx, replace_selection_tx};

use crate::action::Action;
use crate::context::{Context, Viewport};
use crate::state::EditorState;

pub fn dispatch(action: Action, cx: &mut Context<'_>) {
    use Action::*;
    match action {
        // ---- motion ----
        MoveLeft { extend } => {
            let to = cx.editor.buffer.move_left(cx.editor.primary().head);
            cx.editor.move_to(to, extend, false);
        }
        MoveRight { extend } => {
            let to = cx.editor.buffer.move_right(cx.editor.primary().head);
            cx.editor.move_to(to, extend, false);
        }
        MoveUp { extend } => cx.editor.move_vertical(false, extend),
        MoveDown { extend } => cx.editor.move_vertical(true, extend),
        MoveWordLeft { extend } => {
            let to = cx.editor.buffer.word_left(cx.editor.primary().head);
            cx.editor.move_to(to, extend, false);
        }
        MoveWordRight { extend } => {
            let to = cx.editor.buffer.word_right(cx.editor.primary().head);
            cx.editor.move_to(to, extend, false);
        }
        MoveLineStart { extend } => {
            let to = cx.editor.buffer.line_start_of(cx.editor.primary().head);
            cx.editor.move_to(to, extend, false);
        }
        MoveLineEnd { extend } => {
            let to = cx.editor.buffer.line_end_of(cx.editor.primary().head);
            cx.editor.move_to(to, extend, false);
        }
        MoveDocStart { extend } => {
            let to = cx.editor.buffer.doc_start();
            cx.editor.move_to(to, extend, false);
        }
        MoveDocEnd { extend } => {
            let to = cx.editor.buffer.doc_end();
            cx.editor.move_to(to, extend, false);
        }
        PageUp { extend } => {
            let step = page_step(cx.viewport);
            for _ in 0..step {
                cx.editor.move_vertical(false, extend);
            }
        }
        PageDown { extend } => {
            let step = page_step(cx.viewport);
            for _ in 0..step {
                cx.editor.move_vertical(true, extend);
            }
        }

        // ---- edits ----
        InsertChar(c) => {
            let mut buf = [0u8; 4];
            replace_selection(cx, c.encode_utf8(&mut buf));
        }
        InsertNewline => replace_selection(cx, "\n"),
        InsertTab => replace_selection(cx, "    "),
        DeleteBack { word } => {
            delete_primary_or(cx, |buf, head| {
                if head == 0 {
                    return None;
                }
                let start = if word { buf.word_left(head) } else { head - 1 };
                Some((start, head))
            });
        }
        DeleteForward { word } => {
            delete_primary_or(cx, |buf, head| {
                let len = buf.len_chars();
                if head >= len {
                    return None;
                }
                let end = if word { buf.word_right(head) } else { head + 1 };
                Some((head, end))
            });
        }

        // ---- history ----
        Undo => {
            if let Some(sel) = cx.editor.buffer.undo() {
                cx.editor.selection = sel;
                cx.editor.target_col = None;
                cx.status.clear();
            } else {
                cx.status.set("nothing to undo");
            }
        }
        Redo => {
            if let Some(sel) = cx.editor.buffer.redo() {
                cx.editor.selection = sel;
                cx.editor.target_col = None;
                cx.status.clear();
            } else {
                cx.status.set("nothing to redo");
            }
        }

        // ---- selection ----
        SelectAll => {
            let end = cx.editor.buffer.len_chars();
            cx.editor.selection = Selection::single(Range::new(0, end));
            cx.editor.target_col = None;
        }

        // ---- clipboard ----
        Copy => do_copy(cx),
        Cut => do_cut(cx),
        Paste => do_paste(cx),

        // ---- file / disk ----
        Save => {
            let msg = match cx.editor.buffer.save() {
                Ok(()) => "saved".to_string(),
                Err(e) => format!("save failed: {e}"),
            };
            cx.status.set(msg);
        }
        ReloadFromDisk => match cx.editor.buffer.reload_from_disk() {
            Ok(()) => {
                let max = cx.editor.buffer.len_chars();
                cx.editor.selection.clamp(max);
                *cx.disk_changed_pending = false;
                cx.status.set("reloaded from disk");
            }
            Err(e) => cx.status.set(format!("reload failed: {e}")),
        },
        KeepBufferIgnoreDisk => {
            *cx.disk_changed_pending = false;
            cx.status.set("kept buffer; disk change ignored");
        }

        // ---- app ----
        Quit => *cx.quit = true,

        // ---- mouse ----
        ClickAt { col, row, extend } => {
            if let Some(idx) = click_to_char_idx(cx, col, row) {
                cx.editor.move_to(idx, extend, false);
            }
        }
        DragAt { col, row } => {
            if let Some(idx) = click_to_char_idx(cx, col, row) {
                cx.editor.move_to(idx, true, false);
            }
        }
        ScrollUp => {
            cx.editor.scroll_top = cx.editor.scroll_top.saturating_sub(3);
        }
        ScrollDown => {
            let max = cx.editor.buffer.line_count().saturating_sub(1);
            cx.editor.scroll_top = (cx.editor.scroll_top + 3).min(max);
        }
    }
}

// ---------------------------------------------------------------------------
// Edit helpers
// ---------------------------------------------------------------------------

fn replace_selection(cx: &mut Context<'_>, text: &str) {
    let tx = replace_selection_tx(&cx.editor.buffer, &cx.editor.selection, text);
    cx.editor.apply_tx(tx);
    cx.status.clear();
}

fn delete_primary_or(
    cx: &mut Context<'_>,
    builder: impl FnOnce(&Buffer, usize) -> Option<(usize, usize)>,
) {
    let prim = cx.editor.primary();
    if !prim.is_empty() {
        let tx = delete_range_tx(
            &cx.editor.buffer,
            &cx.editor.selection,
            prim.start(),
            prim.end(),
        );
        cx.editor.apply_tx(tx);
        cx.status.clear();
        return;
    }
    let Some((start, end)) = builder(&cx.editor.buffer, prim.head) else {
        return;
    };
    if start == end {
        return;
    }
    let tx = delete_range_tx(&cx.editor.buffer, &cx.editor.selection, start, end);
    cx.editor.apply_tx(tx);
    cx.status.clear();
}

// ---------------------------------------------------------------------------
// Clipboard handlers
// ---------------------------------------------------------------------------

fn current_line_span(buf: &Buffer, head: usize) -> (usize, usize) {
    let line = buf.line_of_char(head);
    let start = buf.line_start(line);
    let end_no_nl = start + buf.line_len_chars(line);
    let end = if line + 1 < buf.line_count() {
        buf.line_start(line + 1)
    } else {
        end_no_nl
    };
    (start, end)
}

fn do_copy(cx: &mut Context<'_>) {
    let prim = cx.editor.primary();
    let (start, end, msg) = if prim.is_empty() {
        let (s, e) = current_line_span(&cx.editor.buffer, prim.head);
        (s, e, "copied line")
    } else {
        (prim.start(), prim.end(), "copied")
    };
    if start == end {
        return;
    }
    let text = cx.editor.buffer.slice_to_string(start, end);
    let Some(cb) = cx.clipboard.as_mut() else {
        cx.status.set("no system clipboard");
        return;
    };
    if cb.set_text(text).is_err() {
        cx.status.set("clipboard error");
        return;
    }
    cx.status.set(msg);
}

fn do_cut(cx: &mut Context<'_>) {
    let prim = cx.editor.primary();
    let (start, end, line_cut) = if prim.is_empty() {
        let (s, e) = current_line_span(&cx.editor.buffer, prim.head);
        (s, e, true)
    } else {
        (prim.start(), prim.end(), false)
    };
    if start == end {
        return;
    }
    let text = cx.editor.buffer.slice_to_string(start, end);
    let Some(cb) = cx.clipboard.as_mut() else {
        cx.status.set("no system clipboard");
        return;
    };
    if cb.set_text(text).is_err() {
        cx.status.set("clipboard error");
        return;
    }
    let tx = delete_range_tx(&cx.editor.buffer, &cx.editor.selection, start, end);
    cx.editor.apply_tx(tx);
    cx.status.set(if line_cut { "cut line" } else { "cut" });
}

fn do_paste(cx: &mut Context<'_>) {
    let text = match cx.clipboard.as_mut().and_then(|cb| cb.get_text().ok()) {
        Some(t) => t,
        None => {
            cx.status.set("clipboard empty");
            return;
        }
    };
    if text.is_empty() {
        return;
    }
    replace_selection(cx, &text);
    cx.status.set("pasted");
}

// ---------------------------------------------------------------------------
// Mouse helpers
// ---------------------------------------------------------------------------

fn click_to_char_idx(cx: &Context<'_>, col: u16, row: u16) -> Option<usize> {
    let v = cx.viewport;
    if row < v.y || row >= v.y + v.height {
        return None;
    }
    let text_x = v.x + v.gutter_width;
    let click_col = col.saturating_sub(text_x) as usize;
    let row_in_view = (row - v.y) as usize;
    let buf = &cx.editor.buffer;
    let line = (cx.editor.scroll_top + row_in_view).min(buf.line_count().saturating_sub(1));
    let local_col = click_col.min(buf.line_len_chars(line));
    Some(buf.line_start(line) + local_col)
}

// ---------------------------------------------------------------------------
// Misc
// ---------------------------------------------------------------------------

fn page_step(viewport: Viewport) -> usize {
    viewport.height.saturating_sub(1).max(1) as usize
}
```

- [ ] **Step 2: Update `crates/workspace/src/lib.rs`** to add the `dispatch` module:

```rust
//! Editor state, action enum, and dispatcher. Phase 3-ready scaffold.

pub mod action;
pub mod context;
pub mod dispatch;
pub mod state;

pub use action::Action;
pub use context::{Context, StatusLine, Viewport};
pub use dispatch::dispatch;
pub use state::EditorState;
```

- [ ] **Step 3: Verify**

Run: `cargo build --workspace`
Expected: success.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: success.

- [ ] **Step 4: Commit**

```bash
git add crates/workspace/
git commit -m "workspace: add dispatch fn and command handlers"
```

---

## Task 3: Scaffold `teditor-config` — `Chord`, `Keymap`, `default_keymap`

**Goal:** Config crate provides chord-keyed action lookup. Crate compiles standalone;
not yet used by app.

**Files:**
- Modify: `crates/config/Cargo.toml`
- Create: `crates/config/src/keymap.rs`
- Modify: `crates/config/src/lib.rs`

- [ ] **Step 1: Add dependencies to `crates/config/Cargo.toml`**

Replace empty `[dependencies]` with:

```toml
[dependencies]
teditor-workspace.workspace = true
crossterm.workspace = true
```

- [ ] **Step 2: Create `crates/config/src/keymap.rs`**

```rust
//! Chord → Action mapping. The default keymap mirrors the Phase-1/2 binding set.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyModifiers};
use teditor_workspace::Action;

#[derive(Copy, Clone, Hash, Eq, PartialEq)]
pub struct Chord {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl Chord {
    pub fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        Self { code, mods }
    }
}

pub struct Keymap {
    bindings: HashMap<Chord, Action>,
}

impl Keymap {
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    pub fn bind(&mut self, chord: Chord, action: Action) {
        self.bindings.insert(chord, action);
    }

    pub fn lookup(&self, chord: Chord) -> Option<Action> {
        self.bindings.get(&chord).cloned()
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Self::new()
    }
}

const C: KeyModifiers = KeyModifiers::CONTROL;
const S: KeyModifiers = KeyModifiers::SHIFT;
const A: KeyModifiers = KeyModifiers::ALT;
const NONE: KeyModifiers = KeyModifiers::NONE;

fn chord(code: KeyCode, mods: KeyModifiers) -> Chord {
    Chord::new(code, mods)
}

fn ch(c: char) -> KeyCode {
    KeyCode::Char(c)
}

pub fn default_keymap() -> Keymap {
    let mut k = Keymap::new();

    // app + file
    k.bind(chord(ch('q'), C), Action::Quit);
    k.bind(chord(ch('s'), C), Action::Save);

    // history
    k.bind(chord(ch('z'), C), Action::Undo);
    k.bind(chord(ch('z'), C | S), Action::Redo);
    k.bind(chord(ch('y'), C), Action::Redo);

    // selection
    k.bind(chord(ch('a'), C), Action::SelectAll);

    // clipboard
    k.bind(chord(ch('c'), C), Action::Copy);
    k.bind(chord(ch('x'), C), Action::Cut);
    k.bind(chord(ch('v'), C), Action::Paste);

    // motion — both extend variants per chord
    for &(extend, sm) in &[(false, NONE), (true, S)] {
        // ctrl + arrows: line/doc bounds
        k.bind(chord(KeyCode::Left, C | sm), Action::MoveLineStart { extend });
        k.bind(chord(KeyCode::Right, C | sm), Action::MoveLineEnd { extend });
        k.bind(chord(KeyCode::Up, C | sm), Action::MoveDocStart { extend });
        k.bind(chord(KeyCode::Down, C | sm), Action::MoveDocEnd { extend });

        // alt + arrows: word motion
        k.bind(chord(KeyCode::Left, A | sm), Action::MoveWordLeft { extend });
        k.bind(chord(KeyCode::Right, A | sm), Action::MoveWordRight { extend });

        // plain arrows
        k.bind(chord(KeyCode::Left, sm), Action::MoveLeft { extend });
        k.bind(chord(KeyCode::Right, sm), Action::MoveRight { extend });
        k.bind(chord(KeyCode::Up, sm), Action::MoveUp { extend });
        k.bind(chord(KeyCode::Down, sm), Action::MoveDown { extend });

        // home / end / pageup / pagedown
        k.bind(chord(KeyCode::Home, sm), Action::MoveLineStart { extend });
        k.bind(chord(KeyCode::End, sm), Action::MoveLineEnd { extend });
        k.bind(chord(KeyCode::PageUp, sm), Action::PageUp { extend });
        k.bind(chord(KeyCode::PageDown, sm), Action::PageDown { extend });
    }

    // edits
    k.bind(chord(KeyCode::Backspace, NONE), Action::DeleteBack { word: false });
    k.bind(chord(KeyCode::Backspace, A), Action::DeleteBack { word: true });
    k.bind(chord(KeyCode::Delete, NONE), Action::DeleteForward { word: false });
    k.bind(chord(KeyCode::Delete, A), Action::DeleteForward { word: true });
    k.bind(chord(KeyCode::Enter, NONE), Action::InsertNewline);
    k.bind(chord(KeyCode::Tab, NONE), Action::InsertTab);

    k
}

/// Normalize a `KeyEvent` into a `Chord` suitable for keymap lookup.
/// Lowercases ASCII alphabetic chars (so Ctrl+s and Ctrl+S share a chord),
/// preserving all modifier bits as crossterm reports them.
pub fn chord_from_key(code: KeyCode, mods: KeyModifiers) -> Chord {
    let code = match code {
        KeyCode::Char(c) if c.is_ascii_alphabetic() => KeyCode::Char(c.to_ascii_lowercase()),
        other => other,
    };
    Chord::new(code, mods)
}
```

- [ ] **Step 3: Replace `crates/config/src/lib.rs`**

```rust
//! Settings, themes, keymap. Phase 4+ will add settings/themes; today we
//! expose a hardcoded default keymap.

pub mod keymap;

pub use keymap::{Chord, Keymap, chord_from_key, default_keymap};
```

- [ ] **Step 4: Verify**

Run: `cargo build --workspace`
Expected: success.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: success.

- [ ] **Step 5: Commit**

```bash
git add crates/config/
git commit -m "config: add Chord, Keymap, default_keymap"
```

---

## Task 4: Split `crates/ui/src/lib.rs` into `editor.rs` + `status.rs`

**Goal:** Move editor render into `editor.rs` (verbatim). Add `status.rs` with
`StatusInfo` value-type + `render_status` function so the app no longer reaches into a
full `App` reference. App side adapts in Task 7.

**Files:**
- Create: `crates/ui/src/editor.rs`
- Create: `crates/ui/src/status.rs`
- Modify: `crates/ui/src/lib.rs`

- [ ] **Step 1: Create `crates/ui/src/editor.rs`**

Move the entire current contents of `crates/ui/src/lib.rs` (lines 1–158) into this new
file unchanged. The file already has no `mod` declarations, so paste verbatim. The first
line is `//! Editor view: pure render of buffer text...`.

- [ ] **Step 2: Create `crates/ui/src/status.rs`**

```rust
//! Status line: pure render of a `StatusInfo` value.
//!
//! `StatusInfo` is built by the binary from its `App` state; this module knows
//! nothing about `App`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Paragraph;

pub struct StatusInfo<'a> {
    pub path: Option<&'a str>,
    pub dirty: bool,
    pub line: usize,
    pub col: usize,
    pub sel_len: usize,
    pub message: Option<&'a str>,
}

pub fn render_status(info: &StatusInfo<'_>, area: Rect, frame: &mut Frame<'_>) {
    let path = info.path.unwrap_or("[scratch]");
    let dirty = if info.dirty { " [+]" } else { "" };
    let sel = if info.sel_len > 0 {
        format!(" ({} sel)", info.sel_len)
    } else {
        String::new()
    };

    let left = format!(" {}{}  {}:{}{}", path, dirty, info.line, info.col, sel);
    let right = info
        .message
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Ctrl+S save · Ctrl+Q quit".to_string());

    let total = area.width as usize;
    let pad = total.saturating_sub(left.chars().count() + right.chars().count() + 1);
    let text = format!("{}{}{} ", left, " ".repeat(pad), right);

    let para = Paragraph::new(text).style(Style::default().bg(Color::DarkGray).fg(Color::White));
    frame.render_widget(para, area);
}
```

- [ ] **Step 3: Replace `crates/ui/src/lib.rs`** with re-exports only:

```rust
//! Ratatui widgets for the editor view and status line.

pub mod editor;
pub mod status;

pub use editor::{EditorRenderResult, EditorView, render_editor};
pub use status::{StatusInfo, render_status};
```

- [ ] **Step 4: Verify**

Run: `cargo build --workspace`
Expected: success. App still references `EditorView` / `render_editor` via `teditor_ui::*`
and those re-exports remain, so the binary keeps building.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: success.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/
git commit -m "ui: split lib.rs into editor.rs + status.rs (introduce StatusInfo)"
```

---

## Task 5: Wire `EditorState` and `StatusLine` into `crates/app/src/main.rs`

**Goal:** The `App` struct uses `EditorState` instead of inline buffer/selection/
target_col/scroll_top. Replaces the `App.status: Option<String>` with `StatusLine`.
Removes the local `move_to`/`move_vertical`/`apply_tx` free functions in favor of
`EditorState` methods. Builds and runs identically. Old `handle_key` / `handle_mouse`
still in place.

**Files:**
- Modify: `crates/app/Cargo.toml`
- Modify: `crates/app/src/main.rs`

- [ ] **Step 1: Add deps to `crates/app/Cargo.toml`**

Add to the `[dependencies]` section:

```toml
teditor-workspace.workspace = true
teditor-config.workspace = true
```

(Order alongside the existing `teditor-buffer.workspace = true` line.)

- [ ] **Step 2: Edit `crates/app/src/main.rs` — imports**

Add a `teditor_workspace` use line. Keep the existing buffer/ui imports — local helpers
(`replace_selection`, `delete_primary_or`, `do_cut`) still reference `Transaction` and
`replace_selection_tx`; Task 6 removes them.

Result:

```rust
use teditor_buffer::{Buffer, Range, Selection, Transaction, delete_range_tx, replace_selection_tx};
use teditor_ui::{EditorView, render_editor};
use teditor_workspace::{EditorState, StatusLine};
```

- [ ] **Step 3: Refactor the `App` struct** (around lines 28–44):

```rust
struct App {
    editor: EditorState,
    status: StatusLine,
    quit: bool,
    last_editor_area: Rect,
    last_gutter_width: u16,
    clipboard: Option<arboard::Clipboard>,
    /// Holds the watcher alive; events flow through `disk_rx`.
    _watcher: Option<notify::RecommendedWatcher>,
    disk_rx: Option<mpsc::Receiver<()>>,
    /// True when an external change has been signaled but we haven't reconciled.
    disk_changed_pending: bool,
}
```

- [ ] **Step 4: Refactor `App::new`** (around lines 46–73):

```rust
impl App {
    fn new(path: Option<PathBuf>) -> Result<Self> {
        let buffer = match path.clone() {
            Some(p) => Buffer::from_path(p)?,
            None => Buffer::empty(),
        };
        let clipboard = arboard::Clipboard::new().ok();

        let (watcher, rx) = match path.as_deref() {
            Some(p) if p.exists() => spawn_watcher(p)
                .ok()
                .map(|(w, r)| (Some(w), Some(r)))
                .unwrap_or((None, None)),
            _ => (None, None),
        };

        Ok(Self {
            editor: EditorState::new(buffer),
            status: StatusLine::default(),
            quit: false,
            last_editor_area: Rect::default(),
            last_gutter_width: 0,
            clipboard,
            _watcher: watcher,
            disk_rx: rx,
            disk_changed_pending: false,
        })
    }

    fn primary(&self) -> Range {
        self.editor.primary()
    }

    fn set_status(&mut self, s: impl Into<String>) {
        self.status.set(s);
    }

    fn clear_status(&mut self) {
        self.status.clear();
    }
}
```

- [ ] **Step 5: Replace every `app.buffer` → `app.editor.buffer`,
`app.selection` → `app.editor.selection`, `app.target_col` →
`app.editor.target_col`, `app.scroll_top` → `app.editor.scroll_top`** throughout the
file.

Search-and-replace using your editor (verify each hit):

- `app.buffer` → `app.editor.buffer`
- `app.selection` → `app.editor.selection`
- `app.target_col` → `app.editor.target_col`
- `app.scroll_top` → `app.editor.scroll_top`

- [ ] **Step 6: Delete the local `move_to`, `move_vertical`, `apply_tx` free functions**
(roughly lines 234–286 of the original file) and replace their callers:

- `move_to(app, idx, extend, sticky)` → `app.editor.move_to(idx, extend, sticky)`
- `move_vertical(app, down, extend)` → `app.editor.move_vertical(down, extend)`
- `apply_tx(app, tx)` calls inside `replace_selection`, `delete_primary_or`, `do_cut`
  remain — but change them to `app.editor.apply_tx(tx); app.clear_status();` (the
  status clear used to be inside the local `apply_tx`).

The local `replace_selection` and `delete_primary_or` helpers in main.rs still exist
for now; they call `app.editor.apply_tx(tx)` and then `app.clear_status()`. These will
be removed in Task 6.

- [ ] **Step 7: Update `render_status`** at the bottom of `render` (around line 193)
to read through `app.status.get()`:

In `fn render_status`, replace:

```rust
let right = app
    .status
    .clone()
    .unwrap_or_else(|| "Ctrl+S save · Ctrl+Q quit".to_string());
```

with:

```rust
let right = app
    .status
    .get()
    .map(|s| s.to_string())
    .unwrap_or_else(|| "Ctrl+S save · Ctrl+Q quit".to_string());
```

- [ ] **Step 8: Verify build**

Run: `cargo build --workspace`
Expected: success.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: success.

- [ ] **Step 9: Manual smoke test (quick)**

```bash
cargo run -- /tmp/refactor-t5.txt
```
Type a few characters, save with Ctrl+S, quit with Ctrl+Q. Verify behavior identical.

- [ ] **Step 10: Commit**

```bash
git add crates/app/Cargo.toml crates/app/src/main.rs
git commit -m "app: route editor state through workspace::EditorState"
```

---

## Task 6: Replace `handle_key` / `handle_mouse` with `Keymap` + `dispatch`

**Goal:** Input handling becomes: build a chord, look up an Action, dispatch (or fall
back to `Action::InsertChar` for plain typing). Mouse becomes
`Action::ClickAt`/`DragAt`/`ScrollUp`/`ScrollDown`. The local clipboard, edit, motion
handlers in main.rs are deleted.

**Files:**
- Modify: `crates/app/src/main.rs`

- [ ] **Step 1: Update imports**

Replace the previous `use teditor_buffer::*` and `use teditor_workspace::*` lines with:

```rust
use teditor_buffer::Buffer;
use teditor_config::{Keymap, chord_from_key, default_keymap};
use teditor_ui::{EditorView, render_editor};
use teditor_workspace::{Action, Context, EditorState, StatusLine, Viewport, dispatch};
```

`Buffer` stays (used by `App::new`). `Range`, `Selection`, `Transaction`,
`delete_range_tx`, `replace_selection_tx` are no longer referenced from `app/main.rs`.
Drop them.

- [ ] **Step 2: Add `keymap: Keymap` to `App` struct**

```rust
struct App {
    editor: EditorState,
    keymap: Keymap,
    status: StatusLine,
    quit: bool,
    last_editor_area: Rect,
    last_gutter_width: u16,
    clipboard: Option<arboard::Clipboard>,
    _watcher: Option<notify::RecommendedWatcher>,
    disk_rx: Option<mpsc::Receiver<()>>,
    disk_changed_pending: bool,
}
```

In `App::new`, add `keymap: default_keymap(),` to the struct initializer.

- [ ] **Step 3: Replace `handle_key`** (entire body, roughly lines 292–472) with the
chord-based version:

```rust
fn handle_key(code: KeyCode, mods: KeyModifiers, app: &mut App) {
    // Disk-pending input gate: special-case Ctrl+R / Ctrl+K. Other chords
    // pass through to the keymap normally.
    if app.disk_changed_pending && mods.contains(KeyModifiers::CONTROL) {
        let lower = match code {
            KeyCode::Char(c) => Some(c.to_ascii_lowercase()),
            _ => None,
        };
        match lower {
            Some('r') => {
                let action = Action::ReloadFromDisk;
                run_action(app, action);
                return;
            }
            Some('k') => {
                let action = Action::KeepBufferIgnoreDisk;
                run_action(app, action);
                return;
            }
            _ => {}
        }
    }

    let chord = chord_from_key(code, mods);
    if let Some(action) = app.keymap.lookup(chord) {
        run_action(app, action);
        return;
    }

    // Fallback: plain typing. Only KeyCode::Char without Ctrl/Alt.
    if let KeyCode::Char(c) = code {
        if !mods.contains(KeyModifiers::CONTROL) && !mods.contains(KeyModifiers::ALT) {
            run_action(app, Action::InsertChar(c));
        }
    }
}
```

- [ ] **Step 4: Replace `handle_mouse`** with Action dispatch:

```rust
fn handle_mouse(me: MouseEvent, app: &mut App) {
    match me.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let extend = me.modifiers.contains(KeyModifiers::SHIFT);
            run_action(app, Action::ClickAt {
                col: me.column,
                row: me.row,
                extend,
            });
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            run_action(app, Action::DragAt {
                col: me.column,
                row: me.row,
            });
        }
        MouseEventKind::ScrollUp => run_action(app, Action::ScrollUp),
        MouseEventKind::ScrollDown => run_action(app, Action::ScrollDown),
        _ => {}
    }
}
```

- [ ] **Step 5: Add `run_action` helper**

```rust
fn run_action(app: &mut App, action: Action) {
    let viewport = Viewport {
        x: app.last_editor_area.x,
        y: app.last_editor_area.y,
        width: app.last_editor_area.width,
        height: app.last_editor_area.height,
        gutter_width: app.last_gutter_width,
    };
    let mut cx = Context {
        editor: &mut app.editor,
        clipboard: &mut app.clipboard,
        status: &mut app.status,
        quit: &mut app.quit,
        disk_changed_pending: &mut app.disk_changed_pending,
        viewport,
    };
    dispatch(action, &mut cx);
}
```

- [ ] **Step 6: Delete dead helpers from `crates/app/src/main.rs`:**

- `fn replace_selection`
- `fn delete_primary_or`
- `fn current_line_span`
- `fn do_copy`
- `fn do_cut`
- `fn do_paste`
- `fn click_to_char_idx`
- `fn page_step`

(All now live in `crates/workspace/src/dispatch.rs`.)

- [ ] **Step 7: Simplify `drain_disk_events`** — the success arm currently calls
`app.buffer.reload_from_disk()`, etc. Replace its success arm to dispatch the Action so
the logic stays in one place:

Find the `drain_disk_events` function. Replace its body (roughly the `if app.buffer.dirty()` block) with:

```rust
fn drain_disk_events(app: &mut App) {
    let Some(rx) = app.disk_rx.as_ref() else { return };
    let mut got = false;
    while rx.try_recv().is_ok() {
        got = true;
    }
    if !got {
        return;
    }

    if app.editor.buffer.dirty() {
        app.disk_changed_pending = true;
        app.set_status("Disk changed (buffer modified) · Ctrl+R reload, Ctrl+K keep");
    } else {
        run_action(app, Action::ReloadFromDisk);
    }
}
```

(The auto-reload path now goes through the Action handler, which clears
`disk_changed_pending` and sets status. The `set_status` path for the dirty case stays
inline because it sets the prompt flag.)

- [ ] **Step 8: Verify build**

Run: `cargo build --workspace`
Expected: success.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: success.

- [ ] **Step 9: Manual smoke test**

```bash
cargo run -- /tmp/refactor-t6.txt
```

Type, motion (arrows + ctrl/alt/shift combos), copy/cut/paste, undo/redo, save, mouse
click and drag, scroll wheel. Trigger external change with another shell:
`echo X >> /tmp/refactor-t6.txt`. Verify reload prompt appears, Ctrl+K dismisses, then
re-trigger and Ctrl+R reloads.

- [ ] **Step 10: Commit**

```bash
git add crates/app/src/main.rs
git commit -m "app: route input through Keymap + dispatch"
```

---

## Task 7: Adopt `StatusInfo` for status rendering

**Goal:** Replace the inline `render_status` in `app/main.rs` with a call to
`teditor_ui::render_status` using a built `StatusInfo`. Removes the only path where the
ui crate would have needed an `App` reference.

**Files:**
- Modify: `crates/app/src/main.rs`

- [ ] **Step 1: Update import**

Add `StatusInfo, render_status as render_status_widget` to the `teditor_ui` import:

```rust
use teditor_ui::{EditorView, EditorRenderResult, StatusInfo, render_editor, render_status as render_status_widget};
```

(`EditorRenderResult` may already be referenced indirectly; if the binary doesn't use
the name, omit it.)

- [ ] **Step 2: Replace the body of `fn render_status`** in main.rs with `StatusInfo`
construction + delegation:

```rust
fn render_status(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let path_str = app
        .editor
        .buffer
        .path()
        .map(|p| p.display().to_string());
    let head = app.primary().head;
    let info = StatusInfo {
        path: path_str.as_deref(),
        dirty: app.editor.buffer.dirty(),
        line: app.editor.buffer.line_of_char(head) + 1,
        col: app.editor.buffer.col_of_char(head) + 1,
        sel_len: app.primary().len(),
        message: app.status.get(),
    };
    render_status_widget(&info, area, frame);
}
```

- [ ] **Step 3: Remove the now-unused `Color`, `Style`, `Paragraph` imports**

Find any imports that became unused after the body change:

- `use ratatui::style::{Color, Style};` — remove if no other use remains.
- `use ratatui::widgets::Paragraph;` — remove if no other use remains.

Use `cargo build` to confirm; rustc will name the offenders.

- [ ] **Step 4: Verify**

Run: `cargo build --workspace`
Expected: success.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: success.

- [ ] **Step 5: Manual smoke (quick)**

```bash
cargo run -- /tmp/refactor-t7.txt
```
Confirm status line still shows path, line:col, sel count, and Ctrl+S/Ctrl+Q hint.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/main.rs
git commit -m "app: render status via teditor_ui::StatusInfo"
```

---

## Task 8: Split `crates/app/src/main.rs` into modules

**Goal:** Carve `main.rs` into `app.rs`, `events.rs`, `clipboard.rs`, `watcher.rs`,
`render.rs`, with `main.rs` reduced to a thin entry point. Each step is one extraction;
build stays green between steps.

**Files:**
- Create: `crates/app/src/watcher.rs`
- Create: `crates/app/src/clipboard.rs`
- Create: `crates/app/src/render.rs`
- Create: `crates/app/src/events.rs`
- Create: `crates/app/src/app.rs`
- Modify: `crates/app/src/main.rs`

**Ordering note:** because `drain_disk_events` references `App` and `run_action`,
which don't exist as separate modules yet, we extract `spawn_watcher` first (no
`App` dependency), then `clipboard`, then `app`, then `events`, then `render`, and
finally relocate `drain_disk_events` into `watcher.rs` once both `app` and `events`
modules exist.

### Sub-task 8a: Extract `spawn_watcher` into `watcher.rs`

- [ ] **Step 1: Create `crates/app/src/watcher.rs`**

```rust
//! Filesystem watch. `drain_disk_events` is added in sub-task 8f after `app`
//! and `events` modules exist.

use std::path::Path;
use std::sync::mpsc;

use anyhow::Result;
use notify::{RecursiveMode, Watcher};

pub fn spawn_watcher(path: &Path) -> Result<(notify::RecommendedWatcher, mpsc::Receiver<()>)> {
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            use notify::EventKind::*;
            if matches!(ev.kind, Modify(_) | Create(_) | Remove(_)) {
                let _ = tx.send(());
            }
        }
    })?;
    let watch_target = path.parent().unwrap_or_else(|| Path::new("."));
    watcher.watch(watch_target, RecursiveMode::NonRecursive)?;
    Ok((watcher, rx))
}
```

- [ ] **Step 2: Add `mod watcher;` to top of `crates/app/src/main.rs`**

After the `use std::...` block, add:

```rust
mod watcher;
```

Update the call site in `App::new` to use `watcher::spawn_watcher(p)` instead of the
local `spawn_watcher(p)`. Delete the old `fn spawn_watcher` from `main.rs`.

- [ ] **Step 3: Verify, commit**

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
git add crates/app/src/
git commit -m "app: extract watcher::spawn_watcher"
```

### Sub-task 8b: Extract `clipboard.rs`

- [ ] **Step 1: Create `crates/app/src/clipboard.rs`**

```rust
//! System-clipboard initialization. Errors are swallowed: when no display
//! server is available, copy/cut/paste operate without a clipboard backend
//! (the Action handlers set status accordingly).

pub fn init() -> Option<arboard::Clipboard> {
    arboard::Clipboard::new().ok()
}
```

- [ ] **Step 2: Add `mod clipboard;` to `main.rs`**, and update `App::new` to call
`clipboard::init()` instead of `arboard::Clipboard::new().ok()`.

- [ ] **Step 3: Verify, commit**

```bash
cargo build --workspace && cargo clippy --workspace --all-targets -- -D warnings
git add crates/app/src/
git commit -m "app: extract clipboard::init"
```

### Sub-task 8c: Extract `app.rs` (App struct + run loop)

- [ ] **Step 1: Create `crates/app/src/app.rs`**

Move the `App` struct, `impl App`, the `run<B: Backend>` function, and (yes,
duplicately, until 8f) the local `drain_disk_events` function here. Headers and imports:

```rust
//! Editor application: owns terminal-side state and runs the event loop.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::Rect;
use teditor_buffer::{Buffer, Range};
use teditor_config::{Keymap, default_keymap};
use teditor_workspace::{Action, EditorState, StatusLine};

use crate::clipboard;
use crate::events::{handle_event, run_action};
use crate::render::render;
use crate::watcher::spawn_watcher;

pub struct App {
    pub editor: EditorState,
    pub keymap: Keymap,
    pub status: StatusLine,
    pub quit: bool,
    pub last_editor_area: Rect,
    pub last_gutter_width: u16,
    pub clipboard: Option<arboard::Clipboard>,
    pub _watcher: Option<notify::RecommendedWatcher>,
    pub disk_rx: Option<mpsc::Receiver<()>>,
    pub disk_changed_pending: bool,
}

impl App {
    pub fn new(path: Option<PathBuf>) -> Result<Self> {
        let buffer = match path.clone() {
            Some(p) => Buffer::from_path(p)?,
            None => Buffer::empty(),
        };
        let clipboard = clipboard::init();

        let (watcher, rx) = match path.as_deref() {
            Some(p) if p.exists() => spawn_watcher(p)
                .ok()
                .map(|(w, r)| (Some(w), Some(r)))
                .unwrap_or((None, None)),
            _ => (None, None),
        };

        Ok(Self {
            editor: EditorState::new(buffer),
            keymap: default_keymap(),
            status: StatusLine::default(),
            quit: false,
            last_editor_area: Rect::default(),
            last_gutter_width: 0,
            clipboard,
            _watcher: watcher,
            disk_rx: rx,
            disk_changed_pending: false,
        })
    }

    pub fn primary(&self) -> Range {
        self.editor.primary()
    }
}

pub fn run<B: Backend>(terminal: &mut Terminal<B>, path: Option<PathBuf>) -> Result<()> {
    let mut app = App::new(path)?;

    while !app.quit {
        drain_disk_events(&mut app);
        terminal.draw(|frame| render(frame, &mut app))?;

        if event::poll(Duration::from_millis(100))? {
            handle_event(event::read()?, &mut app);
        }
    }
    Ok(())
}

// Temporary home for drain_disk_events until sub-task 8f moves it to
// `watcher.rs`. Identical body to today's main.rs version.
fn drain_disk_events(app: &mut App) {
    let Some(rx) = app.disk_rx.as_ref() else { return };
    let mut got = false;
    while rx.try_recv().is_ok() {
        got = true;
    }
    if !got {
        return;
    }
    if app.editor.buffer.dirty() {
        app.disk_changed_pending = true;
        app.status
            .set("Disk changed (buffer modified) · Ctrl+R reload, Ctrl+K keep");
    } else {
        run_action(app, Action::ReloadFromDisk);
    }
}
```

- [ ] **Step 2: Add `pub mod app;` to `main.rs` and remove the original `App`,
`impl App`, `run`, `drain_disk_events` from `main.rs`.** `main.rs` now imports
`crate::app::run` and uses it.

This step also forward-references `crate::events::*` and `crate::render::render`, which
don't exist yet — proceed to 8d and 8e immediately; do not commit between 8c, 8d, 8e.

(Build will fail until 8d and 8e land. Treat 8c+8d+8e as a single commit.)

### Sub-task 8d: Extract `events.rs`

- [ ] **Step 1: Create `crates/app/src/events.rs`**

Move `handle_event`, `handle_key`, `handle_mouse`, and `run_action` from `main.rs` here.
Headers:

```rust
//! Input events: KeyEvent → Chord → Action via keymap; MouseEvent → Action
//! directly. The disk-pending input gate is enforced here.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind};
use teditor_config::chord_from_key;
use teditor_workspace::{Action, Context, Viewport, dispatch};

use crate::app::App;

pub fn handle_event(ev: Event, app: &mut App) {
    match ev {
        Event::Key(KeyEvent { code, modifiers, kind, .. })
            if kind == KeyEventKind::Press || kind == KeyEventKind::Repeat =>
        {
            handle_key(code, modifiers, app);
        }
        Event::Mouse(me) => handle_mouse(me, app),
        Event::Resize(_, _) => {}
        _ => {}
    }
}

pub fn handle_key(code: KeyCode, mods: KeyModifiers, app: &mut App) {
    if app.disk_changed_pending && mods.contains(KeyModifiers::CONTROL) {
        let lower = match code {
            KeyCode::Char(c) => Some(c.to_ascii_lowercase()),
            _ => None,
        };
        match lower {
            Some('r') => {
                run_action(app, Action::ReloadFromDisk);
                return;
            }
            Some('k') => {
                run_action(app, Action::KeepBufferIgnoreDisk);
                return;
            }
            _ => {}
        }
    }

    let chord = chord_from_key(code, mods);
    if let Some(action) = app.keymap.lookup(chord) {
        run_action(app, action);
        return;
    }

    if let KeyCode::Char(c) = code {
        if !mods.contains(KeyModifiers::CONTROL) && !mods.contains(KeyModifiers::ALT) {
            run_action(app, Action::InsertChar(c));
        }
    }
}

pub fn handle_mouse(me: MouseEvent, app: &mut App) {
    match me.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let extend = me.modifiers.contains(KeyModifiers::SHIFT);
            run_action(app, Action::ClickAt {
                col: me.column,
                row: me.row,
                extend,
            });
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            run_action(app, Action::DragAt {
                col: me.column,
                row: me.row,
            });
        }
        MouseEventKind::ScrollUp => run_action(app, Action::ScrollUp),
        MouseEventKind::ScrollDown => run_action(app, Action::ScrollDown),
        _ => {}
    }
}

pub fn run_action(app: &mut App, action: Action) {
    let viewport = Viewport {
        x: app.last_editor_area.x,
        y: app.last_editor_area.y,
        width: app.last_editor_area.width,
        height: app.last_editor_area.height,
        gutter_width: app.last_gutter_width,
    };
    let mut cx = Context {
        editor: &mut app.editor,
        clipboard: &mut app.clipboard,
        status: &mut app.status,
        quit: &mut app.quit,
        disk_changed_pending: &mut app.disk_changed_pending,
        viewport,
    };
    dispatch(action, &mut cx);
}
```

- [ ] **Step 2: Add `pub mod events;` to `main.rs`. Delete `handle_event`,
`handle_key`, `handle_mouse`, `run_action` from `main.rs`.**

### Sub-task 8e: Extract `render.rs`

- [ ] **Step 1: Create `crates/app/src/render.rs`**

Move `fn render` and `fn render_status` (the wrapper that builds `StatusInfo`) from
`main.rs` here:

```rust
//! Frame composition: editor area + status line. Translates `App` state into
//! the `StatusInfo` and `EditorView` value types that the `ui` crate consumes.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use teditor_ui::{EditorView, StatusInfo, render_editor, render_status as render_status_widget};

use crate::app::App;

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());
    let editor_area = chunks[0];
    let status_area = chunks[1];
    app.last_editor_area = editor_area;

    let cur_line = app.editor.buffer.line_of_char(app.primary().head);
    let visible = editor_area.height as usize;
    if visible > 0 {
        if cur_line < app.editor.scroll_top {
            app.editor.scroll_top = cur_line;
        } else if cur_line >= app.editor.scroll_top + visible {
            app.editor.scroll_top = cur_line + 1 - visible;
        }
    }

    let view = EditorView {
        buffer: &app.editor.buffer,
        selection: &app.editor.selection,
        scroll_top: app.editor.scroll_top,
    };
    let r = render_editor(view, editor_area, frame);
    app.last_gutter_width = r.gutter_width;
    if let Some((x, y)) = r.cursor_screen {
        frame.set_cursor_position((x, y));
    }

    render_status(frame, status_area, app);
}

fn render_status(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let path_str = app.editor.buffer.path().map(|p| p.display().to_string());
    let head = app.primary().head;
    let info = StatusInfo {
        path: path_str.as_deref(),
        dirty: app.editor.buffer.dirty(),
        line: app.editor.buffer.line_of_char(head) + 1,
        col: app.editor.buffer.col_of_char(head) + 1,
        sel_len: app.primary().len(),
        message: app.status.get(),
    };
    render_status_widget(&info, area, frame);
}
```

- [ ] **Step 2: Add `pub mod render;` to `main.rs`. Delete `fn render` and
`fn render_status` from `main.rs`.**

- [ ] **Step 3: Verify and commit (combined 8c–8e)**

`main.rs` should now be roughly: imports for terminal setup, `mod app;`, `mod
clipboard;`, `mod events;`, `mod render;`, `mod watcher;`, the `fn main` entry point.

Run: `cargo build --workspace`
Expected: success.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: success.

Manual smoke test:

```bash
cargo run -- /tmp/refactor-t8e.txt
```

Quick spot check: type, save, mouse click, quit.

```bash
git add crates/app/src/
git commit -m "app: split main.rs into app/events/render modules"
```

### Sub-task 8f: Move `drain_disk_events` to `watcher.rs`

- [ ] **Step 1: Cut `fn drain_disk_events` from `crates/app/src/app.rs` and paste
into `crates/app/src/watcher.rs`**, replacing watcher.rs's contents:

```rust
//! Filesystem watch + reconciliation.

use std::path::Path;
use std::sync::mpsc;

use anyhow::Result;
use notify::{RecursiveMode, Watcher};
use teditor_workspace::Action;

use crate::app::App;
use crate::events::run_action;

pub fn spawn_watcher(path: &Path) -> Result<(notify::RecommendedWatcher, mpsc::Receiver<()>)> {
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            use notify::EventKind::*;
            if matches!(ev.kind, Modify(_) | Create(_) | Remove(_)) {
                let _ = tx.send(());
            }
        }
    })?;
    let watch_target = path.parent().unwrap_or_else(|| Path::new("."));
    watcher.watch(watch_target, RecursiveMode::NonRecursive)?;
    Ok((watcher, rx))
}

pub fn drain_disk_events(app: &mut App) {
    let Some(rx) = app.disk_rx.as_ref() else { return };
    let mut got = false;
    while rx.try_recv().is_ok() {
        got = true;
    }
    if !got {
        return;
    }
    if app.editor.buffer.dirty() {
        app.disk_changed_pending = true;
        app.status
            .set("Disk changed (buffer modified) · Ctrl+R reload, Ctrl+K keep");
    } else {
        run_action(app, Action::ReloadFromDisk);
    }
}
```

- [ ] **Step 2: Update `app.rs`** — remove the local `drain_disk_events` and import
the watcher version:

In `crates/app/src/app.rs`, change `use crate::watcher::spawn_watcher;` to:

```rust
use crate::watcher::{drain_disk_events, spawn_watcher};
```

Confirm `run` already calls `drain_disk_events(&mut app);` — leave that line as-is.

- [ ] **Step 3: Verify**

Run: `cargo build --workspace`
Expected: success.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: success.

- [ ] **Step 4: Reduce `main.rs` to its final form**

```rust
use std::io::stdout;
use std::path::PathBuf;

use anyhow::Result;
use crossterm::execute;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

mod app;
mod clipboard;
mod events;
mod render;
mod watcher;

fn main() -> Result<()> {
    let path = std::env::args().nth(1).map(PathBuf::from);

    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(out))?;

    let result = app::run(&mut terminal, path);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    result
}
```

- [ ] **Step 5: Verify**

Run: `cargo build --workspace`
Expected: success.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: success.

- [ ] **Step 6: Final manual smoke test**

```bash
echo "hello world" > /tmp/refactor-final.txt
cargo run -- /tmp/refactor-final.txt
```

Walk through the full smoke test:

1. Cursor visible, line numbers visible.
2. Type `abc` — appears.
3. Arrow keys move cursor. Shift+Arrows extend selection.
4. Ctrl+Left/Right → line bounds; Ctrl+Up/Down → doc bounds; Alt+Left/Right → word.
5. Ctrl+A selects all.
6. Ctrl+C, place cursor elsewhere, Ctrl+V — pastes.
7. Type, then Ctrl+Z undoes; Ctrl+Shift+Z redoes; Ctrl+Y redoes.
8. Ctrl+S saves; status reads "saved".
9. From another terminal: `echo external >> /tmp/refactor-final.txt`. Status reads
   "Disk changed... Ctrl+R reload, Ctrl+K keep" if buffer is dirty; otherwise reloads
   silently.
10. Click in the editor area — cursor moves. Drag selects. Scroll wheel scrolls.
11. Ctrl+Q quits cleanly (terminal restored, no lingering raw mode).

If any behavior diverges from before the refactor, stop and investigate.

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/
git commit -m "app: move drain_disk_events to watcher; finalize main.rs"
```

---

## Final state

- `crates/app/src/main.rs` ~50 lines.
- `dispatch.rs` ~250 lines (the largest single file).
- All other new/split files under ~120 lines.
- No external dependencies added.
- `cargo build --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
  and the manual smoke test all pass.
- Behavior identical to pre-refactor.

Ready for Phase 3 (tabs/splits/panels), which will wrap `EditorState` in a layout-tree
node without touching the dispatcher or keymap.
