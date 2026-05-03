//! Workspace-coupled views: render functions that paint editor/palette/symbols
//! state from `devix-workspace` using primitives from `devix-ui` and
//! `devix-collection`. The split between this crate and `devix-ui` is
//! deliberate — `devix-ui` is a pure design system with no awareness of
//! workspace state, so its widgets can be reused without dragging buffer /
//! syntax / LSP types behind them.

pub mod editor;
pub mod palette;
pub mod symbols;

pub use editor::{EditorRenderResult, EditorView, render_editor};
pub use palette::{format_chord, palette_area, render_palette};
pub use symbols::{render_symbols, symbols_area};
