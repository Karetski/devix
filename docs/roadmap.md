# devix — Roadmap

Status checklist against the original brief in `prompt.md`. Marks reflect what's wired end-to-end in the running editor, not what's been sketched in code. First pass — correct as needed.

Legend: ✅ done · 🚧 in progress · ⬜ not started

## 1. Editor

- ✅ a. Universal, non-modal text input and navigation
- ✅ b. Mouse navigation, selection, scroll
- 🚧 c. Line counts (gutter done) · scroll bars (not started)
- ⬜ d. Right-click context menus
- ✅ e. Clipboard (Ctrl+C/V/X/Z) and undo/redo on a transaction stream
- ✅ f. Navigation/selection bindings (Ctrl/Alt/Shift arrows, multicursor via Ctrl+Alt+↑/↓)
- ✅ g. Tabs (Ctrl+Shift+T, Ctrl+Shift+[/], replace-current-tab on open)
- ✅ h. Tree-sitter highlighting
- ✅ j. Splits (tmux/ghostty-style, recursive)
- ✅ k. Cross-tab document navigation (Ctrl+Alt+←/→)

## 2. Side panels

- ✅ Left/right panels host plugin content
- ✅ Toggle with Ctrl+Shift+Alt+←/→
- ✅ Focus cycling left ↔ editor ↔ right

## 3. Command palette

- ✅ Ctrl+Shift+P opens palette over a registered command set

## 4. LSP

- ⬜ a. Code completion
- ⬜ b. Go-to-definition
- ⬜ c. Build/run project
- ⬜ d. Symbols outline (Ctrl+O / Ctrl+Shift+O)
- ⬜ e. Hover documentation

## 5. Themes

- ✅ Theme loading and tree-sitter scope mapping

## 6. Settings UI

- ⬜ In-editor settings page (shortcut config, plugin settings, themes)

## 7. Plugins

- ✅ a. Contribute actions and shortcuts (mlua + Contributions manifest)
- ✅ b. Render TUI in side panels
- ✅ c. Open documents in the main editor (`devix.open_path`)
- ⬜ d. Contribute custom editors
- ⬜ e. Advanced rendering API — expose ratatui widgets to plugins
- ⬜ f. Multiplugin support — multiple plugins coexisting in one session
---

## Next up

**LSP (§4)** end-to-end against rust-analyzer: completion, hover, go-to-def, document symbols. Workspace symbols and build/run land after.

Settings UI (§6), scroll bars (§1c), right-click menus (§1d), and the custom-editor plugin API (§7d) are deferred until LSP stabilizes.
