//! Pulse bus types — the closed-enum versioned message catalog.
//!
//! Implements `docs/specs/pulse-bus.md` v0. The runtime that consumes
//! these (`PulseBus` — publish / publish_async / drain / subscribe)
//! lives in `devix-core::bus`.
//!
//! Per the v0 catalog, every payload is `Path`-keyed (no raw typed
//! ids). Variants are *additive* between minor versions; renames or
//! removals require a major bump. `serde(default)` is required on any
//! new field added without a major bump.
//!
//! The locked `ClientConnected` / `ClientDisconnected` variants
//! (foundations-review *Gate T-22 → Session lifecycle pulses*) are
//! included.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::input::InputEvent;
use crate::path::Path;
use crate::view::{Axis, Style};

/// The closed-enum v0 pulse catalog.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Pulse {
    // ---- Buffer / document ----
    BufferOpened {
        path: Path,
        fs_path: Option<PathBuf>,
    },
    BufferChanged {
        path: Path,
        revision: u64,
    },
    BufferSaved {
        path: Path,
        fs_path: PathBuf,
    },
    BufferReloaded {
        path: Path,
    },
    BufferClosed {
        path: Path,
    },
    DiskChanged {
        path: Path,
        fs_path: PathBuf,
    },

    // ---- Cursor / selection ----
    CursorMoved {
        cursor: Path,
        doc: Path,
        head: u64,
    },
    SelectionChanged {
        cursor: Path,
        doc: Path,
    },

    // ---- Layout / focus ----
    TabOpened {
        frame: Path,
        doc: Path,
    },
    TabActivated {
        frame: Path,
        doc: Path,
    },
    TabClosed {
        frame: Path,
        doc: Path,
    },
    FrameSplit {
        source: Path,
        new: Path,
        axis: Axis,
    },
    FrameClosed {
        frame: Path,
    },
    SidebarToggled {
        slot: Path,
        open: bool,
    },
    FocusChanged {
        from: Option<Path>,
        to: Option<Path>,
    },

    // ---- Modal ----
    ModalOpened {
        modal: ModalKind,
        frame: Option<Path>,
    },
    ModalDismissed {
        modal: ModalKind,
    },

    // ---- Commands ----
    CommandInvoked {
        command: Path,
        source: InvocationSource,
    },

    // ---- Plugin lifecycle ----
    PluginLoaded {
        plugin: Path,
        version: String,
    },
    PluginUnloaded {
        plugin: Path,
    },
    PluginError {
        plugin: Path,
        message: String,
    },

    // ---- Theme ----
    ThemeChanged {
        theme: Path,
        palette: ThemePalette,
    },

    // ---- Render coordination ----
    RenderDirty {
        reason: DirtyReason,
    },

    // ---- Process lifecycle ----
    StartupFinished,
    ShutdownRequested,

    // ---- Session lifecycle (per foundations-review Gate T-22) ----
    ClientConnected {
        client: Path,
    },
    ClientDisconnected {
        client: Path,
    },

    // ---- Frontend-originated (inbound) ----
    ViewportChanged {
        frame: Path,
        top_line: u32,
        visible_rows: u32,
    },
    InputReceived {
        event: InputEvent,
    },
}

/// Discriminant enum exposed so filters can match without
/// instantiating a payload. Hand-maintained at v0 size; switch to
/// derive macro once the catalog doubles (per pulse-bus.md Q4).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PulseKind {
    BufferOpened,
    BufferChanged,
    BufferSaved,
    BufferReloaded,
    BufferClosed,
    DiskChanged,
    CursorMoved,
    SelectionChanged,
    TabOpened,
    TabActivated,
    TabClosed,
    FrameSplit,
    FrameClosed,
    SidebarToggled,
    FocusChanged,
    ModalOpened,
    ModalDismissed,
    CommandInvoked,
    PluginLoaded,
    PluginUnloaded,
    PluginError,
    ThemeChanged,
    RenderDirty,
    StartupFinished,
    ShutdownRequested,
    ClientConnected,
    ClientDisconnected,
    ViewportChanged,
    InputReceived,
}

