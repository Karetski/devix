//! Manifest reader / validator / discovery — `docs/specs/manifest.md`.
//!
//! T-33 ships the read+validate path and the discovery helper.
//! Concrete contribution wiring (registering loaded commands /
//! keymaps / themes / panes / settings into the live registries)
//! lands in T-70..T-74 (built-ins) and T-110..T-113 (plugins).

use std::path::{Path as FsPath, PathBuf};

use devix_protocol::manifest::{Manifest, ManifestValidationError};
use thiserror::Error;

/// Errors a manifest load can surface. Wraps both I/O / parse and
/// the post-deserialize content validation.
#[derive(Debug, Error)]
pub enum ManifestLoadError {
    #[error("reading manifest from `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing manifest at `{path}`: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("validating manifest at `{path}`: {source}")]
    Validate {
        path: PathBuf,
        #[source]
        source: ManifestValidationError,
    },
}

/// Load a manifest from a JSON file. Reads, parses, validates.
/// Returns the validated `Manifest` on success.
pub fn load_manifest(path: &FsPath) -> Result<Manifest, ManifestLoadError> {
    let bytes = std::fs::read(path).map_err(|source| ManifestLoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_manifest_bytes(&bytes, path)
}

/// Parse + validate a manifest from already-loaded bytes. Used both
/// for filesystem-loaded plugin manifests and the embedded built-in
/// manifest (`include_str!` at T-70).
pub fn parse_manifest_bytes(
    bytes: &[u8],
    path: &FsPath,
) -> Result<Manifest, ManifestLoadError> {
    let manifest: Manifest =
        serde_json::from_slice(bytes).map_err(|source| ManifestLoadError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
    manifest.validate().map_err(|source| ManifestLoadError::Validate {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(manifest)
}

/// Resolve the plugin directory per `manifest.md` § *Manifest
/// discovery*:
///
/// 1. `DEVIX_PLUGIN_DIR` env var (overrides default; primarily for
///    tests).
/// 2. `$XDG_CONFIG_HOME/devix/plugins/`.
/// 3. `~/.config/devix/plugins/`.
///
/// Returns `None` if no candidate resolves (no env var, no XDG, no
/// home dir).
pub fn plugin_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("DEVIX_PLUGIN_DIR") {
        return Some(PathBuf::from(p));
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("devix").join("plugins"));
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".config").join("devix").join("plugins"))
}

/// Register the built-in `contributes.commands` from `manifest`
/// into `reg`. Each manifest entry's bare `id` resolves through the
/// caller-supplied resolver (typically
/// `crate::editor::commands::cmd::handler_for_builtin_id` for the
/// built-in manifest); `label` and `category` come from the manifest.
///
/// Returns the count registered. Per `manifest.md`, an unknown id
/// (no Rust handler) is a load-time error; this function returns
/// the first such error rather than silently skipping.
pub fn register_command_contributions<F>(
    reg: &mut crate::editor::commands::registry::CommandRegistry,
    manifest: &Manifest,
    resolver: F,
) -> Result<usize, ManifestRegisterError>
where
    F: Fn(&str) -> Option<std::sync::Arc<dyn crate::editor::commands::cmd::EditorCommand>>,
{
    let mut count = 0usize;
    for spec in &manifest.contributes.commands {
        let action = resolver(&spec.id).ok_or_else(|| {
            ManifestRegisterError::NoHandlerForCommand(spec.id.clone())
        })?;
        let id = crate::editor::commands::registry::CommandId::builtin(intern_id(&spec.id));
        let label = intern_id(&spec.label);
        let category = spec.category.as_ref().map(|c| intern_id(c));
        reg.register(crate::editor::commands::registry::Command {
            id,
            label,
            category,
            action,
        });
        count += 1;
    }
    Ok(count)
}

/// Errors registering a manifest's contributions into the live
/// registries. Distinct from `ManifestLoadError` (which covers
/// I/O / parse / schema validation); registration errors mean the
/// manifest validated but referenced something the host can't
/// satisfy (e.g., a command id with no Rust handler).
#[derive(Debug, thiserror::Error)]
pub enum ManifestRegisterError {
    #[error("no Rust handler registered for built-in command `{0}`")]
    NoHandlerForCommand(String),
    #[error("keymap binding references unknown command id `{0}`")]
    UnknownKeymapCommand(String),
    #[error("keymap chord `{chord}` cannot be converted to crossterm shape: {reason}")]
    UnsupportedChord {
        chord: String,
        reason: String,
    },
}

/// Register the built-in `contributes.keymaps` from `manifest` into
/// `keymap`, resolving each binding's `command` (bare id) against
/// `commands` (so bind sites point at registered command ids).
/// Returns the count registered. Unknown command ids and chord
/// conversion failures are returned as `ManifestRegisterError`.
pub fn register_keymap_contributions(
    keymap: &mut crate::editor::commands::keymap::Keymap,
    manifest: &Manifest,
    commands: &crate::editor::commands::registry::CommandRegistry,
) -> Result<usize, ManifestRegisterError> {
    let mut count = 0usize;
    for binding in &manifest.contributes.keymaps {
        // The binding's command is a bare id (e.g., "edit.copy") or
        // an absolute Path. T-72 only handles bare ids; absolute
        // /cmd/<id> paths land when plugin contributions arrive
        // (T-110+).
        let bare_id: &str = if binding.command.starts_with('/') {
            // Absolute /cmd/<dotted-id> — strip the prefix.
            binding.command.strip_prefix("/cmd/").ok_or_else(|| {
                ManifestRegisterError::UnknownKeymapCommand(binding.command.clone())
            })?
        } else {
            &binding.command
        };

        // Look the id up in the live registry to get its CommandId
        // (the leaked `&'static str`); fall back to interning if
        // not found, which surfaces as a load-time error since the
        // command must exist before its keymap is registered.
        let cmd_id = command_id_in_registry(commands, bare_id).ok_or_else(|| {
            ManifestRegisterError::UnknownKeymapCommand(bare_id.to_string())
        })?;

        let chord = chord_from_protocol(&binding.key).map_err(|reason| {
            ManifestRegisterError::UnsupportedChord {
                chord: format!("{}", binding.key),
                reason,
            }
        })?;
        keymap.bind_command(chord, cmd_id);
        count += 1;
    }
    Ok(count)
}

fn command_id_in_registry(
    commands: &crate::editor::commands::registry::CommandRegistry,
    id: &str,
) -> Option<crate::editor::commands::registry::CommandId> {
    use devix_protocol::Lookup;
    let path = devix_protocol::path::Path::parse(&format!("/cmd/{}", id)).ok()?;
    if commands.lookup(&path).is_some() {
        Some(crate::editor::commands::registry::CommandId::builtin(intern_id(id)))
    } else {
        None
    }
}

/// Convert a `devix_protocol::input::Chord` (wire shape) to the
/// keymap's crossterm-flavored `Chord` (KeyCode + KeyModifiers).
/// Returns the raw error string on failure.
fn chord_from_protocol(
    p: &devix_protocol::input::Chord,
) -> Result<crate::editor::commands::keymap::Chord, String> {
    use crossterm::event::{KeyCode as CtCode, KeyModifiers};
    use devix_protocol::input::KeyCode as PKey;

    let code = match p.key {
        PKey::Char(c) => CtCode::Char(c),
        PKey::Enter => CtCode::Enter,
        PKey::Tab => CtCode::Tab,
        PKey::BackTab => CtCode::BackTab,
        PKey::Esc => CtCode::Esc,
        PKey::Backspace => CtCode::Backspace,
        PKey::Delete => CtCode::Delete,
        PKey::Insert => CtCode::Insert,
        PKey::Left => CtCode::Left,
        PKey::Right => CtCode::Right,
        PKey::Up => CtCode::Up,
        PKey::Down => CtCode::Down,
        PKey::Home => CtCode::Home,
        PKey::End => CtCode::End,
        PKey::PageUp => CtCode::PageUp,
        PKey::PageDown => CtCode::PageDown,
        PKey::F(n) if (1..=12).contains(&n) => CtCode::F(n),
        PKey::F(n) => return Err(format!("F-key {} out of range", n)),
    };
    let mut mods = KeyModifiers::NONE;
    if p.modifiers.ctrl {
        mods |= KeyModifiers::CONTROL;
    }
    if p.modifiers.alt {
        mods |= KeyModifiers::ALT;
    }
    if p.modifiers.shift {
        mods |= KeyModifiers::SHIFT;
    }
    if p.modifiers.super_key {
        mods |= KeyModifiers::SUPER;
    }
    Ok(crate::editor::commands::keymap::Chord::new(code, mods))
}

/// Intern a `String` as `&'static str`. Used to satisfy `CommandId`'s
/// `&'static str` shape from a runtime-loaded manifest. Strings stay
/// alive for the process lifetime; built-ins are loaded once.
fn intern_id(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// Construct a `Theme` from a manifest's theme spec.
/// Maps protocol `Style` / `Color` to the in-memory ratatui shape
/// the renderer consumes. Returns `None` only for empty palettes
/// (a theme must have at least one scope).
pub fn theme_from_manifest_spec(spec: &devix_protocol::ThemeSpec) -> crate::theme::Theme {
    use crate::theme::Theme;
    use ratatui::style::{Modifier, Style as RtStyle};

    // Default text + selection styles. Manifest doesn't yet carry a
    // top-level text/selection palette (spec extension would be
    // required); for now the editor's existing One-Dark fallbacks
    // apply to manifest-loaded themes that don't override them
    // through scopes.
    let text = RtStyle::default().fg(ratatui::style::Color::Rgb(0xab, 0xb2, 0xbf));
    let selection = RtStyle::default().bg(ratatui::style::Color::Rgb(60, 80, 130));
    let mut theme = Theme::new(text, selection);

    for (scope, style) in &spec.scopes {
        let mut rt = RtStyle::default();
        if let Some(fg) = style.fg {
            rt = rt.fg(protocol_color_to_ratatui(fg));
        }
        if let Some(bg) = style.bg {
            rt = rt.bg(protocol_color_to_ratatui(bg));
        }
        if style.bold {
            rt = rt.add_modifier(Modifier::BOLD);
        }
        if style.italic {
            rt = rt.add_modifier(Modifier::ITALIC);
        }
        if style.underline {
            rt = rt.add_modifier(Modifier::UNDERLINED);
        }
        if style.dim {
            rt = rt.add_modifier(Modifier::DIM);
        }
        if style.reverse {
            rt = rt.add_modifier(Modifier::REVERSED);
        }
        theme = theme.with_scope(scope.clone(), rt);
    }
    theme
}

/// Find a theme by id in the manifest's themes list and construct it.
pub fn theme_from_manifest(
    manifest: &Manifest,
    id: &str,
) -> Option<crate::theme::Theme> {
    let spec = manifest.contributes.themes.iter().find(|t| t.id == id)?;
    Some(theme_from_manifest_spec(spec))
}

fn protocol_color_to_ratatui(c: devix_protocol::view::Color) -> ratatui::style::Color {
    use devix_protocol::view::{Color, NamedColor};
    use ratatui::style::Color as Rt;
    match c {
        Color::Default => Rt::Reset,
        Color::Rgb(r, g, b) => Rt::Rgb(r, g, b),
        Color::Indexed(n) => Rt::Indexed(n),
        Color::Named(n) => match n {
            NamedColor::Black => Rt::Black,
            NamedColor::Red => Rt::Red,
            NamedColor::Green => Rt::Green,
            NamedColor::Yellow => Rt::Yellow,
            NamedColor::Blue => Rt::Blue,
            NamedColor::Magenta => Rt::Magenta,
            NamedColor::Cyan => Rt::Cyan,
            NamedColor::White => Rt::White,
            NamedColor::DarkGray => Rt::DarkGray,
            NamedColor::LightRed => Rt::LightRed,
            NamedColor::LightGreen => Rt::LightGreen,
            NamedColor::LightYellow => Rt::LightYellow,
            NamedColor::LightBlue => Rt::LightBlue,
            NamedColor::LightMagenta => Rt::LightMagenta,
            NamedColor::LightCyan => Rt::LightCyan,
        },
    }
}

/// User keymap override file. Resolved from
/// `$XDG_CONFIG_HOME/devix/keymap-overrides.json` (or the
/// `~/.config/devix/...` fallback), parsed as
/// `{"<chord>": "<command-id-or-path>", ...}`. Applied *after*
/// every manifest's keymap has loaded so user picks displace any
/// builtin / plugin binding for the named chord.
///
/// Returns the number of overrides applied. Missing file is a
/// silent no-op (no overrides = no error).
pub fn apply_keymap_overrides(
    keymap: &mut crate::editor::commands::keymap::Keymap,
    commands: &crate::editor::commands::registry::CommandRegistry,
    overrides_path: &FsPath,
) -> Result<usize, ManifestRegisterError> {
    if !overrides_path.exists() {
        return Ok(0);
    }
    let bytes = match std::fs::read(overrides_path) {
        Ok(b) => b,
        Err(_) => return Ok(0),
    };
    let map: std::collections::HashMap<String, String> =
        serde_json::from_slice(&bytes).map_err(|e| {
            ManifestRegisterError::UnsupportedChord {
                chord: overrides_path.to_string_lossy().to_string(),
                reason: format!("parse error: {}", e),
            }
        })?;
    let mut count = 0usize;
    for (chord_str, cmd_str) in map {
        let proto: devix_protocol::input::Chord =
            devix_protocol::input::Chord::parse(&chord_str)
                .map_err(|reason| ManifestRegisterError::UnsupportedChord {
                    chord: chord_str.clone(),
                    reason,
                })?;
        let chord = chord_from_protocol(&proto).map_err(|reason| {
            ManifestRegisterError::UnsupportedChord {
                chord: chord_str.clone(),
                reason,
            }
        })?;
        let bare_id: &str = if let Some(s) = cmd_str.strip_prefix("/cmd/") {
            s
        } else {
            cmd_str.as_str()
        };
        let cmd_id = command_id_in_registry(commands, bare_id).ok_or_else(|| {
            ManifestRegisterError::UnknownKeymapCommand(bare_id.to_string())
        })?;
        keymap.bind_command(chord, cmd_id);
        count += 1;
    }
    Ok(count)
}

/// Resolve the user keymap-overrides path:
/// `$XDG_CONFIG_HOME/devix/keymap-overrides.json` →
/// `~/.config/devix/keymap-overrides.json`. Returns `None` if no
/// candidate resolves.
pub fn keymap_overrides_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("devix").join("keymap-overrides.json"));
    }
    let home = std::env::var("HOME").ok()?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("devix")
            .join("keymap-overrides.json"),
    )
}

