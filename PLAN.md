# teditor — TUI Coding IDE Plan

## Goal

Build a basic but high-performance TUI coding IDE in Rust, with tree-sitter and LSP support and an extensive plugin system (Lua initially).

---

## User instructions (raw)

> okay, here's the idea i have. i want to build a basic, but high-performance, tui coding ide, with tree sitter and lsp support, and extensive plugin system (probably using lua or javascript). i want to use rust as a main programming language for the ide itself. i did my own research and found ox as an interesting reference. i don't want anything fancy just basic features like.
> 1. main code editor functionality with natural editing. code editor shows currently opened document. code editor has features like:
>  a. universal text input and navigation, non-modal.
>  b. using mouse inputs to navigate, select and scroll text.
>  c. scroll bars and line counts.
>  d. right click support for context menus.
>  e. basic clipboard management using ctrl+c, ctrl+v, ctrl+x, ctrl+z, etc. or using context menus.
>  f. code navigation and selection using:
>   i. ctrl+harrows to jump to the end/beginning of the line
>   ii. ctrl+varrows to jump top/bottom of the document.
>   iii. alt+harrows to jump words.
>   iv. same actions with shift added also select text.
>   v. multicursor editing and navigation using the same actions above. multicursor activates using ctrl+alt+varrows.
>  g. tabs support. new tabs are only opened on user's request using shift+ctrl+t. users can switch tabs using shift+ctrl+[ or shift+ctrl+]. when new document is opened, replace current tab content, not open a new tab.
>  h. code highlighting support. treesitter.
>  j. splits like tmux/ghostty.
>  k. go to next/previous opened document using ctrl+alt+harrows.
> 2. left and right panels hosting plugin contents. panels should be toggleable using shift+ctrl+alt+harrows. focus between left panel <> editor <> right panel
> 3. command palette which can be opened using ctrl+shift+p.
> 4. lsp support.
>  a. code completion.
>  b. go to definition using context menu or command palette.
>  c. build/run project.
>  d. symbols(outline) navigation using ctrl+shift+o (same display style as command palette). ctrl+o for local document symbols.
>  e. documentation using context menu or command palette..
> 5. theme support.
> 6. settings page with things like shortcut config, plugin settings, themes, etc.
> 7. plugin support. plugins can:
>  a. contribute actions and shortcuts
>  b. render tui in left or right panels.
>  c. open documents in the editor (main split).
>  d. contribute custom editors.

---

## Scope reality check

Roughly a Helix + tmux + Zed-style plugin host with a slimmer feature surface. Achievable as a focused solo project if scope discipline holds.

## Don't fork ox. Treat it as reading material.

For these goals (tree-sitter + LSP + serious plugins) kaolinite's limits — no graphemes, no transactional events, line-cache rebuild on undo, regex-only highlighter — will hurt sooner than expected. Start clean.

Two things worth stealing from ox:
- The `FileLayout` recursive split tree pattern.
- The Lua-userdata-per-config-object pattern.

---

## Recommended stack