impl Pulse {
    /// The discriminant of this pulse, for `PulseFilter::kinds`.
    pub fn kind(&self) -> PulseKind {
        match self {
            Pulse::BufferOpened { .. } => PulseKind::BufferOpened,
            Pulse::BufferChanged { .. } => PulseKind::BufferChanged,
            Pulse::BufferSaved { .. } => PulseKind::BufferSaved,
            Pulse::BufferReloaded { .. } => PulseKind::BufferReloaded,
            Pulse::BufferClosed { .. } => PulseKind::BufferClosed,
            Pulse::DiskChanged { .. } => PulseKind::DiskChanged,
            Pulse::CursorMoved { .. } => PulseKind::CursorMoved,
            Pulse::SelectionChanged { .. } => PulseKind::SelectionChanged,
            Pulse::TabOpened { .. } => PulseKind::TabOpened,
            Pulse::TabActivated { .. } => PulseKind::TabActivated,
            Pulse::TabClosed { .. } => PulseKind::TabClosed,
            Pulse::FrameSplit { .. } => PulseKind::FrameSplit,
            Pulse::FrameClosed { .. } => PulseKind::FrameClosed,
            Pulse::SidebarToggled { .. } => PulseKind::SidebarToggled,
            Pulse::FocusChanged { .. } => PulseKind::FocusChanged,
            Pulse::ModalOpened { .. } => PulseKind::ModalOpened,
            Pulse::ModalDismissed { .. } => PulseKind::ModalDismissed,
            Pulse::CommandInvoked { .. } => PulseKind::CommandInvoked,
            Pulse::PluginLoaded { .. } => PulseKind::PluginLoaded,
            Pulse::PluginUnloaded { .. } => PulseKind::PluginUnloaded,
            Pulse::PluginError { .. } => PulseKind::PluginError,
            Pulse::ThemeChanged { .. } => PulseKind::ThemeChanged,
            Pulse::RenderDirty { .. } => PulseKind::RenderDirty,
            Pulse::StartupFinished => PulseKind::StartupFinished,
            Pulse::ShutdownRequested => PulseKind::ShutdownRequested,
            Pulse::ClientConnected { .. } => PulseKind::ClientConnected,
            Pulse::ClientDisconnected { .. } => PulseKind::ClientDisconnected,
            Pulse::ViewportChanged { .. } => PulseKind::ViewportChanged,
            Pulse::InputReceived { .. } => PulseKind::InputReceived,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InvocationSource {
    Keymap,
    Palette,
    Plugin,
    Programmatic,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ModalKind {
    Palette,
    Picker,
    Custom,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DirtyReason {
    Buffer,
    Layout,
    Theme,
    Modal,
    Frontend,
}

/// Resolved style table for the active theme. Sent to subscribers
/// (typically frontends) on `Pulse::ThemeChanged` so they can
/// interpret highlight scope names from `View::Buffer` against the
/// new palette without a separate request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThemePalette {
    pub text: Style,
    pub selection: Style,
    pub scopes: HashMap<String, Style>,
}

/// Identifies the *role* a `Path` field plays on a pulse payload —
/// not the literal field name. The same role can map to differently
/// named fields across variants (e.g., `Frame` covers both
/// `frame: Path` and `FrameSplit.source: Path`).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PulseField {
    /// Default — `BufferOpened.path`, `BufferChanged.path`,
    /// `BufferClosed.path`, `DiskChanged.path`, `BufferReloaded.path`,
    /// `BufferSaved.path`.
    Path,
    /// `CursorMoved.cursor`, `SelectionChanged.cursor`.
    Cursor,
    /// `CursorMoved.doc`, `SelectionChanged.doc`, `TabOpened.doc`,
    /// `TabActivated.doc`, `TabClosed.doc`.
    Doc,
    /// `TabOpened.frame`, `TabActivated.frame`, `TabClosed.frame`,
    /// `FrameSplit.source`, `FrameClosed.frame`, `ModalOpened.frame`,
    /// `ViewportChanged.frame`.
    Frame,
    /// `FrameSplit.new`.
    NewFrame,
    /// `SidebarToggled.slot`.
    Slot,
    /// `FocusChanged.from`.
    FocusFrom,
    /// `FocusChanged.to`.
    FocusTo,
    /// `CommandInvoked.command`.
    Command,
    /// `PluginLoaded.plugin`, `PluginUnloaded.plugin`,
    /// `PluginError.plugin`. Always shaped `/plugin/<name>`.
    Plugin,
    /// `ThemeChanged.theme`. Always shaped `/theme/<id>`.
    Theme,
    /// `ClientConnected.client`, `ClientDisconnected.client`.
    Client,
}

impl Pulse {
    /// Extract the `Path` value of `field` on this pulse, if the
    /// variant has that role. Returns `None` for variants without the
    /// role (e.g., asking for `PulseField::Cursor` on
    /// `Pulse::BufferChanged`).
    pub fn field_path(&self, field: PulseField) -> Option<&Path> {
        match (field, self) {
            // PulseField::Path
            (PulseField::Path, Pulse::BufferOpened { path, .. })
            | (PulseField::Path, Pulse::BufferChanged { path, .. })
            | (PulseField::Path, Pulse::BufferSaved { path, .. })
            | (PulseField::Path, Pulse::BufferReloaded { path })
            | (PulseField::Path, Pulse::BufferClosed { path })
            | (PulseField::Path, Pulse::DiskChanged { path, .. }) => Some(path),

            // PulseField::Cursor
            (PulseField::Cursor, Pulse::CursorMoved { cursor, .. })
            | (PulseField::Cursor, Pulse::SelectionChanged { cursor, .. }) => Some(cursor),

            // PulseField::Doc
            (PulseField::Doc, Pulse::CursorMoved { doc, .. })
            | (PulseField::Doc, Pulse::SelectionChanged { doc, .. })
            | (PulseField::Doc, Pulse::TabOpened { doc, .. })
            | (PulseField::Doc, Pulse::TabActivated { doc, .. })
            | (PulseField::Doc, Pulse::TabClosed { doc, .. }) => Some(doc),

            // PulseField::Frame
            (PulseField::Frame, Pulse::TabOpened { frame, .. })
            | (PulseField::Frame, Pulse::TabActivated { frame, .. })
            | (PulseField::Frame, Pulse::TabClosed { frame, .. })
            | (PulseField::Frame, Pulse::FrameClosed { frame })
            | (PulseField::Frame, Pulse::ViewportChanged { frame, .. }) => Some(frame),
            (PulseField::Frame, Pulse::FrameSplit { source, .. }) => Some(source),
            (PulseField::Frame, Pulse::ModalOpened { frame, .. }) => frame.as_ref(),

            // PulseField::NewFrame
            (PulseField::NewFrame, Pulse::FrameSplit { new, .. }) => Some(new),

            // PulseField::Slot
            (PulseField::Slot, Pulse::SidebarToggled { slot, .. }) => Some(slot),

            // PulseField::FocusFrom / FocusTo
            (PulseField::FocusFrom, Pulse::FocusChanged { from, .. }) => from.as_ref(),
            (PulseField::FocusTo, Pulse::FocusChanged { to, .. }) => to.as_ref(),

            // PulseField::Command
            (PulseField::Command, Pulse::CommandInvoked { command, .. }) => Some(command),

            // PulseField::Plugin
            (PulseField::Plugin, Pulse::PluginLoaded { plugin, .. })
            | (PulseField::Plugin, Pulse::PluginUnloaded { plugin })
            | (PulseField::Plugin, Pulse::PluginError { plugin, .. }) => Some(plugin),

            // PulseField::Theme
            (PulseField::Theme, Pulse::ThemeChanged { theme, .. }) => Some(theme),

            // PulseField::Client
            (PulseField::Client, Pulse::ClientConnected { client })
            | (PulseField::Client, Pulse::ClientDisconnected { client }) => Some(client),

            // No match.
            _ => None,
        }
    }
}

/// Filter shape passed to `PulseBus::subscribe`. Plugin manifests
/// deserialize their `subscribe` entries directly into this.
#[derive(Clone, Default, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PulseFilter {
    /// If set, the pulse's `kind()` must be in this list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<PulseKind>>,
    /// If set, the pulse payload's `field` value must start with this
    /// prefix (segment-aware via `Path::starts_with`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<Path>,
    /// Which `Path`-typed role on the payload to test against
    /// `path_prefix`. Defaults to `PulseField::Path` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<PulseField>,
}

impl PulseFilter {
    /// Match-anything filter (debugging / logging).
    pub fn any() -> Self {
        Self::default()
    }

    /// Match a single kind.
    pub fn kind(k: PulseKind) -> Self {
        Self {
            kinds: Some(vec![k]),
            path_prefix: None,
            field: None,
        }
    }

    /// Match a list of kinds.
    pub fn kinds<I: IntoIterator<Item = PulseKind>>(ks: I) -> Self {
        Self {
            kinds: Some(ks.into_iter().collect()),
            path_prefix: None,
            field: None,
        }
    }

    /// Match every pulse whose default `Path` field is under `prefix`.
    pub fn under(prefix: Path) -> Self {
        Self {
            kinds: None,
            path_prefix: Some(prefix),
            field: None,
        }
    }

    /// Match every pulse whose `field`-typed role is under `prefix`.
    pub fn under_field(field: PulseField, prefix: Path) -> Self {
        Self {
            kinds: None,
            path_prefix: Some(prefix),
            field: Some(field),
        }
    }

    /// Test whether `pulse` matches this filter.
    pub fn matches(&self, pulse: &Pulse) -> bool {
        if let Some(ks) = &self.kinds {
            if !ks.contains(&pulse.kind()) {
                return false;
            }
        }
        if let Some(prefix) = &self.path_prefix {
            let role = self.field.unwrap_or(PulseField::Path);
            match pulse.field_path(role) {
                Some(p) => {
                    if !p.starts_with(prefix) {
                        return false;
                    }
                }
                None => return false,
            }
        }
        true
    }
}

/// Stable handle to an active subscription. Returned from
/// `PulseBus::subscribe`; passed to `PulseBus::unsubscribe`.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
pub struct SubscriptionId(pub u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_round_trips_serde_for_every_variant() {
        let kinds = [
            PulseKind::BufferOpened,
            PulseKind::BufferChanged,
            PulseKind::BufferSaved,
            PulseKind::BufferReloaded,
            PulseKind::BufferClosed,
            PulseKind::DiskChanged,
            PulseKind::CursorMoved,
            PulseKind::SelectionChanged,
            PulseKind::TabOpened,
            PulseKind::TabActivated,
            PulseKind::TabClosed,
            PulseKind::FrameSplit,
            PulseKind::FrameClosed,
            PulseKind::SidebarToggled,
            PulseKind::FocusChanged,
            PulseKind::ModalOpened,
            PulseKind::ModalDismissed,
            PulseKind::CommandInvoked,
            PulseKind::PluginLoaded,
            PulseKind::PluginUnloaded,
            PulseKind::PluginError,
            PulseKind::ThemeChanged,
            PulseKind::RenderDirty,
            PulseKind::StartupFinished,
            PulseKind::ShutdownRequested,
            PulseKind::ClientConnected,
            PulseKind::ClientDisconnected,
            PulseKind::ViewportChanged,
            PulseKind::InputReceived,
        ];
        for k in kinds {
            let s = serde_json::to_string(&k).unwrap();
            let back: PulseKind = serde_json::from_str(&s).unwrap();
            assert_eq!(k, back);
        }
        assert_eq!(kinds.len(), 29, "v0 catalog size");
    }

    #[test]
    fn pulse_kind_matches_variant() {
        let p = Pulse::BufferChanged {
            path: Path::parse("/buf/42").unwrap(),
            revision: 1,
        };
        assert_eq!(p.kind(), PulseKind::BufferChanged);
    }

    #[test]
    fn field_path_resolves_each_role() {
        let p = Path::parse("/buf/42").unwrap();
        let pulse = Pulse::BufferChanged {
            path: p.clone(),
            revision: 1,
        };
        assert_eq!(pulse.field_path(PulseField::Path), Some(&p));
        assert_eq!(pulse.field_path(PulseField::Cursor), None);

        let cursor = Path::parse("/cur/3").unwrap();
        let doc = Path::parse("/buf/42").unwrap();
        let cm = Pulse::CursorMoved {
            cursor: cursor.clone(),
            doc: doc.clone(),
            head: 0,
        };
        assert_eq!(cm.field_path(PulseField::Cursor), Some(&cursor));
        assert_eq!(cm.field_path(PulseField::Doc), Some(&doc));
        assert_eq!(cm.field_path(PulseField::Path), None);

        let frame_a = Path::parse("/pane/0").unwrap();
        let frame_b = Path::parse("/pane/1").unwrap();
        let split = Pulse::FrameSplit {
            source: frame_a.clone(),
            new: frame_b.clone(),
            axis: Axis::Horizontal,
        };
        assert_eq!(split.field_path(PulseField::Frame), Some(&frame_a));
        assert_eq!(split.field_path(PulseField::NewFrame), Some(&frame_b));
    }

    #[test]
    fn filter_kind_matches_only_matching_kinds() {
        let filter = PulseFilter::kind(PulseKind::BufferChanged);
        let yes = Pulse::BufferChanged {
            path: Path::parse("/buf/42").unwrap(),
            revision: 1,
        };
        let no = Pulse::BufferOpened {
            path: Path::parse("/buf/42").unwrap(),
            fs_path: None,
        };
        assert!(filter.matches(&yes));
        assert!(!filter.matches(&no));
    }

    #[test]
    fn filter_under_uses_default_path_field() {
        let filter = PulseFilter::under(Path::parse("/buf").unwrap());
        let in_buf = Pulse::BufferChanged {
            path: Path::parse("/buf/42").unwrap(),
            revision: 1,
        };
        assert!(filter.matches(&in_buf));
        // CursorMoved has no `Path` role default — filter doesn't match.
        let cm = Pulse::CursorMoved {
            cursor: Path::parse("/cur/3").unwrap(),
            doc: Path::parse("/buf/42").unwrap(),
            head: 0,
        };
        assert!(!filter.matches(&cm));
    }

    #[test]
    fn filter_under_field_targets_role() {
        let filter = PulseFilter::under_field(
            PulseField::Doc,
            Path::parse("/buf/42").unwrap(),
        );
        let cm = Pulse::CursorMoved {
            cursor: Path::parse("/cur/3").unwrap(),
            doc: Path::parse("/buf/42").unwrap(),
            head: 0,
        };
        assert!(filter.matches(&cm));
        let cm_other = Pulse::CursorMoved {
            cursor: Path::parse("/cur/3").unwrap(),
            doc: Path::parse("/buf/99").unwrap(),
            head: 0,
        };
        assert!(!filter.matches(&cm_other));
    }

    #[test]
    fn filter_kind_and_path_prefix_are_anded() {
        let filter = PulseFilter {
            kinds: Some(vec![PulseKind::BufferChanged]),
            path_prefix: Some(Path::parse("/buf/42").unwrap()),
            field: Some(PulseField::Path),
        };
        // Right kind, right path: match.
        let p_match = Pulse::BufferChanged {
            path: Path::parse("/buf/42").unwrap(),
            revision: 1,
        };
        assert!(filter.matches(&p_match));
        // Right kind, wrong path.
        let p_path = Pulse::BufferChanged {
            path: Path::parse("/buf/99").unwrap(),
            revision: 1,
        };
        assert!(!filter.matches(&p_path));
        // Wrong kind, right path.
        let p_kind = Pulse::BufferOpened {
            path: Path::parse("/buf/42").unwrap(),
            fs_path: None,
        };
        assert!(!filter.matches(&p_kind));
    }

    #[test]
    fn filter_serde_round_trips() {
        let filter = PulseFilter {
            kinds: Some(vec![PulseKind::BufferChanged, PulseKind::BufferSaved]),
            path_prefix: Some(Path::parse("/buf").unwrap()),
            field: Some(PulseField::Doc),
        };
        let json = serde_json::to_string(&filter).unwrap();
        let back: PulseFilter = serde_json::from_str(&json).unwrap();
        assert_eq!(filter, back);
    }

    #[test]
    fn manifest_form_typo_in_field_fails_deserialize() {
        // `"field": "document"` is a typo for "doc"; deserialize must reject.
        let bad = r#"{"kinds":["buffer_changed"],"field":"document"}"#;
        assert!(serde_json::from_str::<PulseFilter>(bad).is_err());
    }

    #[test]
    fn pulse_serde_round_trips() {
        let pulse = Pulse::FrameSplit {
            source: Path::parse("/pane/0").unwrap(),
            new: Path::parse("/pane/1").unwrap(),
            axis: Axis::Horizontal,
        };
        let json = serde_json::to_string(&pulse).unwrap();
        let back: Pulse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind(), PulseKind::FrameSplit);
    }
}
