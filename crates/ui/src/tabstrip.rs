//! Tab strip widget — a thin presentation layer on top of the collection
//! primitives in `devix_collection`.
//!
//! Pipeline:
//!
//! 1. Pick per-tab cell widths (natural / uniform shrink down to
//!    `MIN_TAB_WIDTH` / overflow). This is the only logic specific to the tab
//!    strip; everything below is generic.
//! 2. Build a `LinearLayout` from those widths with a 1-cell separator
//!    spacing. Decorations come for free.
//! 3. Sticky scroll the `CollectionState` so the active tab stays visible.
//! 4. Render visible items + decorations through `CollectionPass`. Cells with
//!    `clip_left > 0` use `Paragraph::scroll` to show the right portion of a
//!    partially-visible tab.

use devix_collection::{
    Axis, CollectionLayout, CollectionPass, CollectionState, LinearLayout,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub struct TabInfo {
    pub label: String,
    pub dirty: bool,
}

/// One clickable hit-test region produced by [`render_tabstrip`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabHit {
    pub idx: usize,
    pub rect: Rect,
}

/// Result of a tab-strip render. Owners cache `content_width` to drive
/// scroll-clamp math from input handlers.
#[derive(Default, Clone, Debug)]
pub struct TabStripRender {
    pub hits: Vec<TabHit>,
    pub content_width: u32,
}

/// Smallest cell-width a tab may shrink to before the strip starts to overflow
/// (and scroll). Eight cells fit `" abc… "` — three letters of context plus
/// the ellipsis.
pub const MIN_TAB_WIDTH: u32 = 8;
const SEP: &str = "│";

pub fn render_tabstrip(
    tabs: &[TabInfo],
    active: usize,
    state: &mut CollectionState,
    recenter_active: &mut bool,
    area: Rect,
    frame: &mut Frame<'_>,
) -> TabStripRender {
    if tabs.is_empty() || area.width == 0 || area.height == 0 {
        // No content → reset scroll so a future re-fill doesn't inherit stale state.
        state.scroll_x = 0;
        state.scroll_y = 0;
        *recenter_active = false;
        return TabStripRender::default();
    }
    let active = active.min(tabs.len() - 1);
    let widths = pick_widths(tabs, area.width);
    let layout = LinearLayout {
        axis: Axis::Horizontal,
        sizes: widths,
        cross: 1,
        spacing: 1,
    };
    let content = layout.content_size();
    let viewport = (area.width as u32, area.height as u32);

    // Always re-clamp so resize / tab-close can shrink an out-of-bounds scroll.
    state.set_scroll(state.scroll_x, state.scroll_y, content, viewport);
    // Scroll-into-view is one-shot: only the operation that just changed the
    // active tab can ask for it. Manual wheel scroll past the active tab must
    // not snap back.
    if *recenter_active {
        state.ensure_visible(layout.rect_for(active), content, viewport);
        *recenter_active = false;
    }

    let mut hits = Vec::new();
    let pass = CollectionPass::new(&layout, state, area);

    for (idx, geom) in pass.visible_items() {
        let style = if idx == active {
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        // Render the slice of the tab's text that lands in `geom.screen`,
        // sliced by char count up-front. Avoids leaning on ratatui's Paragraph
        // horizontal-scroll behavior, which has subtle interactions with
        // styled spans (background colors, padding) at the partial-cell edges.
        let text = render_label(&tabs[idx], geom.virt.w);
        let visible = slice_chars(&text, geom.clip_left as usize, geom.screen.width as usize);
        // .style() on Paragraph paints the whole rect (including any cells the
        // sliced text doesn't cover) with the active highlight, so a partial
        // active tab still has a contiguous background.
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(visible, style))).style(style),
            geom.screen,
        );
        hits.push(TabHit { idx, rect: geom.screen });
    }

    for (_id, geom) in pass.visible_decorations() {
        // Separators are 1 cell wide → never partially clipped (project_to_screen
        // returns None when the visible width would be zero), so just paint the
        // glyph as-is.
        let _ = geom.clip_left;
        frame.render_widget(
            Paragraph::new(Line::from(Span::raw(SEP))),
            geom.screen,
        );
    }

    TabStripRender { hits, content_width: content.0 }
}

