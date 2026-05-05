//! Frame composition: hand the structural Pane tree to ratatui.
//!
//! Two phases per draw cycle:
//!
//! 1. `Editor::layout(area)` — pre-paint state mutation. Walks the
//!    structural tree, runs the caret-anchor pass on each `Frame`'s
//!    active `Cursor.scroll`, and populates the render-cache (frame
//!    body rects, tab-strip hit regions, sidebar rects). All
//!    mutation lives here so paint stays pure.
//! 2. [`render`] — pure draw. The structural tree's own `render` impls
//!    paint themselves: `Editor.root.render(area, ctx)` walks the
//!    whole tree (no parallel render-tree pass, no per-leaf match arm
//!    in the binary). Modal Panes paint last for z-ordering.

use devix_panes::{Pane, RenderCtx, SidebarSlot};
use devix_editor::RenderServices;
use ratatui::Frame;
use ratatui::layout::Rect;
use devix_editor::PalettePane;
use devix_panes::{PaletteRow, palette_area, render_palette};

use crate::app::App;

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Sets `DEVIX_PLUGIN` for one test; restores the previous value on
    /// drop so concurrent tests don't poison each other.
    struct EnvGuard {
        key: &'static str,
        prev: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl EnvGuard {
        fn set(key: &'static str, val: &str) -> Self {
            static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let _lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let prev = std::env::var_os(key);
            unsafe { std::env::set_var(key, val); }
            Self { key, prev, _lock }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match self.prev.take() {
                    Some(v) => std::env::set_var(self.key, v),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    #[test]
    fn sidebar_renders_plugin_supplied_lines() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let example = manifest
            .parent()
            .unwrap()
            .join("plugin/examples/file_tree.lua");
        let _g = EnvGuard::set("DEVIX_PLUGIN", &example.to_string_lossy());
        let mut app = App::new(None, None).expect("App constructs with plugin");
        assert!(app.plugins.is_some(), "plugin should have loaded");

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::render::render(f, &mut app)).unwrap();

        let buf = terminal.backend().buffer();
        let mut all = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                all.push_str(buf[(x, y)].symbol());
            }
            all.push('\n');
        }
        assert!(
            all.contains('▸') || all.contains("Cargo"),
            "expected file-tree content (▸ marker or `Cargo` entry) somewhere in:\n{all}",
        );
    }
}

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    let editor_area = frame.area();

    // Phase 1 — pre-paint mutation. Owned by `Editor::layout`:
    // scroll-into-view, tab-strip layout, render-cache writes.
    app.editor.layout(editor_area);

    // The structural Pane tree paints itself. Bundle the editor-side
    // borrows it needs into a `RenderServices` and stash them in a
    // scoped thread-local for the duration of `root.render(...)`;
    // structural Panes (`LayoutFrame`, `LayoutSidebar`) read it via
    // `RenderServices::with`. Plugin sidebar resolution is a closure
    // so `editor` doesn't depend on the binary's plugin world.
    let focused_leaf =
        devix_editor::pane_at_indices(app.editor.root.as_ref(), &app.editor.focus)
            .and_then(devix_editor::pane_leaf_id);
    let plugin_resolver = |slot: SidebarSlot| -> Option<Box<dyn Pane>> {
        crate::plugin::sidebar_pane(app, slot)
            .map(|p| Box::new(p) as Box<dyn Pane>)
    };
    {
        let services = RenderServices {
            documents: &app.editor.documents,
            cursors: &app.editor.cursors,
            theme: &app.theme,
            render_cache: &app.editor.render_cache,
            focused_leaf,
            plugin_sidebar: &plugin_resolver,
        };
        let mut ctx = RenderCtx { frame };
        let root = app.editor.root.as_ref();
        services.scope(|| root.render(editor_area, &mut ctx));
    }

    // Modal Panes paint last (z-order is paint order in ratatui).
    if let Some(modal) = app.editor.modal.as_ref() {
        let any = modal.as_any();
        if let Some(p) = any.and_then(|a| a.downcast_ref::<PalettePane>()) {
            paint_palette(p, app, editor_area, frame);
        } else {
            let mut overlay_ctx = RenderCtx { frame };
            modal.render(editor_area, &mut overlay_ctx);
        }
    }
}

/// Project the palette state into the rendering-friendly `PaletteRow` shape
/// (label/category/chord-string/selected) so `devix_panes::render_palette` can
/// stay free of `commands`-side types.
fn paint_palette(p: &PalettePane, app: &App, editor_area: Rect, frame: &mut Frame<'_>) {
    let state = &p.state;
    let selected = state.selected();

    let mut chords: Vec<String> = Vec::with_capacity(state.matches().len());
    let mut row_data: Vec<(String, &'static str, usize)> = Vec::with_capacity(state.matches().len());
    for i in 0..state.matches().len() {
        let Some(id) = state.matched_command_id(i) else { continue };
        let Some(cmd) = app.commands.get(id) else { continue };
        let chord_str = app
            .keymap
            .chord_for(id)
            .map(devix_editor::format_chord)
            .unwrap_or_default();
        chords.push(chord_str);
        row_data.push((cmd.label.to_string(), cmd.category.unwrap_or(""), i));
    }
    let rows: Vec<PaletteRow<'_>> = row_data
        .iter()
        .zip(chords.iter())
        .map(|((label, category, i), chord)| PaletteRow {
            label: label.as_str(),
            category,
            chord: chord.as_str(),
            selected: *i == selected,
        })
        .collect();

    render_palette(
        state.query(),
        &rows,
        selected,
        palette_area(editor_area),
        &app.theme,
        frame,
    );
}

