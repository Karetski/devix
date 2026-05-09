//! Lua-table → `View` deserializer + a minimal painter for plugin
//! panes (T-111).
//!
//! Plugins push View IR via `pane:set_view(view_table)`. The table
//! shape mirrors the JSON wire form: `{ kind = "...", ... }`. A
//! sub-set of the View enum is supported in v0 — enough for plugin
//! sidebars to paint structured content without reaching for the
//! line-fallback API:
//!
//! - `{ kind = "empty" }` → `View::Empty`
//! - `{ kind = "text", spans = { { text, style? }, … } }` →
//!   `View::Text`
//! - `{ kind = "stack", axis = "vertical"|"horizontal", weights = {…},
//!    children = {…} }` → `View::Stack`
//!
//! Other variants (`Buffer`, `TabStrip`, `Sidebar`, `Popup`, `Modal`,
//! `Split`, `List`) are rejected at deserialize time — they require
//! editor-side context plugins shouldn't fabricate (`/buf/<id>` paths,
//! tab handles, modal lifecycle). The full View IR walker
//! ([`devix-tui::view_paint::paint_view`]) takes over in T-95 when
//! `paint_view` becomes the only paint path.
//!
//! `paint_minimal` is the matching painter: it walks the supported
//! View variants and stamps cells directly through `Frame::buffer_mut`.
//! Keeping a minimal painter in `devix-core` avoids the
//! `devix-core ↔ devix-tui` cycle that would arise from reaching
//! into `view_paint::paint_view` from the plugin runtime.

use anyhow::anyhow;
use devix_protocol::path::Path;
use devix_protocol::view::{
    Axis, Color, NamedColor, Style, TextSpan, View, ViewNodeId, WrapMode,
};
use mlua::{Table, Value};
use ratatui::Frame;
use ratatui::layout::Rect;

/// Deserialize a Lua table into a [`View`].
///
/// Synthetic node IDs are auto-generated under `/synthetic/plugin/<n>`
/// so plugin-side View nodes don't collide with editor-emitted IDs.
pub(crate) fn view_from_lua_table(t: &Table) -> mlua::Result<View> {
    view_from_lua_inner(t, &mut SyntheticIdCounter::default())
}

#[derive(Default)]
struct SyntheticIdCounter(u32);

impl SyntheticIdCounter {
    fn next_id(&mut self, kind: &str) -> ViewNodeId {
        self.0 += 1;
        let path = Path::parse(&format!("/synthetic/plugin/{}-{}", kind, self.0))
            .expect("synthetic plugin id is canonical");
        ViewNodeId(path)
    }
}

fn view_from_lua_inner(
    t: &Table,
    ids: &mut SyntheticIdCounter,
) -> mlua::Result<View> {
    let kind: String = t.get("kind").unwrap_or_else(|_| "empty".to_string());
    match kind.as_str() {
        "empty" => Ok(View::Empty),
        "text" => {
            let spans_raw: Value = t.get("spans").unwrap_or(Value::Nil);
            let spans = match spans_raw {
                Value::Nil => Vec::new(),
                Value::Table(spans_t) => {
                    let mut out = Vec::new();
                    for v in spans_t.sequence_values::<Value>() {
                        let v = v?;
                        out.push(span_from_lua(v)?);
                    }
                    out
                }
                other => {
                    return Err(mlua::Error::external(anyhow!(
                        "view text spans must be a sequence, got {:?}",
                        other,
                    )));
                }
            };
            let wrap = match t.get::<Option<String>>("wrap")?.as_deref() {
                None | Some("nowrap") => WrapMode::NoWrap,
                Some("wrap") => WrapMode::Wrap,
                Some("truncate") => WrapMode::Truncate,
                Some(other) => {
                    return Err(mlua::Error::external(anyhow!(
                        "unknown wrap mode `{other}` (use `wrap` / `nowrap` / `truncate`)",
                    )));
                }
            };
            Ok(View::Text {
                id: ids.next_id("text"),
                spans,
                wrap,
                transition: None,
            })
        }
        "stack" => {
            let axis = match t.get::<String>("axis")?.as_str() {
                "vertical" => Axis::Vertical,
                "horizontal" => Axis::Horizontal,
                other => {
                    return Err(mlua::Error::external(anyhow!(
                        "unknown stack axis `{other}` (use `vertical` / `horizontal`)",
                    )));
                }
            };
            let weights_t: Table = t.get("weights").unwrap_or_else(|_| {
                t.raw_get("weights")
                    .unwrap_or_else(|_| t.raw_get("children").unwrap())
            });
            let mut weights: Vec<u16> = Vec::new();
            for w in weights_t.sequence_values::<u32>() {
                weights.push(w?.min(u16::MAX as u32) as u16);
            }
            let children_t: Table = t.get("children")?;
            let mut children: Vec<View> = Vec::new();
            for child in children_t.sequence_values::<Table>() {
                children.push(view_from_lua_inner(&child?, ids)?);
            }
            // If the plugin omitted weights, default to 1 per child.
            if weights.is_empty() {
                weights = vec![1; children.len()];
            }
            let spacing: u32 = t.get("spacing").unwrap_or(0);
            Ok(View::Stack {
                id: ids.next_id("stack"),
                axis,
                weights,
                children,
                spacing,
                transition: None,
            })
        }
        other => Err(mlua::Error::external(anyhow!(
            "view kind `{other}` not supported in plugin View IR (T-111 v0); \
             supported: empty, text, stack",
        ))),
    }
}

