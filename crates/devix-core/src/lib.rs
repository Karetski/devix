//! `devix-core` — the engine.
//!
//! Owns every model state, every command handler, the plugin host, the
//! pulse bus implementation (lands in T-31), the manifest loader (T-33),
//! the theme registry. The eventual no-`ratatui` / no-`crossterm` shape
//! described in `docs/specs/crates.md` is reached at the end of Stage 9:
//! T-11 absorbed the chrome widgets and ratatui-bound layout primitives
//! to break a transient `devix-core ↔ devix-tui` cycle; T-92 / T-95
//! finish the move to `devix-tui` once the layout-tree dissolution
//! (T-91) finishes (per the foundations-review amendment log
//! 2026-05-06).
//!
//! Stage 1 (T-10..T-13) absorbed the dissolved `devix-editor`,
//! `devix-plugin`, and `devix-panes` crates into this crate. The
//! pre-Stage-1 public surface is preserved at the crate root so the
//! binary keeps importing the same names; later stages will rename /
//! split as the spec implementations land.

/// Built-in manifest, embedded at compile time. Loaded by
/// `manifest_loader` to register every built-in command, keymap
/// binding, and theme — replacing the hard-coded tables in
/// `editor::commands::builtins` / `editor::commands::keymap` /
/// `theme.rs` (T-74). Single source of truth for what palette,
/// settings UI, and `:help` enumerate.
pub const BUILTIN_MANIFEST: &str = include_str!("../manifests/builtin.json");

pub mod action;
pub mod bus;
pub mod clipboard;
pub mod composites;
pub mod editor;
pub mod event;
pub mod geom;
pub mod layout_geom;
pub mod manifest_loader;
pub mod pane;
pub mod plugin;
pub mod settings_store;
pub mod supervise;
pub mod theme;
pub mod theme_store;
pub mod walk;
pub mod widgets;

pub use bus::PulseBus;

// Trait surface (pre-Stage-1: was `devix-panes`'s public surface).
pub use action::Action;
pub use clipboard::{Clipboard, NoClipboard};
pub use composites::{SidebarSlotPane, TabbedPane};
pub use event::Event;
pub use geom::{Anchor, AnchorEdge, Rect};
pub use layout_geom::{Axis, Direction, SidebarSlot, split_rects};
pub use pane::{HandleCtx, Outcome, Pane, RenderCtx};
pub use theme::Theme;
pub use walk::{focusable_at, focusable_leaves, pane_at, pane_at_path};
pub use widgets::{
    CompletionLine, MIN_TAB_WIDTH, PaletteRow, Popup, PopupAnchor, PopupContent, SidebarInfo,
    SidebarPane, TabInfo, TabStripPane, TabStripRender, layout_tabstrip, palette_area,
    render_palette, render_popup, render_sidebar, render_tabstrip, tab_strip_layout,
};
// `format_chord` (editor::commands::modal) is the canonical export; the
// chrome-side `widgets::palette::format_chord` is module-qualified to
// avoid the name collision until widgets move to `devix-tui` (T-12).
pub use editor::commands::modal::format_chord;
// `TabHit` from `widgets::tabstrip` (the chrome hit-test type) is the
// canonical crate-root export; the editor's structurally-identical
// `editor::editor::TabHit` is exposed as `EditorTabHit`.
pub use widgets::TabHit;

// Editor surface (pre-Stage-1: was `devix-editor`'s public surface).
pub use editor::buffer::{BufferRender, EditorPane, EditorRenderResult, render_buffer};
pub use editor::commands::{
    Chord, Command, CommandId, CommandRegistry, Context, EditorCommand, Keymap, ModalOutcome,
    PalettePane, PaletteState, Viewport, build_registry, chord_from_key, cmd, default_keymap,
    register_builtins,
};
pub use editor::cursor::{Cursor, CursorId, ScrollMode};
pub use editor::document::{DocId, Document};
pub use editor::editor::{
    Editor, LeafRef, RenderCache, TabHit as EditorTabHit, TabStripCache, TabStripHit,
};
pub use editor::frame::FrameId;
pub use editor::tree::{
    LayoutCtx, LayoutFrame, LayoutSidebar, LayoutSplit, frame_pane, sidebar_pane, split_pane,
};

// Plugin surface (pre-Stage-1: was `devix-plugin`'s public surface).
pub use plugin::{
    CommandSpec as PluginCommandSpec, Contributions as PluginContributions,
    LuaAction, LuaPane, LuaPaneHandle, MsgSink, PaneSpec as PluginPaneSpec, PluginCommandAction,
    PluginHost, PluginInput, PluginMsg, PluginPane, PluginRuntime, default_plugin_path,
    make_command_action, parse_chord,
};