/// Decide the cell width for each tab in the strip:
///
/// * If every natural-width tab fits with separators, return naturals.
/// * Otherwise distribute the strip evenly across all tabs (down to
///   `MIN_TAB_WIDTH`), sprinkling remainder cells onto leading tabs.
/// * Otherwise (still doesn't fit at minimum) every tab gets `MIN_TAB_WIDTH`
///   and the strip overflows — the caller's `CollectionState` handles scroll.
fn pick_widths(tabs: &[TabInfo], area_width: u16) -> Vec<u32> {
    let n = tabs.len();
    let natural: Vec<u32> = tabs.iter().map(label_width).collect();
    let seps = (n as u32).saturating_sub(1);
    let natural_total: u32 = natural.iter().sum::<u32>() + seps;

    if natural_total <= area_width as u32 { return natural; }

    let tabs_room = (area_width as u32).saturating_sub(seps);
    let per_tab = tabs_room / n as u32;
    let extras = tabs_room % n as u32;
    if per_tab >= MIN_TAB_WIDTH {
        return (0..n)
            .map(|i| per_tab + if (i as u32) < extras { 1 } else { 0 })
            .collect();
    }
    vec![MIN_TAB_WIDTH; n]
}

/// Slice `s` to chars `[skip, skip + take)`. Returns up to `take` chars.
fn slice_chars(s: &str, skip: usize, take: usize) -> String {
    s.chars().skip(skip).take(take).collect()
}

fn label_width(t: &TabInfo) -> u32 {
    (t.label.chars().count() + 2 + if t.dirty { 1 } else { 0 }) as u32
}

/// Render a tab to exactly `width` cells (left-padded, ellipsis-truncated).
fn render_label(t: &TabInfo, width: u32) -> String {
    if width == 0 { return String::new(); }
    let dirty_w = if t.dirty { 1u32 } else { 0 };
    if width < 2 + dirty_w {
        return " ".repeat(width as usize);
    }
    let label_room = (width - 2 - dirty_w) as usize;
    let chars: Vec<char> = t.label.chars().collect();
    let label_part: String = if chars.len() <= label_room {
        let mut s: String = chars.iter().collect();
        for _ in 0..(label_room - chars.len()) { s.push(' '); }
        s
    } else if label_room <= 1 {
        "…".to_string()
    } else {
        let take = label_room - 1;
        let mut s: String = chars.iter().take(take).collect();
        s.push('…');
        s
    };
    format!(" {}{} ", label_part, if t.dirty { "*" } else { "" })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(label: &str) -> TabInfo {
        TabInfo { label: label.to_string(), dirty: false }
    }

    #[test]
    fn pick_widths_natural_when_strip_is_wide() {
        // " a.rs " = 6, sep = 1; 6+6+6 + 2 = 20.
        let widths = pick_widths(&[t("a.rs"), t("b.rs"), t("c.rs")], 100);
        assert_eq!(widths, vec![6, 6, 6]);
    }

    #[test]
    fn pick_widths_uniform_shrink_when_natural_overflows() {
        // 7-char labels (width 9) * 3 + 2 seps = 29 > 26 → shrink.
        // tabs_room = 24, per_tab = 8 ≥ MIN.
        let widths = pick_widths(&[t("abcdefg"), t("hijklmn"), t("opqrstu")], 26);
        assert_eq!(widths, vec![8, 8, 8]);
    }

    #[test]
    fn pick_widths_overflow_uses_min_width_each() {
        // Five long tabs in a 20-wide strip → 5 * 8 + 4 = 44 > 20.
        let tabs: Vec<TabInfo> = (0..5).map(|i| t(&format!("longname{i}"))).collect();
        let widths = pick_widths(&tabs, 20);
        assert!(widths.iter().all(|w| *w == MIN_TAB_WIDTH));
    }

    #[test]
    fn render_label_truncates_with_ellipsis() {
        let s = render_label(&t("verylongname.rs"), 8);
        assert_eq!(s.chars().count(), 8);
        assert!(s.contains('…'));
    }

    #[test]
    fn render_label_pads_short_label() {
        let s = render_label(&t("a"), 8);
        assert_eq!(s.chars().count(), 8);
        assert!(s.starts_with(' ') && s.ends_with(' '));
    }
}
