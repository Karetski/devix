//! View IR — `docs/specs/frontend.md`.
//!
//! T-40 lands the full closed `View` enum and its supporting types.
//! T-41 replaces the placeholder derive-serde for `Color` with the
//! canonical string form (`"default"`, `"#rrggbb"`, `"@<n>"`,
//! `"<named>"`).
//!
//! `View` is the snapshot of renderable visual state core ships to
//! the frontend. Synchronous request/response (not bus-fanned) per
//! spec § *What does not flow over the bus*. Every node carries a
//! stable `ViewNodeId(Path)` so the frontend can diff across
//! renders, animate transitions, and preserve focus.

use serde::{Deserialize, Serialize};

use crate::HighlightSpan;
use crate::path::Path;

/// Stable id for a view node. Resource-bound nodes use the
/// resource's canonical path (`/buf/42`, `/pane/0/sidebar/left`);
/// synthetic nodes use `/synthetic/<kind>/<id>`. The synthetic-id
/// strategy itself (mint-and-cache vs deterministic derivation) is
/// picked at T-90.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ViewNodeId(pub Path);

/// Closed view-IR enum. Frontends interpret these directly; new
/// renderable kinds live as new variants here, never as a parallel
/// type.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum View {
    /// Empty placeholder; renders nothing.
    Empty,

    /// Styled text run. Single line; explicit newlines are not
    /// embedded — multi-line text uses `Stack` of `Text` nodes or
    /// `List`.
    Text {
        id: ViewNodeId,
        spans: Vec<TextSpan>,
        wrap: WrapMode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transition: Option<TransitionHint>,
    },

    /// Vertical or horizontal stack of children with proportional
    /// weights.
    Stack {
        id: ViewNodeId,
        axis: Axis,
        weights: Vec<u16>,
        children: Vec<View>,
        spacing: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transition: Option<TransitionHint>,
    },

    /// Top-to-bottom list of items; one item per logical line.
    /// Frontends virtualize (paint only visible items).
    List {
        id: ViewNodeId,
        items: Vec<View>,
        item_keys: Vec<ViewNodeId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selected: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transition: Option<TransitionHint>,
    },

    /// Document body. Producer materializes the visible window into
    /// `lines` (gutter + style-run spans per visible line) so the
    /// renderer can paint without reaching back to the document
    /// store. T-95 producer-materialization design choice. The
    /// producer also keeps `path` + `scroll_top_line` etc. populated
    /// so frontends that prefer to virtualize themselves can route
    /// through their own buffer provider — `lines` is empty when no
    /// materialization happened (e.g., minimum-viable producer paths
    /// from T-43 / pre-T-95).
    Buffer {
        id: ViewNodeId,
        path: Path,
        scroll_top_line: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cursor: Option<CursorMark>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        selection: Vec<SelectionMark>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        highlights: Vec<HighlightSpan>,
        gutter: GutterMode,
        active: bool,
        /// Materialized visible-window content. Empty when the
        /// producer hasn't pre-rendered (back-compat default).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        lines: Vec<BufferLine>,
        /// Gutter width in cells. `0` when `lines` is empty or when
        /// the gutter mode is `None`.
        #[serde(default, skip_serializing_if = "is_zero_u32")]
        gutter_width: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transition: Option<TransitionHint>,
    },

    /// Tab strip. Frontend lays out, scrolls, hit-tests.
    TabStrip {
        id: ViewNodeId,
        tabs: Vec<TabItem>,
        active: u32,
    },

    /// Sidebar with title + content. Content is itself a `View`.
    Sidebar {
        id: ViewNodeId,
        slot: SidebarSlot,
        title: String,
        focused: bool,
        content: Box<View>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transition: Option<TransitionHint>,
    },

    /// Layout split. Children rendered side-by-side along the axis.
    Split {
        id: ViewNodeId,
        axis: Axis,
        weights: Vec<u16>,
        children: Vec<View>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transition: Option<TransitionHint>,
    },

    /// Floating overlay anchored to a cell (popup, hover,
    /// completion).
    Popup {
        id: ViewNodeId,
        anchor: Anchor,
        content: Box<View>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_size: Option<(u16, u16)>,
        chrome: PopupChrome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transition: Option<TransitionHint>,
    },

    /// Centered modal (palette, picker). Z-top in its frame.
    Modal {
        id: ViewNodeId,
        title: String,
        content: Box<View>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transition: Option<TransitionHint>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TextSpan {
    pub text: String,
    pub style: Style,
}

/// One materialized visible line of a `View::Buffer`. The gutter +
/// content come pre-rendered so the consumer is a thin walker.
/// T-95.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BufferLine {
    /// 0-based logical line number within the buffer (post-scroll).
    pub line: u32,
    /// Pre-formatted gutter text (e.g. `" 42 "`). Empty when
    /// `GutterMode::None`.
    pub gutter: String,
    /// Style runs covering the visible portion of the line.
    /// Producers split a single line into one span per
    /// theme-resolved scope group; the renderer concatenates spans
    /// in order.
    pub spans: Vec<TextSpan>,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WrapMode {
    Wrap,
    NoWrap,
    Truncate,
}

/// Layout axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// Sidebar slot identifier.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SidebarSlot {
    Left,
    Right,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TabItem {
    pub id: ViewNodeId,
    pub label: String,
    pub dirty: bool,
    pub doc: Path,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Anchor {
    pub col: u16,
    pub row: u16,
    pub edge: AnchorEdge,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnchorEdge {
    Above,
    Below,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PopupChrome {
    Bordered,
    Borderless,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GutterMode {
    LineNumbers,
    None,
}

/// Position of the primary caret. `View::Buffer` carries one of
/// these via `cursor: Option<CursorMark>`; secondary multicursor
/// carets are zero-extent `SelectionMark`s in `selection`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CursorMark {
    pub line: u32,
    pub col: u32,
}

/// One selection range. Point cursors (multicursor secondaries)
/// appear as zero-extent marks where `start == end`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelectionMark {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// Animation hint. Gated on `Capability::Animations`; when the
/// frontend doesn't advertise the bit, core sets every `transition`
/// to `None`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransitionHint {
    pub kind: TransitionKind,
    pub duration_ms: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    Enter,
    Exit,
    Move,
}

/// Resolved style. T-41 replaces `Color` with the canonical
/// string-form serde; the field shape is stable.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct Style {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fg: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<Color>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dim: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub reverse: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Color value. Custom serde to/from the canonical string form per
/// `docs/specs/frontend.md` § *Color serialization*:
///
/// | String form                          | Variant            |
/// |--------------------------------------|--------------------|
/// | `"default"`                          | `Color::Default`   |
/// | `"#rrggbb"` (hex, lower or upper)    | `Color::Rgb(...)`  |
/// | `"@<n>"` where `0 ≤ n ≤ 255`         | `Color::Indexed(n)`|
/// | snake_case `NamedColor` (e.g. `"red"`, `"dark_gray"`) | `Color::Named(...)` |
///
/// Anything else is a deserialize error — there is no fallback to a
/// default value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Default,
    Rgb(u8, u8, u8),
    Indexed(u8),
    Named(NamedColor),
}

impl std::fmt::Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Color::Default => f.write_str("default"),
            Color::Rgb(r, g, b) => write!(f, "#{:02x}{:02x}{:02x}", r, g, b),
            Color::Indexed(n) => write!(f, "@{}", n),
            Color::Named(n) => f.write_str(named_color_str(*n)),
        }
    }
}

