//! Modal-overlay paint helper. The structural Pane tree paints itself
//! via `Editor.root.render(...)` from inside `Application::render`; this
//! module covers the modal layer that needs `commands` + `keymap` access
//! to project the palette state into rendering-friendly rows.

use devix_editor::{CommandRegistry, Keymap, PalettePane, format_chord};
use devix_panes::{Pane, PaletteRow, RenderCtx, Theme, palette_area, render_palette};
use ratatui::Frame;
use ratatui::layout::Rect;

pub(crate) fn paint_modal(
    modal: &dyn Pane,
    area: Rect,
    frame: &mut Frame<'_>,
    theme: &Theme,
    commands: &CommandRegistry,
    keymap: &Keymap,
) {
    let any = modal.as_any();
    if let Some(p) = any.and_then(|a| a.downcast_ref::<PalettePane>()) {
        paint_palette(p, area, frame, theme, commands, keymap);
    } else {
        let mut overlay_ctx = RenderCtx { frame };
        modal.render(area, &mut overlay_ctx);
    }
}

fn paint_palette(
    p: &PalettePane,
    editor_area: Rect,
    frame: &mut Frame<'_>,
    theme: &Theme,
    commands: &CommandRegistry,
    keymap: &Keymap,
) {
    let state = &p.state;
    let selected = state.selected();

    let mut chords: Vec<String> = Vec::with_capacity(state.matches().len());
    let mut row_data: Vec<(String, &'static str, usize)> =
        Vec::with_capacity(state.matches().len());
    for i in 0..state.matches().len() {
        let Some(id) = state.matched_command_id(i) else {
            continue;
        };
        let Some(cmd) = commands.get(id) else {
            continue;
        };
        let chord_str = keymap
            .chord_for(id)
            .map(format_chord)
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
        theme,
        frame,
    );
}
