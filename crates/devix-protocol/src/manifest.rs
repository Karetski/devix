//! Manifest schema — `docs/specs/manifest.md`.
//!
//! The same shape covers plugin manifests and the embedded built-in
//! manifest (`crates/devix-core/manifests/builtin.json`, lands in
//! T-70). The reader/validator that consumes these types lives in
//! `devix-core::manifest_loader`.
//!
//! T-33 partial: schemars JSON Schema generation (manifest.md Q5) is
//! deferred to a follow-up — emitting a clean schema requires custom
//! `JsonSchema` impls for `Path`, `Chord`, `Color`, `ProtocolVersion`,
//! which are best landed alongside their canonical-string serde
//! (T-41 / T-42). Schema generation is "polish, not blocking" per the
//! Q5 lean.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::input::Chord;
use crate::path::Path;
use crate::protocol::ProtocolVersion;
use crate::pulse::PulseFilter;
use crate::view::{SidebarSlot, Style};

/// Top-level manifest. `serde(deny_unknown_fields)` enforces strict
/// schema match — typos and forward-looking fields are rejected.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub engines: Engines,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(default, skip_serializing_if = "Contributes::is_empty")]
    pub contributes: Contributes,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscribe: Vec<SubscriptionSpec>,
}

/// Required versions of the subsystems the manifest targets.
/// `engines.devix` is the user-facing alias for `protocol_version`
/// (matches the project name in user-edited config).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Engines {
    #[serde(rename = "devix")]
    pub protocol_version: ProtocolVersion,
    pub pulse_bus: ProtocolVersion,
    pub manifest: ProtocolVersion,
}

/// Declarative contribution set.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Contributes {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<CommandSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keymaps: Vec<KeymapSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panes: Vec<PaneSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<ThemeSpec>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub settings: HashMap<String, SettingSpec>,
}

