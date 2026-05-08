//! Settings store — collects every `contributes.settings` declaration
//! into a single typed key/value map, applies user-side overrides
//! from a JSON file, and exposes a typed read + mutate API.
//!
//! Per `manifest.md` § *contributes.settings* and `pulse-bus.md` §
//! *Settings*. `set` publishes `Pulse::SettingChanged` so subscribers
//! (plugin runtimes exposing `devix.on_setting_changed`, future
//! settings-UI panes) can react to runtime mutations. The Lua-side
//! bridge that calls `set` is gated on T-81 full's plugin host
//! restructure — see T-113 task notes.

use std::collections::HashMap;
use std::path::Path as FsPath;

use devix_protocol::manifest::{Manifest, SettingSpec};
use devix_protocol::path::Path;
use devix_protocol::pulse::Pulse;
use thiserror::Error;

pub use devix_protocol::manifest::SettingValue;

use crate::PulseBus;

#[derive(Default)]
pub struct SettingsStore {
    /// Resolved values keyed by dotted setting key (e.g. `editor.tab_size`).
    by_key: HashMap<String, SettingValue>,
    /// Per-key admissible enum value list, taken from the manifest.
    /// Used to validate file overrides; non-enum settings are absent
    /// from this map.
    enum_values: HashMap<String, Vec<String>>,
}

impl SettingsStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed the store from a manifest's `contributes.settings`. Each
    /// declaration installs its `default` value; collisions
    /// (`first-loaded-wins`) are silent. Returns the count newly
    /// added.
    pub fn register_from_manifest(&mut self, manifest: &Manifest) -> usize {
        let mut added = 0usize;
        for (key, spec) in &manifest.contributes.settings {
            if self.by_key.contains_key(key) {
                continue;
            }
            let value = match spec {
                SettingSpec::Boolean { default, .. } => SettingValue::Boolean(*default),
                SettingSpec::String { default, .. } => {
                    SettingValue::String(default.clone())
                }
                SettingSpec::Number { default, .. } => {
                    SettingValue::Number(*default)
                }
                SettingSpec::Enum { default, values, .. } => {
                    self.enum_values.insert(key.clone(), values.clone());
                    SettingValue::EnumString(default.clone())
                }
            };
            self.by_key.insert(key.clone(), value);
            added += 1;
        }
        added
    }

    /// Apply user-supplied overrides from a JSON file shaped as
    /// `{ "<key>": <value>, ... }`. Missing file is a silent no-op.
    /// Unknown keys (no manifest declared them) are skipped with no
    /// error — they may belong to a plugin that's currently
    /// unloaded. Type mismatches and out-of-list enum values
    /// surface as errors; the override is rejected and the
    /// previously-resolved default stays in place.
    pub fn apply_overrides_from_file(
        &mut self,
        path: &FsPath,
    ) -> Result<usize, SettingsOverrideError> {
        if !path.exists() {
            return Ok(0);
        }
        let bytes = std::fs::read(path).map_err(|source| SettingsOverrideError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let map: HashMap<String, serde_json::Value> = serde_json::from_slice(&bytes)
            .map_err(|source| SettingsOverrideError::Parse {
                path: path.to_path_buf(),
                source,
            })?;
        let mut applied = 0usize;
        for (key, raw) in map {
            let Some(existing) = self.by_key.get(&key) else {
                continue; // Unknown — likely an unloaded plugin's key.
            };
            let coerced = coerce_override(&key, raw, existing, &self.enum_values)?;
            self.by_key.insert(key, coerced);
            applied += 1;
        }
        Ok(applied)
    }

    pub fn get(&self, key: &str) -> Option<&SettingValue> {
        self.by_key.get(key)
    }

    /// Update `key` to `value` and publish `Pulse::SettingChanged` so
    /// observers (plugin runtimes, settings UI) react. Returns
    /// `false` if `key` was never registered (no manifest declared
    /// it) or the new value's type contradicts the registered shape;
    /// in either case the store is unchanged.
    pub fn set(&mut self, key: &str, value: SettingValue, bus: &PulseBus) -> bool {
        let Some(existing) = self.by_key.get(key) else {
            return false;
        };
        if !same_kind(existing, &value) {
            return false;
        }
        if let SettingValue::EnumString(s) = &value {
            if let Some(values) = self.enum_values.get(key) {
                if !values.iter().any(|v| v == s) {
                    return false;
                }
            }
        }
        self.by_key.insert(key.to_string(), value.clone());
        let setting_path = match Path::parse(&format!("/setting/{}", key)) {
            Ok(p) => p,
            Err(_) => return true,
        };
        bus.publish(Pulse::SettingChanged {
            setting: setting_path,
            value,
        });
        true
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

fn same_kind(a: &SettingValue, b: &SettingValue) -> bool {
    matches!(
        (a, b),
        (SettingValue::Boolean(_), SettingValue::Boolean(_))
            | (SettingValue::String(_), SettingValue::String(_))
            | (SettingValue::Number(_), SettingValue::Number(_))
            | (SettingValue::EnumString(_), SettingValue::EnumString(_)),
    )
}

#[derive(Debug, Error)]
pub enum SettingsOverrideError {
    #[error("reading settings overrides from `{path}`: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing settings overrides at `{path}`: {source}")]
    Parse {
        path: std::path::PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("setting `{key}`: type mismatch (expected {expected}, got {got})")]
    TypeMismatch {
        key: String,
        expected: &'static str,
        got: &'static str,
    },
    #[error(
        "setting `{key}`: value `{got}` not in admissible enum list `{values:?}`"
    )]
    EnumOutOfRange {
        key: String,
        got: String,
        values: Vec<String>,
    },
}