/// Discover every plugin under `dir`. Each subdirectory containing
/// a `manifest.json` is a plugin candidate. Returns the list of
/// candidate manifest paths in alphabetical (loader-deterministic)
/// order. The caller is responsible for invoking `load_manifest` on
/// each entry — discovery is a separate step from validation.
pub fn discover_plugin_manifests(dir: &FsPath) -> std::io::Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            if !p.is_dir() {
                return None;
            }
            let manifest = p.join("manifest.json");
            manifest.is_file().then_some(manifest)
        })
        .collect();
    entries.sort();
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn good_manifest_json() -> &'static str {
        r#"{
            "name": "file-tree",
            "version": "0.1.0",
            "engines": { "devix": "0.1", "pulse_bus": "0.1", "manifest": "0.1" },
            "entry": "main.lua"
        }"#
    }

    #[test]
    fn parse_manifest_bytes_round_trips_good_manifest() {
        let m = parse_manifest_bytes(
            good_manifest_json().as_bytes(),
            FsPath::new("/test/manifest.json"),
        )
        .unwrap();
        assert_eq!(m.name, "file-tree");
        assert_eq!(m.version, "0.1.0");
    }

    #[test]
    fn parse_rejects_unknown_field() {
        let json = r#"{
            "name": "ok",
            "version": "0.1.0",
            "engines": { "devix": "0.1", "pulse_bus": "0.1", "manifest": "0.1" },
            "typo": "oops"
        }"#;
        let bad = parse_manifest_bytes(json.as_bytes(), FsPath::new("/test/manifest.json"));
        assert!(matches!(bad, Err(ManifestLoadError::Parse { .. })));
    }

    #[test]
    fn parse_rejects_invalid_name() {
        let json = r#"{
            "name": "Bad_Name",
            "version": "0.1.0",
            "engines": { "devix": "0.1", "pulse_bus": "0.1", "manifest": "0.1" }
        }"#;
        let bad = parse_manifest_bytes(json.as_bytes(), FsPath::new("/test/manifest.json"));
        assert!(matches!(bad, Err(ManifestLoadError::Validate { .. })));
    }

    #[test]
    fn discover_finds_subdirs_with_manifest_json() {
        let tmp = tempdir();
        let plugin_a = tmp.join("alpha");
        let plugin_b = tmp.join("bravo");
        let no_manifest = tmp.join("charlie");
        std::fs::create_dir_all(&plugin_a).unwrap();
        std::fs::create_dir_all(&plugin_b).unwrap();
        std::fs::create_dir_all(&no_manifest).unwrap();
        write_file(&plugin_a.join("manifest.json"), b"{}");
        write_file(&plugin_b.join("manifest.json"), b"{}");
        // charlie has no manifest.json — should be skipped.

        let found = discover_plugin_manifests(&tmp).unwrap();
        assert_eq!(found.len(), 2);
        // Alphabetical order.
        assert!(found[0].ends_with("alpha/manifest.json"));
        assert!(found[1].ends_with("bravo/manifest.json"));

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn discover_returns_empty_for_missing_dir() {
        let nonexistent = std::env::temp_dir().join("devix-discover-test-no-such-dir");
        let _ = std::fs::remove_dir_all(&nonexistent);
        let found = discover_plugin_manifests(&nonexistent).unwrap();
        assert!(found.is_empty());
    }

    fn tempdir() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "devix-manifest-loader-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn write_file(path: &FsPath, bytes: &[u8]) {
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(bytes).unwrap();
    }
}