fn span_from_lua(v: Value) -> mlua::Result<TextSpan> {
    match v {
        Value::String(s) => Ok(TextSpan {
            text: s.to_str()?.to_string(),
            style: Style::default(),
        }),
        Value::Table(t) => {
            let text: String = t.get("text").unwrap_or_default();
            let style_t: Option<Table> = t.get("style").ok();
            let style = match style_t {
                Some(st) => style_from_lua(&st)?,
                None => Style::default(),
            };
            Ok(TextSpan { text, style })
        }
        other => Err(mlua::Error::external(anyhow!(
            "text span must be a string or {{ text, style? }} table, got {:?}",
            other,
        ))),
    }
}

fn style_from_lua(t: &Table) -> mlua::Result<Style> {
    let mut out = Style::default();
    if let Some(fg) = t.get::<Option<String>>("fg")? {
        out.fg = Some(parse_color(&fg)?);
    }
    if let Some(bg) = t.get::<Option<String>>("bg")? {
        out.bg = Some(parse_color(&bg)?);
    }
    out.bold = t.get("bold").unwrap_or(false);
    out.italic = t.get("italic").unwrap_or(false);
    out.underline = t.get("underline").unwrap_or(false);
    out.dim = t.get("dim").unwrap_or(false);
    out.reverse = t.get("reverse").unwrap_or(false);
    Ok(out)
}

fn parse_color(s: &str) -> mlua::Result<Color> {
    Color::parse(s).map_err(|e| mlua::Error::external(anyhow!(e)))
}

/// Minimal painter for plugin-pushed View IR. Supports `Empty`,
/// `Text`, and `Stack`; other variants paint nothing in v0. The full
/// `paint_view` walker in `devix-tui` becomes the sole renderer at
/// T-95; until then this mirror lets plugins exercise the View IR
/// surface without forcing a `devix-core ↔ devix-tui` cycle.
pub(crate) fn paint_minimal(view: &View, area: Rect, frame: &mut Frame<'_>) {
    paint_inner(view, area, frame);
}

fn paint_inner(view: &View, area: Rect, frame: &mut Frame<'_>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    match view {
        View::Empty => {}
        View::Text { spans, .. } => paint_text(spans, area, frame),
        View::Stack { axis, weights, children, .. } => {
            paint_stack(*axis, weights, children, area, frame);
        }
        // Other variants ship cells through the full paint_view in
        // devix-tui at T-95. Plugin-side IR rejects them at deserialize
        // time, so this arm is unreachable for plugin panes today.
        _ => {}
    }
}