impl serde::Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Color;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a canonical color string (`default`, `#rrggbb`, `@<n>`, or a named color)")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Color, E> {
                Color::parse(v).map_err(serde::de::Error::custom)
            }
        }
        d.deserialize_str(V)
    }
}

impl schemars::JsonSchema for Color {
    fn schema_name() -> String {
        "Color".to_string()
    }
    fn json_schema(_: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        use schemars::schema::{InstanceType, Metadata, SchemaObject};
        SchemaObject {
            instance_type: Some(InstanceType::String.into()),
            metadata: Some(Box::new(Metadata {
                description: Some(
                    "Color: 'default', '#rrggbb', '@<n>' (0-255), or a named color (e.g. 'red')."
                        .into(),
                ),
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
    }
}

impl Color {
    /// Parse a color from its canonical string form. Returns the
    /// raw error string on failure.
    pub fn parse(s: &str) -> Result<Self, String> {
        if s == "default" {
            return Ok(Color::Default);
        }
        if let Some(rest) = s.strip_prefix('#') {
            if rest.len() != 6 {
                return Err(format!("hex color `{}` must be exactly 6 hex digits", s));
            }
            let r = u8::from_str_radix(&rest[0..2], 16)
                .map_err(|_| format!("invalid hex byte in `{}`", s))?;
            let g = u8::from_str_radix(&rest[2..4], 16)
                .map_err(|_| format!("invalid hex byte in `{}`", s))?;
            let b = u8::from_str_radix(&rest[4..6], 16)
                .map_err(|_| format!("invalid hex byte in `{}`", s))?;
            return Ok(Color::Rgb(r, g, b));
        }
        if let Some(rest) = s.strip_prefix('@') {
            let n: u32 = rest
                .parse()
                .map_err(|_| format!("indexed color `{}` must be a non-negative integer", s))?;
            if n > 255 {
                return Err(format!("indexed color `{}` exceeds 255", s));
            }
            return Ok(Color::Indexed(n as u8));
        }
        named_color_from_str(s)
            .map(Color::Named)
            .ok_or_else(|| format!("unknown color `{}`", s))
    }
}

fn named_color_str(n: NamedColor) -> &'static str {
    match n {
        NamedColor::Black => "black",
        NamedColor::Red => "red",
        NamedColor::Green => "green",
        NamedColor::Yellow => "yellow",
        NamedColor::Blue => "blue",
        NamedColor::Magenta => "magenta",
        NamedColor::Cyan => "cyan",
        NamedColor::White => "white",
        NamedColor::DarkGray => "dark_gray",
        NamedColor::LightRed => "light_red",
        NamedColor::LightGreen => "light_green",
        NamedColor::LightYellow => "light_yellow",
        NamedColor::LightBlue => "light_blue",
        NamedColor::LightMagenta => "light_magenta",
        NamedColor::LightCyan => "light_cyan",
    }
}

fn named_color_from_str(s: &str) -> Option<NamedColor> {
    Some(match s {
        "black" => NamedColor::Black,
        "red" => NamedColor::Red,
        "green" => NamedColor::Green,
        "yellow" => NamedColor::Yellow,
        "blue" => NamedColor::Blue,
        "magenta" => NamedColor::Magenta,
        "cyan" => NamedColor::Cyan,
        "white" => NamedColor::White,
        "dark_gray" => NamedColor::DarkGray,
        "light_red" => NamedColor::LightRed,
        "light_green" => NamedColor::LightGreen,
        "light_yellow" => NamedColor::LightYellow,
        "light_blue" => NamedColor::LightBlue,
        "light_magenta" => NamedColor::LightMagenta,
        "light_cyan" => NamedColor::LightCyan,
        _ => return None,
    })
}

/// Named ANSI/VT100-equivalent colors. Serde shape is the
/// snake_case string segment of `Color`'s wire form (e.g.
/// `"dark_gray"`); the derive on this enum is unused at the wire
/// level — `Color`'s custom serde routes through
/// `named_color_str` / `named_color_from_str`. The derive remains
/// so this enum can serde-round-trip on its own (e.g., as a manifest
/// theme palette key) without forcing every consumer through `Color`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamedColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    DarkGray,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> ViewNodeId {
        ViewNodeId(Path::parse(s).unwrap())
    }

    #[test]
    fn empty_round_trips() {
        let v = View::Empty;
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#"{"kind":"empty"}"#);
        let _: View = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn buffer_round_trips_with_path_id() {
        let v = View::Buffer {
            id: id("/buf/42"),
            path: Path::parse("/buf/42").unwrap(),
            scroll_top_line: 0,
            cursor: Some(CursorMark { line: 0, col: 5 }),
            selection: Vec::new(),
            highlights: Vec::new(),
            gutter: GutterMode::LineNumbers,
            active: true,
            lines: Vec::new(),
            gutter_width: 0,
            transition: None,
        };
        let json = serde_json::to_string(&v).unwrap();
        let back: View = serde_json::from_str(&json).unwrap();
        match back {
            View::Buffer {
                id, path, cursor, ..
            } => {
                assert_eq!(id.0.as_str(), "/buf/42");
                assert_eq!(path.as_str(), "/buf/42");
                assert_eq!(cursor, Some(CursorMark { line: 0, col: 5 }));
            }
            _ => panic!("variant mismatch"),
        }
    }

    #[test]
    fn nested_split_round_trips() {
        let v = View::Split {
            id: id("/pane"),
            axis: Axis::Horizontal,
            weights: vec![1, 1],
            children: vec![
                View::Buffer {
                    id: id("/buf/1"),
                    path: Path::parse("/buf/1").unwrap(),
                    scroll_top_line: 0,
                    cursor: None,
                    selection: Vec::new(),
                    highlights: Vec::new(),
                    gutter: GutterMode::None,
                    active: false,
                    lines: Vec::new(),
                    gutter_width: 0,
                    transition: None,
                },
                View::Empty,
            ],
            transition: None,
        };
        let json = serde_json::to_string(&v).unwrap();
        let _back: View = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn tab_strip_carries_typed_tab_items() {
        let v = View::TabStrip {
            id: id("/pane/0/tabstrip"),
            tabs: vec![
                TabItem {
                    id: id("/pane/0/tab/0"),
                    label: "main.rs".into(),
                    dirty: true,
                    doc: Path::parse("/buf/42").unwrap(),
                },
                TabItem {
                    id: id("/pane/0/tab/1"),
                    label: "lib.rs".into(),
                    dirty: false,
                    doc: Path::parse("/buf/43").unwrap(),
                },
            ],
            active: 0,
        };
        let _: View = serde_json::from_str(&serde_json::to_string(&v).unwrap()).unwrap();
    }

    #[test]
    fn view_node_id_round_trips_through_path_string() {
        let id = ViewNodeId(Path::parse("/synthetic/stack/42").unwrap());
        let json = serde_json::to_string(&id).unwrap();
        // Inner Path serializes as the canonical string; the wrapper
        // serializes as a struct holding it.
        assert!(json.contains("/synthetic/stack/42"));
        let back: ViewNodeId = serde_json::from_str(&json).unwrap();
        assert_eq!(back.0.as_str(), "/synthetic/stack/42");
    }

    #[test]
    fn style_default_omits_unset_fields() {
        let s = Style::default();
        let json = serde_json::to_string(&s).unwrap();
        // No fg, bg, or boolean modifiers set — empty object.
        assert_eq!(json, "{}");
        let back: Style = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn anchor_and_popup_round_trip() {
        let v = View::Popup {
            id: id("/synthetic/popup/1"),
            anchor: Anchor {
                col: 10,
                row: 5,
                edge: AnchorEdge::Below,
            },
            content: Box::new(View::Empty),
            max_size: Some((40, 10)),
            chrome: PopupChrome::Bordered,
            transition: None,
        };
        let _: View = serde_json::from_str(&serde_json::to_string(&v).unwrap()).unwrap();
    }

    #[test]
    fn color_default_round_trips_string_form() {
        let c = Color::Default;
        assert_eq!(serde_json::to_string(&c).unwrap(), "\"default\"");
        let back: Color = serde_json::from_str("\"default\"").unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn color_rgb_round_trips_lower_hex() {
        let c = Color::Rgb(0xaa, 0xbb, 0xcc);
        assert_eq!(serde_json::to_string(&c).unwrap(), "\"#aabbcc\"");
        let back: Color = serde_json::from_str("\"#aabbcc\"").unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn color_rgb_accepts_upper_hex_on_input() {
        let back: Color = serde_json::from_str("\"#AABBCC\"").unwrap();
        assert_eq!(back, Color::Rgb(0xaa, 0xbb, 0xcc));
    }

    #[test]
    fn color_indexed_round_trips() {
        for n in [0u8, 8, 42, 255] {
            let c = Color::Indexed(n);
            let s = serde_json::to_string(&c).unwrap();
            assert_eq!(s, format!("\"@{}\"", n));
            let back: Color = serde_json::from_str(&s).unwrap();
            assert_eq!(back, c);
        }
    }

    #[test]
    fn color_named_round_trips_all_variants() {
        let names = [
            (NamedColor::Black, "black"),
            (NamedColor::Red, "red"),
            (NamedColor::Green, "green"),
            (NamedColor::Yellow, "yellow"),
            (NamedColor::Blue, "blue"),
            (NamedColor::Magenta, "magenta"),
            (NamedColor::Cyan, "cyan"),
            (NamedColor::White, "white"),
            (NamedColor::DarkGray, "dark_gray"),
            (NamedColor::LightRed, "light_red"),
            (NamedColor::LightGreen, "light_green"),
            (NamedColor::LightYellow, "light_yellow"),
            (NamedColor::LightBlue, "light_blue"),
            (NamedColor::LightMagenta, "light_magenta"),
            (NamedColor::LightCyan, "light_cyan"),
        ];
        for (n, s) in names {
            let c = Color::Named(n);
            let json = serde_json::to_string(&c).unwrap();
            assert_eq!(json, format!("\"{}\"", s));
            let back: Color = serde_json::from_str(&json).unwrap();
            assert_eq!(back, c);
        }
    }

    #[test]
    fn color_deserialize_rejects_malformed() {
        // Wrong-length hex.
        assert!(serde_json::from_str::<Color>("\"#abc\"").is_err());
        // Hex with non-hex chars.
        assert!(serde_json::from_str::<Color>("\"#xyzxyz\"").is_err());
        // Indexed > 255.
        assert!(serde_json::from_str::<Color>("\"@256\"").is_err());
        // Indexed not a number.
        assert!(serde_json::from_str::<Color>("\"@abc\"").is_err());
        // Unknown named color.
        assert!(serde_json::from_str::<Color>("\"chartreuse\"").is_err());
        // Empty string.
        assert!(serde_json::from_str::<Color>("\"\"").is_err());
    }

    #[test]
    fn color_inside_style_round_trips() {
        let s = Style {
            fg: Some(Color::Rgb(0xaa, 0xaa, 0xaa)),
            bg: Some(Color::Default),
            bold: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        // Sanity: hex form appears, structured form doesn't.
        assert!(json.contains("\"#aaaaaa\""));
        assert!(!json.contains("\"Rgb\""));
        let back: Style = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn transition_hint_serializes_kebab_kinds() {
        let hint = TransitionHint {
            kind: TransitionKind::Enter,
            duration_ms: 200,
        };
        let json = serde_json::to_string(&hint).unwrap();
        assert!(json.contains("\"enter\""));
        let back: TransitionHint = serde_json::from_str(&json).unwrap();
        assert_eq!(hint, back);
    }
}
