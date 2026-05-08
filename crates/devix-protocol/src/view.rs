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

    /// Document body. Frontend handles virtualization, horizontal
    /// scroll, gutter rendering. Highlights ship as scope names
    /// (per *Resolved during initial review*); the frontend resolves
    /// them against the active palette delivered via
    /// `Pulse::ThemeChanged`.
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

/// Color value. Stub serde at T-40 (default externally-tagged form);
/// T-41 replaces with the canonical string form (`"default"` /
/// `"#rrggbb"` / `"@<n>"` / `"<named>"`) per
/// `docs/specs/frontend.md` § *Color serialization*.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Color {
    Default,
    Rgb(u8, u8, u8),
    Indexed(u8),
    Named(NamedColor),
}

/// Named ANSI/VT100-equivalent colors.
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
