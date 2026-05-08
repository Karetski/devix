//! Theme store — collects every `contributes.themes` declaration
//! across built-ins and plugin manifests into a single store keyed by
//! theme id, and provides the activation entrypoint that publishes
//! `Pulse::ThemeChanged` and returns the resolved in-memory `Theme`.
//!
//! Per `manifest.md` § *contributes.themes* and `pulse-bus.md` §
//! *Theme*. T-112 ships the registry + activation seam; user-side
//! theme-switching UI is out of scope for v0.

use std::collections::HashMap;

use devix_protocol::manifest::{Manifest, ThemeSpec};
use devix_protocol::path::Path;
use devix_protocol::pulse::{Pulse, ThemePalette};

use crate::manifest_loader::theme_from_manifest_spec;
use crate::theme::Theme;
use crate::PulseBus;

/// Collected theme specs. Keyed by theme id (`ThemeSpec.id`).
/// First-loaded-wins on collisions — built-ins seed first, plugin
/// manifests register in `discover_plugin_manifests` order.
#[derive(Default)]
pub struct ThemeStore {
    by_id: HashMap<String, ThemeSpec>,
    order: Vec<String>,
}

impl ThemeStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add every `contributes.themes` entry from `manifest` into the
    /// store. Returns the count newly added — entries whose id
    /// collides with an existing one are skipped (first-loaded-wins
    /// per `manifest.md` § *Manifest discovery*).
    pub fn register_from_manifest(&mut self, manifest: &Manifest) -> usize {
        let mut added = 0usize;
        for spec in &manifest.contributes.themes {
            if self.by_id.contains_key(&spec.id) {
                continue;
            }
            self.by_id.insert(spec.id.clone(), spec.clone());
            self.order.push(spec.id.clone());
            added += 1;
        }
        added
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.order.iter().map(String::as_str)
    }

    pub fn get(&self, id: &str) -> Option<&ThemeSpec> {
        self.by_id.get(id)
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

/// Activate the theme identified by `id`. Returns the resolved
/// in-memory `Theme` ready to install into the application, and
/// publishes `Pulse::ThemeChanged { theme: /theme/<id>, palette }`
/// on `bus` so subscribers (frontends) can re-resolve their highlight
/// scope tables. Returns `None` if no theme with that id is
/// registered.
pub fn activate(store: &ThemeStore, id: &str, bus: &PulseBus) -> Option<Theme> {
    let spec = store.get(id)?;
    let theme = theme_from_manifest_spec(spec);
    let theme_path = Path::parse(&format!("/theme/{}", id))
        .expect("/theme/<id> is canonical for validated theme ids");
    let palette = palette_from_spec(spec);
    bus.publish(Pulse::ThemeChanged {
        theme: theme_path,
        palette,
    });
    Some(theme)
}

/// Build the wire-shape `ThemePalette` from a manifest theme spec.
/// Mirrors `theme_from_manifest_spec` but stays in the protocol's
/// shape so `Pulse::ThemeChanged` subscribers consume it directly.
fn palette_from_spec(spec: &ThemeSpec) -> ThemePalette {
    use devix_protocol::view::Style;
    ThemePalette {
        text: Style::default(),
        selection: Style::default(),
        scopes: spec.scopes.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devix_protocol::manifest::{Contributes, Engines, Manifest, ThemeSpec};
    use devix_protocol::protocol::ProtocolVersion;
    use devix_protocol::pulse::{PulseFilter, PulseKind};
    use devix_protocol::view::{Color, Style};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn manifest_with_theme(name: &str, theme_id: &str, scope_color: Color) -> Manifest {
        let mut scopes = HashMap::new();
        let mut style = Style::default();
        style.fg = Some(scope_color);
        scopes.insert("keyword".to_string(), style);
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
                themes: vec![ThemeSpec {
                    id: theme_id.to_string(),
                    label: theme_id.to_string(),
                    scopes,
                }],
                ..Default::default()
            },
            subscribe: Vec::new(),
        }
    }

    #[test]
    fn register_from_manifest_first_wins_on_collision() {
        let mut store = ThemeStore::new();
        let m1 = manifest_with_theme("a", "shared", Color::Rgb(1, 2, 3));
        let m2 = manifest_with_theme("b", "shared", Color::Rgb(9, 9, 9));
        assert_eq!(store.register_from_manifest(&m1), 1);
        assert_eq!(store.register_from_manifest(&m2), 0, "first-loaded wins");
        assert_eq!(store.len(), 1);
        let entry = store.get("shared").unwrap();
        let style = entry.scopes.get("keyword").unwrap();
        assert_eq!(style.fg, Some(Color::Rgb(1, 2, 3)));
    }

    #[test]
    fn activate_publishes_theme_changed_pulse() {
        let bus = PulseBus::new();
        let captured = Arc::new(Mutex::new(Vec::<Pulse>::new()));
        let cap = captured.clone();
        bus.subscribe(PulseFilter::kind(PulseKind::ThemeChanged), move |p| {
            cap.lock().unwrap().push(p.clone());
        });
        let mut store = ThemeStore::new();
        store.register_from_manifest(&manifest_with_theme(
            "p", "midnight", Color::Rgb(10, 20, 30),
        ));
        let theme = activate(&store, "midnight", &bus).expect("theme activates");
        // Resolved Theme carries the manifest's scope mapping.
        let style = theme.style_for("keyword").expect("keyword scope resolved");
        // ratatui Style has Color::Rgb(r, g, b); compare via debug fmt
        // since Style is opaque about fg accessor.
        let dbg = format!("{:?}", style);
        assert!(dbg.contains("Rgb(10, 20, 30)"), "got: {dbg}");

        let pulses = captured.lock().unwrap();
        assert_eq!(pulses.len(), 1);
        if let Pulse::ThemeChanged { theme, palette } = &pulses[0] {
            assert_eq!(theme.as_str(), "/theme/midnight");
            assert!(palette.scopes.contains_key("keyword"));
        }
    }

    #[test]
    fn activate_unknown_id_returns_none() {
        let bus = PulseBus::new();
        let store = ThemeStore::new();
        assert!(activate(&store, "nope", &bus).is_none());
    }
}
