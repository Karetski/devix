//! Frame composition: the editor area fills the entire frame.
//!
//! Two distinct phases per draw cycle:
//!
//! 1. `Surface::layout(area)` — pre-paint state mutation. Owned by the
//!    surface model so the layout pass can be tested without an `App`.
//!    Walks every `Frame` leaf, runs the cursor-anchor pass on its active
//!    `View.scroll`, and clamps any stale scroll offsets.
//! 2. [`paint`] — pure draw + render-cache updates.

use devix_core::{Pane, RenderCtx};
use ratatui::Frame;
use ratatui::layout::Rect;
use devix_ui::{
    SidebarPane as SidebarChrome, TabStripPane, tab_strip_layout,
};
use devix_editor::{EditorPane, SidebarSlotPane, TabbedPane};
use devix_commands::PalettePane;
use devix_core::SidebarSlot;
use devix_surface::{Document, FrameId, LeafRef, Surface, View};
use devix_ui::{PaletteRow, palette_area, render_palette};

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

    // Phase 1 — layout: scroll-into-view + clamp. Mutates `app.surface`
    // before paint sees it. Owned by surface so the contract "paint
    // never mutates state" stays a one-liner.
    app.surface.layout(editor_area);

    let leaves =
        devix_surface::leaves_with_rects(app.surface.root.as_ref(), editor_area);

    // Phase 2 — paint (pure, plus render-cache writes).
    paint(&leaves, app, frame);

    // Modal Panes paint last (z-order is paint order in ratatui).
    if let Some(modal) = app.surface.modal.as_ref() {
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
/// (label/category/chord-string/selected) so `devix_ui::render_palette` can
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
            .map(devix_commands::format_chord)
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

fn paint(leaves: &[(LeafRef, Rect)], app: &mut App, frame: &mut Frame<'_>) {
    populate_cache(leaves, &mut app.surface);
    paint_leaves(leaves, app, frame);
}

fn populate_cache(leaves: &[(LeafRef, Rect)], ws: &mut Surface) {
    ws.render_cache.frame_rects.clear();
    ws.render_cache.sidebar_rects.clear();
    ws.render_cache.tab_strips.clear();
    for (leaf, rect) in leaves {
        match leaf {
            LeafRef::Sidebar(slot) => {
                ws.render_cache.sidebar_rects.insert(*slot, *rect);
            }
            LeafRef::Frame(fid) => {
                let strip_area = Rect { height: 1, ..*rect };
                let body_area = frame_body_rect(*rect);
                let tabs = build_tab_infos(ws, *fid);
                let Some(frame_state) = devix_surface::find_frame(ws.root.as_ref(), *fid)
                else { continue };
                let active = frame_state.active_tab;
                let scroll = frame_state.tab_strip_scroll;
                let (hits_pure, content_width) =
                    tab_strip_layout(&tabs, active, scroll, strip_area);
                let hits = hits_pure
                    .iter()
                    .map(|h| devix_surface::TabHit { idx: h.idx, rect: h.rect })
                    .collect();
                ws.render_cache.tab_strips.insert(
                    *fid,
                    devix_surface::TabStripCache {
                        strip_rect: strip_area,
                        content_width,
                        hits,
                    },
                );
                ws.render_cache.frame_rects.insert(*fid, body_area);
            }
        }
    }
}

fn paint_leaves(leaves: &[(LeafRef, Rect)], app: &App, frame: &mut Frame<'_>) {
    let mut ctx = RenderCtx { frame };
    for (leaf, rect) in leaves {
        match leaf {
            LeafRef::Frame(id) => {
                let pane = build_tabbed_pane(app, *id);
                pane.render(*rect, &mut ctx);
            }
            LeafRef::Sidebar(slot) => {
                let pane = build_sidebar_pane(app, *slot);
                pane.render(*rect, &mut ctx);
            }
        }
    }
}

fn build_tabbed_pane<'a>(app: &'a App, frame: FrameId) -> TabbedPane<'a> {
    let f = devix_surface::find_frame(app.surface.root.as_ref(), frame)
        .expect("active frame must exist in tree");
    let strip = TabStripPane {
        tabs: build_tab_infos(&app.surface, frame),
        active: f.active_tab,
        scroll: f.tab_strip_scroll,
    };
    let view_id = f.active_view().expect("frame must have an active view");
    let view = &app.surface.views[view_id];
    let doc = &app.surface.documents[view.doc];
    let cached_body = app
        .surface
        .render_cache
        .frame_rects
        .get(&frame)
        .copied()
        .unwrap_or(Rect { x: 0, y: 0, width: 0, height: 0 });
    let height_rows = cached_body.height as usize;
    let (s, e) = visible_byte_range(doc, view, height_rows);
    let highlights = doc.highlights(s, e);
    let editor = EditorPane {
        buffer: &doc.buffer,
        selection: &view.selection,
        scroll: view.scroll,
        theme: &app.theme,
        highlights,
        active: app.surface.active_frame() == Some(frame),
    };
    TabbedPane { strip, editor }
}

pub(crate) fn build_sidebar_pane<'a>(app: &'a App, slot: SidebarSlot) -> SidebarSlotPane<'a> {
    let title = match slot {
        SidebarSlot::Left => "left",
        SidebarSlot::Right => "right",
    };
    let focused = devix_surface::pane_at_indices(
        app.surface.root.as_ref(),
        &app.surface.focus,
    )
    .and_then(devix_surface::pane_leaf_id)
    .map(|id| matches!(id, LeafRef::Sidebar(s) if s == slot))
    .unwrap_or(false);
    let content: Option<Box<dyn devix_core::Pane>> = crate::plugin::sidebar_pane(app, slot)
        .map(|p| Box::new(p) as Box<dyn devix_core::Pane>);
    SidebarSlotPane {
        chrome: SidebarChrome { title: title.to_string(), focused },
        content,
    }
}

fn build_tab_infos(ws: &Surface, frame: FrameId) -> Vec<devix_ui::TabInfo> {
    let Some(frame_state) = devix_surface::find_frame(ws.root.as_ref(), frame) else {
        return Vec::new();
    };
    frame_state
        .tabs
        .iter()
        .map(|vid| {
            let v = &ws.views[*vid];
            let d = &ws.documents[v.doc];
            let label = d
                .buffer
                .path()
                .and_then(|p| p.file_name())
                .and_then(|f| f.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "[scratch]".to_string());
            devix_ui::TabInfo { label, dirty: d.buffer.dirty() }
        })
        .collect()
}

fn frame_body_rect(area: Rect) -> Rect {
    Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    }
}

/// Byte range covering all lines currently visible in `view`'s editor body.
fn visible_byte_range(doc: &Document, view: &View, height_rows: usize) -> (usize, usize) {
    let line_count = doc.buffer.line_count();
    let rope = doc.buffer.rope();
    let top = view.scroll_top().min(line_count);
    let bottom = (view.scroll_top() + height_rows).min(line_count);
    let start = rope.line_to_byte(top);
    let end = if bottom >= line_count {
        rope.len_bytes()
    } else {
        rope.line_to_byte(bottom)
    };
    (start, end)
}