fn coerce_override(
    key: &str,
    raw: serde_json::Value,
    existing: &SettingValue,
    enum_values: &HashMap<String, Vec<String>>,
) -> Result<SettingValue, SettingsOverrideError> {
    use serde_json::Value as J;
    match (existing, raw) {
        (SettingValue::Boolean(_), J::Bool(b)) => Ok(SettingValue::Boolean(b)),
        (SettingValue::String(_), J::String(s)) => Ok(SettingValue::String(s)),
        (SettingValue::Number(_), J::Number(n)) => Ok(SettingValue::Number(
            n.as_f64().unwrap_or(0.0),
        )),
        (SettingValue::EnumString(_), J::String(s)) => {
            if let Some(values) = enum_values.get(key) {
                if !values.iter().any(|v| v == &s) {
                    return Err(SettingsOverrideError::EnumOutOfRange {
                        key: key.to_string(),
                        got: s,
                        values: values.clone(),
                    });
                }
            }
            Ok(SettingValue::EnumString(s))
        }
        (SettingValue::Boolean(_), other) => Err(SettingsOverrideError::TypeMismatch {
            key: key.to_string(),
            expected: "boolean",
            got: json_kind(&other),
        }),
        (SettingValue::String(_), other) => Err(SettingsOverrideError::TypeMismatch {
            key: key.to_string(),
            expected: "string",
            got: json_kind(&other),
        }),
        (SettingValue::Number(_), other) => Err(SettingsOverrideError::TypeMismatch {
            key: key.to_string(),
            expected: "number",
            got: json_kind(&other),
        }),
        (SettingValue::EnumString(_), other) => Err(SettingsOverrideError::TypeMismatch {
            key: key.to_string(),
            expected: "enum (string)",
            got: json_kind(&other),
        }),
    }
}