fn paint_text(spans: &[TextSpan], area: Rect, frame: &mut Frame<'_>) {
    let buf = frame.buffer_mut();
    // Concatenate spans into a single line; v0 doesn't wrap.
    let mut x = area.x;
    let max_x = area.x.saturating_add(area.width);
    for span in spans {
        if x >= max_x {
            break;
        }
        let style = view_style_to_ratatui(&span.style);
        let remaining = (max_x - x) as usize;
        let consumed = buf.set_stringn(x, area.y, &span.text, remaining, style);
        // `set_stringn` returns (next_x, _next_y); we only care about
        // the column advance.
        x = consumed.0;
    }
}

fn paint_stack(
    axis: Axis,
    weights: &[u16],
    children: &[View],
    area: Rect,
    frame: &mut Frame<'_>,
) {
    if children.is_empty() {
        return;
    }
    let total: u32 = weights
        .iter()
        .copied()
        .map(|w| w.max(1) as u32)
        .sum::<u32>()
        .max(1);
    let mut cursor = match axis {
        Axis::Vertical => area.y,
        Axis::Horizontal => area.x,
    };
    let extent = match axis {
        Axis::Vertical => area.height,
        Axis::Horizontal => area.width,
    } as u32;
    let mut consumed: u32 = 0;
    for (i, child) in children.iter().enumerate() {
        let w = weights.get(i).copied().unwrap_or(1).max(1) as u32;
        let span = if i + 1 == children.len() {
            extent.saturating_sub(consumed)
        } else {
            extent.saturating_mul(w) / total
        };
        let span_u16 = span.min(u16::MAX as u32) as u16;
        let child_area = match axis {
            Axis::Vertical => Rect {
                x: area.x,
                y: cursor,
                width: area.width,
                height: span_u16,
            },
            Axis::Horizontal => Rect {
                x: cursor,
                y: area.y,
                width: span_u16,
                height: area.height,
            },
        };
        paint_inner(child, child_area, frame);
        cursor = match axis {
            Axis::Vertical => cursor.saturating_add(span_u16),
            Axis::Horizontal => cursor.saturating_add(span_u16),
        };
        consumed = consumed.saturating_add(span);
    }
}

fn view_style_to_ratatui(style: &Style) -> ratatui::style::Style {
    use ratatui::style::{Modifier, Style as RStyle};
    let mut s = RStyle::default();
    if let Some(c) = style.fg {
        s = s.fg(view_color_to_ratatui(c));
    }
    if let Some(c) = style.bg {
        s = s.bg(view_color_to_ratatui(c));
    }
    let mut mods = Modifier::empty();
    if style.bold {
        mods |= Modifier::BOLD;
    }
    if style.italic {
        mods |= Modifier::ITALIC;
    }
    if style.underline {
        mods |= Modifier::UNDERLINED;
    }
    if style.dim {
        mods |= Modifier::DIM;
    }
    if style.reverse {
        mods |= Modifier::REVERSED;
    }
    s.add_modifier(mods)
}

fn view_color_to_ratatui(c: Color) -> ratatui::style::Color {
    use ratatui::style::Color as RColor;
    match c {
        Color::Default => RColor::Reset,
        Color::Rgb(r, g, b) => RColor::Rgb(r, g, b),
        Color::Indexed(n) => RColor::Indexed(n),
        Color::Named(n) => match n {
            NamedColor::Black => RColor::Black,
            NamedColor::Red => RColor::Red,
            NamedColor::Green => RColor::Green,
            NamedColor::Yellow => RColor::Yellow,
            NamedColor::Blue => RColor::Blue,
            NamedColor::Magenta => RColor::Magenta,
            NamedColor::Cyan => RColor::Cyan,
            NamedColor::White => RColor::White,
            NamedColor::DarkGray => RColor::DarkGray,
            NamedColor::LightRed => RColor::LightRed,
            NamedColor::LightGreen => RColor::LightGreen,
            NamedColor::LightYellow => RColor::LightYellow,
            NamedColor::LightBlue => RColor::LightBlue,
            NamedColor::LightMagenta => RColor::LightMagenta,
            NamedColor::LightCyan => RColor::LightCyan,
        },
    }
}