impl Contributes {
    fn is_empty(&self) -> bool {
        self.commands.is_empty()
            && self.keymaps.is_empty()
            && self.panes.is_empty()
            && self.themes.is_empty()
            && self.settings.is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CommandSpec {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// `None` for built-ins; required for plugins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lua_handle: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct KeymapSpec {
    pub key: Chord,
    /// Bare command id (`refresh`) or absolute `Path`
    /// (`/cmd/edit.copy`, `/plugin/file-tree/cmd/refresh`).
    pub command: String,
    /// Reserved for v1 conditional bindings; v0 must be `null`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PaneSpec {
    pub id: String,
    pub slot: SidebarSlot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lua_handle: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ThemeSpec {
    pub id: String,
    pub label: String,
    pub scopes: HashMap<String, Style>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SettingSpec {
    Boolean { default: bool, label: String },
    String { default: String, label: String },
    Number { default: f64, label: String },
    Enum {
        default: String,
        values: Vec<String>,
        label: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SubscriptionSpec {
    #[serde(flatten)]
    pub filter: PulseFilter,
    pub lua_handle: String,
}

/// Errors a manifest's content (after JSON parsing succeeded) can
/// fail validation with. Per `manifest.md` § *Validation*.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ManifestValidationError {
    #[error("name `{0}` does not match `^[a-z0-9][a-z0-9-]*$`")]
    InvalidName(String),
    #[error("name `{0}` uses the reserved `devix-` prefix (first-party only)")]
    ReservedNamePrefix(String),
    #[error("version `{0}` is not valid semver (`<major>.<minor>.<patch>`)")]
    InvalidVersion(String),
    #[error("command id `{0}` is not a valid path segment")]
    InvalidCommandId(String),
    #[error("pane id `{0}` is not a valid path segment")]
    InvalidPaneId(String),
    #[error("theme id `{0}` is not a valid path segment")]
    InvalidThemeId(String),
    #[error("setting key `{0}` must be dotted (`<prefix>.<name>`)")]
    InvalidSettingKey(String),
    #[error("keymap command `{0}` must be a bare segment or absolute Path")]
    InvalidKeymapCommand(String),
}

impl Manifest {
    /// Validate the post-deserialize manifest content. Per
    /// `manifest.md` § *Validation*. JSON well-formedness, schema
    /// match, `engines.devix.major` host check, etc., are upstream
    /// of this method (the loader runs serde first, then
    /// `validate`).
    pub fn validate(&self) -> Result<(), ManifestValidationError> {
        // name format
        if !is_valid_name(&self.name) {
            return Err(ManifestValidationError::InvalidName(self.name.clone()));
        }
        if self.name.starts_with("devix-") && self.name != "devix-builtin" {
            return Err(ManifestValidationError::ReservedNamePrefix(
                self.name.clone(),
            ));
        }
        // version is semver-shaped ("x.y.z" with non-empty integer-ish
        // parts; we don't pull semver as a dep at this layer).
        if !is_valid_semver(&self.version) {
            return Err(ManifestValidationError::InvalidVersion(self.version.clone()));
        }
        // commands
        for cmd in &self.contributes.commands {
            if !is_valid_segment(&cmd.id) {
                return Err(ManifestValidationError::InvalidCommandId(cmd.id.clone()));
            }
        }
        // panes
        for pane in &self.contributes.panes {
            if !is_valid_segment(&pane.id) {
                return Err(ManifestValidationError::InvalidPaneId(pane.id.clone()));
            }
        }
        // themes
        for theme in &self.contributes.themes {
            if !is_valid_segment(&theme.id) {
                return Err(ManifestValidationError::InvalidThemeId(theme.id.clone()));
            }
        }
        // settings keys
        for key in self.contributes.settings.keys() {
            if !is_dotted_segment(key) {
                return Err(ManifestValidationError::InvalidSettingKey(key.clone()));
            }
        }
        // keymap commands must parse as either bare segment or
        // absolute path.
        for keymap in &self.contributes.keymaps {
            if !is_valid_keymap_command(&keymap.command) {
                return Err(ManifestValidationError::InvalidKeymapCommand(
                    keymap.command.clone(),
                ));
            }
        }
        Ok(())
    }
}

fn is_valid_name(n: &str) -> bool {
    let mut bytes = n.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_valid_semver(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

fn is_valid_segment(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

fn is_dotted_segment(s: &str) -> bool {
    is_valid_segment(s) && s.contains('.')
}

fn is_valid_keymap_command(cmd: &str) -> bool {
    // Either an absolute path (parses as Path) or a bare segment.
    if cmd.starts_with('/') {
        Path::parse(cmd).is_ok()
    } else {
        is_valid_segment(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{Chord, KeyCode, Modifiers};

    fn good_manifest() -> Manifest {
        Manifest {
            name: "file-tree".into(),
            version: "0.1.0".into(),
            engines: Engines {
                protocol_version: ProtocolVersion::new(0, 1),
                pulse_bus: ProtocolVersion::new(0, 1),
                manifest: ProtocolVersion::new(0, 1),
            },
            entry: Some("main.lua".into()),
            contributes: Contributes::default(),
            subscribe: Vec::new(),
        }
    }

    #[test]
    fn valid_manifest_round_trips_serde() {
        let m = good_manifest();
        let json = serde_json::to_string(&m).unwrap();
        let back: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn engines_serializes_devix_alias() {
        let m = good_manifest();
        let json = serde_json::to_string(&m).unwrap();
        // The wire form uses `devix`, the Rust field is
        // `protocol_version`.
        assert!(json.contains("\"devix\":\"0.1\""));
        assert!(!json.contains("\"protocol_version\""));
    }

    #[test]
    fn deny_unknown_fields_rejects_typos() {
        let mut json = serde_json::to_value(good_manifest()).unwrap();
        json.as_object_mut()
            .unwrap()
            .insert("typo".into(), serde_json::json!("oops"));
        let s = serde_json::to_string(&json).unwrap();
        let bad = serde_json::from_str::<Manifest>(&s);
        assert!(bad.is_err());
    }

    #[test]
    fn validate_accepts_well_formed() {
        good_manifest().validate().unwrap();
    }

    #[test]
    fn validate_rejects_uppercase_name() {
        let mut m = good_manifest();
        m.name = "FileTree".into();
        assert!(matches!(
            m.validate(),
            Err(ManifestValidationError::InvalidName(_))
        ));
    }

    #[test]
    fn validate_rejects_underscore_in_name() {
        let mut m = good_manifest();
        m.name = "file_tree".into();
        assert!(matches!(
            m.validate(),
            Err(ManifestValidationError::InvalidName(_))
        ));
    }

    #[test]
    fn validate_rejects_devix_prefix_for_third_party() {
        let mut m = good_manifest();
        m.name = "devix-fancy".into();
        assert!(matches!(
            m.validate(),
            Err(ManifestValidationError::ReservedNamePrefix(_))
        ));
    }

    #[test]
    fn validate_allows_devix_builtin_exact() {
        let mut m = good_manifest();
        m.name = "devix-builtin".into();
        m.validate().unwrap();
    }

    #[test]
    fn validate_rejects_non_semver_version() {
        let mut m = good_manifest();
        m.version = "0.1".into();
        assert!(matches!(
            m.validate(),
            Err(ManifestValidationError::InvalidVersion(_))
        ));
    }

    #[test]
    fn validate_rejects_invalid_command_id() {
        let mut m = good_manifest();
        m.contributes.commands.push(CommandSpec {
            id: "not a segment".into(),
            label: "Bad".into(),
            category: None,
            lua_handle: None,
        });
        assert!(matches!(
            m.validate(),
            Err(ManifestValidationError::InvalidCommandId(_))
        ));
    }

    #[test]
    fn validate_rejects_undotted_setting_key() {
        let mut m = good_manifest();
        m.contributes
            .settings
            .insert("undotted".into(), SettingSpec::Boolean {
                default: false,
                label: "x".into(),
            });
        assert!(matches!(
            m.validate(),
            Err(ManifestValidationError::InvalidSettingKey(_))
        ));
    }

    #[test]
    fn validate_accepts_keymap_command_as_bare_or_absolute() {
        let mut m = good_manifest();
        m.contributes.keymaps.push(KeymapSpec {
            key: Chord {
                key: KeyCode::Char('a'),
                modifiers: Modifiers::default(),
            },
            command: "refresh".into(),
            when: None,
        });
        m.contributes.keymaps.push(KeymapSpec {
            key: Chord {
                key: KeyCode::Char('b'),
                modifiers: Modifiers::default(),
            },
            command: "/cmd/edit.copy".into(),
            when: None,
        });
        m.validate().unwrap();
    }

    #[test]
    fn validate_rejects_malformed_keymap_command() {
        let mut m = good_manifest();
        m.contributes.keymaps.push(KeymapSpec {
            key: Chord {
                key: KeyCode::Char('a'),
                modifiers: Modifiers::default(),
            },
            command: "not a path".into(),
            when: None,
        });
        assert!(matches!(
            m.validate(),
            Err(ManifestValidationError::InvalidKeymapCommand(_))
        ));
    }

    #[test]
    fn subscription_spec_round_trips() {
        let json = r#"{
            "kinds":["buffer_changed"],
            "lua_handle":"on_buf"
        }"#;
        let s: SubscriptionSpec = serde_json::from_str(json).unwrap();
        assert_eq!(s.lua_handle, "on_buf");
        let back = serde_json::to_string(&s).unwrap();
        let again: SubscriptionSpec = serde_json::from_str(&back).unwrap();
        assert_eq!(s, again);
    }

    #[test]
    fn setting_spec_tagged_form_round_trips() {
        let json = r#"{"type":"boolean","default":false,"label":"Show"}"#;
        let s: SettingSpec = serde_json::from_str(json).unwrap();
        match s {
            SettingSpec::Boolean { default, label } => {
                assert!(!default);
                assert_eq!(label, "Show");
            }
            _ => panic!("variant mismatch"),
        }
    }
}