fn json_kind(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Resolve the user settings file path:
/// `$XDG_CONFIG_HOME/devix/settings.json` →
/// `~/.config/devix/settings.json`. Returns `None` if no candidate
/// resolves.
pub fn settings_overrides_path() -> Option<std::path::PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(
            std::path::PathBuf::from(xdg)
                .join("devix")
                .join("settings.json"),
        );
    }
    let home = std::env::var("HOME").ok()?;
    Some(
        std::path::PathBuf::from(home)
            .join(".config")
            .join("devix")
            .join("settings.json"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use devix_protocol::manifest::{Contributes, Engines, Manifest, SettingSpec};
    use devix_protocol::protocol::ProtocolVersion;
    use std::collections::HashMap;
    use std::io::Write;

    fn manifest_with_settings(
        name: &str,
        entries: Vec<(&'static str, SettingSpec)>,
    ) -> Manifest {
        let mut settings = HashMap::new();
        for (key, spec) in entries {
            settings.insert(key.to_string(), spec);
        }
        Manifest {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            engines: Engines {
                protocol_version: ProtocolVersion::new(0, 1),
                pulse_bus: ProtocolVersion::new(0, 1),
                manifest: ProtocolVersion::new(0, 1),
            },
            entry: None,
            contributes: Contributes {
                settings,
                ..Default::default()
            },
            subscribe: Vec::new(),
        }
    }

    #[test]
    fn register_seeds_defaults() {
        let mut store = SettingsStore::new();
        let m = manifest_with_settings(
            "p",
            vec![
                (
                    "p.flag",
                    SettingSpec::Boolean { default: true, label: "Flag".into() },
                ),
                (
                    "p.name",
                    SettingSpec::String {
                        default: "alpha".into(),
                        label: "Name".into(),
                    },
                ),
                (
                    "p.size",
                    SettingSpec::Number { default: 4.0, label: "Size".into() },
                ),
            ],
        );
        let added = store.register_from_manifest(&m);
        assert_eq!(added, 3);
        assert_eq!(store.get("p.flag"), Some(&SettingValue::Boolean(true)));
        assert_eq!(
            store.get("p.name"),
            Some(&SettingValue::String("alpha".to_string())),
        );
        assert_eq!(store.get("p.size"), Some(&SettingValue::Number(4.0)));
    }

    #[test]
    fn enum_default_recorded_with_values() {
        let mut store = SettingsStore::new();
        let m = manifest_with_settings(
            "p",
            vec![(
                "p.mode",
                SettingSpec::Enum {
                    default: "auto".into(),
                    values: vec!["auto".into(), "always".into(), "never".into()],
                    label: "Mode".into(),
                },
            )],
        );
        store.register_from_manifest(&m);
        assert_eq!(
            store.get("p.mode"),
            Some(&SettingValue::EnumString("auto".to_string())),
        );
    }

    #[test]
    fn first_wins_on_collision() {
        let mut store = SettingsStore::new();
        let m1 = manifest_with_settings(
            "p1",
            vec![(
                "shared.key",
                SettingSpec::Boolean { default: true, label: "X".into() },
            )],
        );
        let m2 = manifest_with_settings(
            "p2",
            vec![(
                "shared.key",
                SettingSpec::Boolean { default: false, label: "X".into() },
            )],
        );
        assert_eq!(store.register_from_manifest(&m1), 1);
        assert_eq!(store.register_from_manifest(&m2), 0);
        assert_eq!(store.get("shared.key"), Some(&SettingValue::Boolean(true)));
    }

    #[test]
    fn override_file_applies_typed_values() {
        let mut store = SettingsStore::new();
        store.register_from_manifest(&manifest_with_settings(
            "p",
            vec![
                (
                    "p.flag",
                    SettingSpec::Boolean { default: false, label: "F".into() },
                ),
                (
                    "p.name",
                    SettingSpec::String {
                        default: "alpha".into(),
                        label: "N".into(),
                    },
                ),
                (
                    "p.size",
                    SettingSpec::Number { default: 1.0, label: "S".into() },
                ),
                (
                    "p.mode",
                    SettingSpec::Enum {
                        default: "auto".into(),
                        values: vec!["auto".into(), "manual".into()],
                        label: "M".into(),
                    },
                ),
            ],
        ));

        let path = std::env::temp_dir()
            .join(format!("devix-settings-test-{}.json", std::process::id()));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(
            br#"{
                "p.flag": true,
                "p.name": "beta",
                "p.size": 7.5,
                "p.mode": "manual",
                "unknown.key": "ignored"
            }"#,
        )
        .unwrap();

        let applied = store.apply_overrides_from_file(&path).unwrap();
        assert_eq!(applied, 4, "four known keys applied; unknown.key skipped");
        assert_eq!(store.get("p.flag"), Some(&SettingValue::Boolean(true)));
        assert_eq!(
            store.get("p.name"),
            Some(&SettingValue::String("beta".to_string())),
        );
        assert_eq!(store.get("p.size"), Some(&SettingValue::Number(7.5)));
        assert_eq!(
            store.get("p.mode"),
            Some(&SettingValue::EnumString("manual".to_string())),
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn override_file_type_mismatch_errors() {
        let mut store = SettingsStore::new();
        store.register_from_manifest(&manifest_with_settings(
            "p",
            vec![(
                "p.flag",
                SettingSpec::Boolean { default: false, label: "F".into() },
            )],
        ));

        let path = std::env::temp_dir().join(format!(
            "devix-settings-mismatch-{}.json",
            std::process::id()
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(br#"{ "p.flag": "not-a-bool" }"#).unwrap();
        let err = store.apply_overrides_from_file(&path).unwrap_err();
        assert!(matches!(err, SettingsOverrideError::TypeMismatch { .. }));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn override_file_enum_out_of_range_errors() {
        let mut store = SettingsStore::new();
        store.register_from_manifest(&manifest_with_settings(
            "p",
            vec![(
                "p.mode",
                SettingSpec::Enum {
                    default: "auto".into(),
                    values: vec!["auto".into(), "manual".into()],
                    label: "M".into(),
                },
            )],
        ));

        let path = std::env::temp_dir().join(format!(
            "devix-settings-enum-{}.json",
            std::process::id()
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(br#"{ "p.mode": "nope" }"#).unwrap();
        let err = store.apply_overrides_from_file(&path).unwrap_err();
        assert!(matches!(err, SettingsOverrideError::EnumOutOfRange { .. }));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn set_mutates_value_and_publishes_setting_changed() {
        use devix_protocol::pulse::{Pulse, PulseFilter, PulseKind};
        use std::sync::{Arc, Mutex};

        let mut store = SettingsStore::new();
        store.register_from_manifest(&manifest_with_settings(
            "p",
            vec![(
                "p.flag",
                SettingSpec::Boolean { default: false, label: "F".into() },
            )],
        ));
        let bus = crate::PulseBus::new();
        let captured = Arc::new(Mutex::new(Vec::<Pulse>::new()));
        let cap = captured.clone();
        bus.subscribe(PulseFilter::kind(PulseKind::SettingChanged), move |p| {
            cap.lock().unwrap().push(p.clone());
        });

        assert!(store.set("p.flag", SettingValue::Boolean(true), &bus));
        assert_eq!(store.get("p.flag"), Some(&SettingValue::Boolean(true)));
        let pulses = captured.lock().unwrap();
        assert_eq!(pulses.len(), 1);
        if let Pulse::SettingChanged { setting, value } = &pulses[0] {
            assert_eq!(setting.as_str(), "/setting/p.flag");
            assert_eq!(*value, SettingValue::Boolean(true));
        }
    }

    #[test]
    fn set_rejects_unknown_key_and_type_mismatch() {
        let mut store = SettingsStore::new();
        store.register_from_manifest(&manifest_with_settings(
            "p",
            vec![(
                "p.flag",
                SettingSpec::Boolean { default: false, label: "F".into() },
            )],
        ));
        let bus = crate::PulseBus::new();
        // Unknown key — not in the store.
        assert!(!store.set("nope.key", SettingValue::Boolean(true), &bus));
        // Type mismatch — boolean key getting a string.
        assert!(!store.set("p.flag", SettingValue::String("x".into()), &bus));
        assert_eq!(store.get("p.flag"), Some(&SettingValue::Boolean(false)));
    }

    #[test]
    fn set_enum_rejects_out_of_range_value() {
        let mut store = SettingsStore::new();
        store.register_from_manifest(&manifest_with_settings(
            "p",
            vec![(
                "p.mode",
                SettingSpec::Enum {
                    default: "auto".into(),
                    values: vec!["auto".into(), "manual".into()],
                    label: "M".into(),
                },
            )],
        ));
        let bus = crate::PulseBus::new();
        assert!(!store.set("p.mode", SettingValue::EnumString("nope".into()), &bus));
        assert!(store.set("p.mode", SettingValue::EnumString("manual".into()), &bus));
    }

    #[test]
    fn missing_override_file_is_silent() {
        let mut store = SettingsStore::new();
        store.register_from_manifest(&manifest_with_settings(
            "p",
            vec![(
                "p.flag",
                SettingSpec::Boolean { default: false, label: "F".into() },
            )],
        ));
        let path = std::env::temp_dir().join(format!(
            "devix-settings-no-file-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let applied = store.apply_overrides_from_file(&path).unwrap();
        assert_eq!(applied, 0);
        assert_eq!(store.get("p.flag"), Some(&SettingValue::Boolean(false)));
    }
}