| Concern | Pick | Why |
|---|---|---|
| TUI | `ratatui` + `crossterm` | Mature widgets, mouse, large ecosystem. Don't hand-roll like ox. |
| Buffer | `ropey` directly + `unicode-segmentation` | Grapheme-aware from day 1. Build a thin transaction layer on top. |
| (alternative) | vendor `helix-core` | MPL-2.0 (file-level copyleft, OK to combine with permissive). Most mature ropey-based buffer in Rust. Multi-region selections, transactions, grapheme cursor — solved. |
| Syntax | `tree-sitter` + per-language grammar crates | Standard. Wire incremental edits to the buffer's transaction stream. |
| LSP client | `lsp-types` + own JSON-RPC client over child stdio | `tower-lsp` is for *servers*. Reference: `helix-lsp`. ~1.5k LOC of real work. |
| Async | `tokio` | LSP, file watch, plugin I/O all need it. Render loop stays sync. |
| Config/themes | `serde` + `toml` | Standard. Themes = TextMate-style scope → style table. |
| Clipboard | `arboard` | Cross-platform, no surprises. |
| File watch | `notify` | Standard. |
| Fuzzy match | `nucleo` (Helix's matcher) | Fast, quality scores, async-friendly. For command palette + symbols. |
| Plugins v1 | `mlua` (Lua 5.4) | Tiny, fast, mature. JS/WASM later if needed. |

**Plugin runtime decision:** pick **one** for v1. Lua is the right call — small binary, mature mlua, easy to sandbox by stripping `os`/`io` from globals. JS via `deno_core` pulls in V8 (~30MB). WASM via `wasmtime` is where things are headed (Zed) but TUI rendering ergonomics are rough. Ship Lua, add WASM in v2 if there's demand.

---

## Architecture seams

```
┌─ tokio runtime ────────────────────────────────────────────┐
│                                                            │
│  Input task ──► App state ◄── LSP tasks (one per server)   │
│  (crossterm)        │         File watcher                 │
│                     │         Plugin tasks                 │
│                     ▼                                      │
│              Workspace { Layout tree, Keymap, Commands }   │
│                     │                                      │
│             ┌───────┴─────────┐                            │
│             ▼                 ▼                            │
│         Buffer            PluginHost                       │
│         (ropey +          (mlua VMs,                       │
│          transactions +   sandboxed,                       │
│          undo +           render hooks)                    │
│          tree-sitter)                                      │
│                     │                                      │
│                     ▼                                      │
│                 Renderer (ratatui, sync, 60fps cap)        │
└────────────────────────────────────────────────────────────┘
```

### Three rules to enforce from day 1

1. **Buffer mutations are transactions.** Every edit produces a `Transaction { changes, selection_before, selection_after }`. Undo replays inverse transactions. LSP/tree-sitter consume the same stream. This is the single most important early decision — kaolinite gets this wrong.
2. **Layout tree is recursive and generic.**
   ```
   enum Node {
       Split(Vertical | Horizontal, Vec<(Node, ratio)>),
       Pane(Tabs),
       Panel(PluginPanel),
   }
   ```
   Don't hardcode "left panel / right panel" — they're just panes pinned to edges with a toggle.
3. **Render is pure.** `fn render(state: &App, frame: &mut Frame)`. State mutations only happen in the input/event-pump phase. Makes testing tractable.

---

## Responsiveness commitments

Non-negotiable UX rules. The whole point of building this is to not be slow.

1. **External file changes reflect immediately.** `notify` watcher → mpsc → app. If the buffer is unmodified, swap in fresh contents on the next frame. If dirty, surface a non-blocking "reload / keep / diff" prompt. Never silently desync from disk; never block the editor while reconciling.
2. **No blocking I/O on the render thread, ever.** File open, save, LSP request, plugin call — all dispatched to tokio tasks; results return via channels.
3. **Frame budget is 16ms.** Anything heavier (tree-sitter on huge files, LSP response handling, fuzzy-match over a large symbol set) runs off-thread and streams results.
4. **Input is never queued behind work.** Keystrokes always preempt pending render or background results.

---

## Phased build plan

| Phase | Deliverable |
|---|---|
| 1. Skeleton | ratatui shell, ropey buffer, open/save, typing, arrows, mouse click, single pane |
| 2. Editing | Selection, clipboard, undo/redo (transactional), word motion, line ops, file watcher (reload-on-disk-change) |
| 3. Layout | Tabs, splits (recursive tree), side panels w/ toggle, focus management |
| 4. Syntax | Tree-sitter integration, theme loading, one language (Rust) end-to-end |
| 5. Palette | Command palette + keymap registry + nucleo fuzzy |
| 6. LSP | Client, diagnostics, completion popup, hover, goto-def, symbols (`ctrl+o`/`ctrl+shift+o`) |
| 7. Multi-cursor | Easier if Phase 2 used multi-region selections from the start |
| 8. Plugins | mlua host, action/keybind contributions, panel render API, custom-editor API |
| 9. Settings UI | In-editor settings page (just a special buffer with custom renderer) |

---

## Workspace skeleton

```
teditor/
├── Cargo.toml                 # workspace
├── crates/
│   ├── app/                   # binary, event loop, wires everything
│   ├── buffer/                # ropey + transactions + undo + grapheme cursor
│   ├── syntax/                # tree-sitter wrapper, incremental parse
│   ├── ui/                    # ratatui widgets (editor, palette, panels, popups)
│   ├── lsp/                   # JSON-RPC client + server lifecycle
│   ├── plugin/                # mlua host + sandbox + plugin API surface
│   ├── config/                # settings, themes, keymap (serde + toml)
│   └── workspace/             # layout tree, tabs, focus, command registry
└── PLAN.md
```

---

## Specific gotchas

1. **TUI right-click menus** — workable but feel off. Most terminals send mouse events fine via SGR mode, but stacking a popup *over* text means tracking a z-order layer in ratatui yourself.
2. **LSP completion popups** — same z-order issue. Plan a single "overlay" layer rendered last each frame.
3. **Tree-sitter on huge files** — fine <50k lines, stuttery beyond. Budget 4–8ms/frame for parsing; defer or chunk if over.
4. **Lua sandboxing** — mlua doesn't sandbox by default. Strip `os`, `io`, `package`, `debug`, `dofile`, `loadfile` from each plugin's environment. Start from `Lua::new_with_safe`, then build up exposed API.
5. **Splits + panels in one tree** — don't separate them. Both are just nodes in the layout tree with different render behavior. Zellij got tangled here early.
6. **Async/sync boundary** — render must be sync. Use `tokio::sync::mpsc` for LSP→app, file-watch→app, plugin→app. App owns the receiver; render loop drains it each frame.
7. **"Replace current tab on open" + tabs** — unusual default. Most editors append. Consider a config flag so it isn't surprising to users.

---

## Keymap (from spec, consolidated)

| Action | Binding |
|---|---|
| Jump line start/end | `Ctrl + ← / →` |
| Jump doc top/bottom | `Ctrl + ↑ / ↓` |
| Jump word left/right | `Alt + ← / →` |
| Extend selection | add `Shift` to any motion |
| Add cursor above/below | `Ctrl + Alt + ↑ / ↓` |
| New tab | `Ctrl + Shift + T` |
| Prev/next tab | `Ctrl + Shift + [` / `Ctrl + Shift + ]` |
| Prev/next document (across tabs) | `Ctrl + Alt + ← / →` |
| Toggle left/right panel | `Ctrl + Shift + Alt + ← / →` |
| Command palette | `Ctrl + Shift + P` |
| Workspace symbols | `Ctrl + Shift + O` |
| Document symbols | `Ctrl + O` |
| Copy / cut / paste / undo | `Ctrl + C / X / V / Z` |

Open question: redo binding (`Ctrl + Shift + Z` vs `Ctrl + Y`), save (`Ctrl + S`), close tab (`Ctrl + W`?), find (`Ctrl + F`?). Define before Phase 5.

---

## Open questions to resolve before coding

1. Redo / save / close-tab / find / find-in-project bindings — not in spec.
2. Which terminals are in the support matrix? (Affects mouse encoding choice, true-color assumption, undercurl support for diagnostics.)
3. Project model — single root, multi-root, no project (just files)? Affects LSP root resolution and file tree plugin design.
4. License for the project itself? (Affects whether vendoring `helix-core` is acceptable.)
5. Configuration file format — TOML assumed. Confirm.
6. Initial language target for Phase 4 — Rust assumed (rust-analyzer is the best-tested LSP server). Confirm.

---

## Next step

Phase 1 deliverable: scaffold the Cargo workspace and a runnable editor that opens a file, lets you type, and saves.
