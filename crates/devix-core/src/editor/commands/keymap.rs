//! Chord → command resolution.
//!
//! The keymap maps a [`Chord`] to either a [`CommandId`] (for discoverable,
//! palette-listable commands registered in [`crate::command`]) or directly
//! to an `Arc<dyn EditorCommand>` (for continuously-fired actions like
//! motion, character insertion, mouse — these never appear in the palette
//! so they bypass the registry).
//!
//! Phase 5: storage migrated from `Action` enum values to trait objects.
//! `lookup` returns `Arc<dyn EditorCommand>` so the caller can drop the
//! immutable keymap/registry borrow before invoking with `&mut Context`.

use std::collections::HashMap;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use devix_protocol::Lookup;
use devix_protocol::path::Path;

use crate::editor::commands::builtins as cmd_id;
use crate::editor::commands::cmd::{self, EditorCommand};
use crate::editor::commands::registry::{CommandId, CommandRegistry};
use crate::{Direction, SidebarSlot};

#[derive(Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct Chord {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl Chord {
    pub fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        Self { code, mods }
    }
}

enum Binding {
    Command(CommandId),
    Action(Arc<dyn EditorCommand>),
}

pub struct Keymap {
    bindings: HashMap<Chord, Binding>,
    /// Inverse index: command id → first chord bound to it. Built lazily;
    /// rebuilt on every `bind_command`. Kept as a small map (~30 entries) so
    /// the rebuild cost is irrelevant.
    chord_for: HashMap<CommandId, Chord>,
    /// Path-keyed cache: `/keymap/<chord>` → `/cmd/<dotted-id>`. Only
    /// `Binding::Command` entries appear here — `Binding::Action`
    /// has no command-id path. Built lazily on `bind_command`.
    bound_paths: HashMap<Path, Path>,
}

impl Keymap {
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
            chord_for: HashMap::new(),
            bound_paths: HashMap::new(),
        }
    }

    pub fn bind_command(&mut self, chord: Chord, id: CommandId) {
        self.bindings.insert(chord, Binding::Command(id));
        // Only record the *first* chord we see for an id — palettes typically
        // show one canonical hint, not every alias (Ctrl+W vs Ctrl+F4).
        self.chord_for.entry(id).or_insert(chord);
        // Maintain the path-keyed cache for `Lookup<Resource = Path>`.
        if let Some(chord_path) = chord_to_keymap_path(chord) {
            self.bound_paths.insert(chord_path, id.to_path());
        }
    }

    /// Bind a chord to a command iff the chord is currently unbound.
    /// Returns `true` if newly bound, `false` if a binding already
    /// existed (left intact). Used by plugin-manifest registration
    /// per `manifest.md` § *Manifest discovery* — first-loaded-wins
    /// on chord conflicts.
    pub fn bind_command_if_free(&mut self, chord: Chord, id: CommandId) -> bool {
        if self.bindings.contains_key(&chord) {
            return false;
        }
        self.bind_command(chord, id);
        true
    }

    pub fn bind_action(&mut self, chord: Chord, action: Arc<dyn EditorCommand>) {
        self.bindings.insert(chord, Binding::Action(action));
        // Action bindings have no command-id path, so they don't
        // populate `bound_paths` — they're invisible to the
        // namespace lookup.
    }

    /// Resolve a chord to its bound action via this keymap's
    /// bindings (and the registry, for `Binding::Command`). Renamed
    /// from `lookup` so it doesn't clash with the
    /// `Lookup::lookup(&Path)` trait impl.
    pub fn resolve_chord(
        &self,
        chord: Chord,
        reg: &CommandRegistry,
    ) -> Option<Arc<dyn EditorCommand>> {
        match self.bindings.get(&chord)? {
            Binding::Command(id) => reg.resolve(*id),
            Binding::Action(a) => Some(a.clone()),
        }
    }

    /// First chord bound to `id`, for palette / status-line display.
    pub fn chord_for(&self, id: CommandId) -> Option<Chord> {
        self.chord_for.get(&id).copied()
    }
}

impl Default for Keymap {
    fn default() -> Self { Self::new() }
}

impl Lookup for Keymap {
    type Resource = Path;

    fn lookup(&self, path: &Path) -> Option<&Path> {
        self.bound_paths.get(path)
    }

    /// Path-keyed mutation goes through `bind_command`, not
    /// `lookup_mut` (consistent with the 2026-05-07 lookup_mut
    /// resolution).
    fn lookup_mut(&mut self, _path: &Path) -> Option<&mut Path> {
        None
    }

    fn paths(&self) -> Box<dyn Iterator<Item = Path> + '_> {
        Box::new(self.bound_paths.keys().cloned())
    }
}

/// Encode a keymap `Chord` (crossterm-flavored) into the canonical
/// `/keymap/<chord>` path. Returns `None` if the chord can't be
/// represented in the canonical kebab-case form (e.g.
/// non-printable / unknown KeyCode variants).
fn chord_to_keymap_path(chord: Chord) -> Option<Path> {
    let proto = crossterm_chord_to_protocol(chord)?;
    let segment = format!("{}", proto);
    Path::parse(&format!("/keymap/{}", segment)).ok()
}

fn crossterm_chord_to_protocol(chord: Chord) -> Option<devix_protocol::input::Chord> {
    use crossterm::event::KeyCode as CtCode;
    use devix_protocol::input::{Chord as PChord, KeyCode as PKey, Modifiers as PMods};

    let key = match chord.code {
        CtCode::Char(c) => PKey::Char(c.to_ascii_lowercase()),
        CtCode::Enter => PKey::Enter,
        CtCode::Tab => PKey::Tab,
        CtCode::BackTab => PKey::BackTab,
        CtCode::Esc => PKey::Esc,
        CtCode::Backspace => PKey::Backspace,
        CtCode::Delete => PKey::Delete,
        CtCode::Insert => PKey::Insert,
        CtCode::Left => PKey::Left,
        CtCode::Right => PKey::Right,
        CtCode::Up => PKey::Up,
        CtCode::Down => PKey::Down,
        CtCode::Home => PKey::Home,
        CtCode::End => PKey::End,
        CtCode::PageUp => PKey::PageUp,
        CtCode::PageDown => PKey::PageDown,
        CtCode::F(n) if (1..=12).contains(&n) => PKey::F(n),
        _ => return None,
    };
    let modifiers = PMods {
        ctrl: chord.mods.contains(KeyModifiers::CONTROL),
        alt: chord.mods.contains(KeyModifiers::ALT),
        shift: chord.mods.contains(KeyModifiers::SHIFT),
        super_key: chord.mods.contains(KeyModifiers::SUPER),
    };
    Some(PChord { key, modifiers })
}

const C: KeyModifiers = KeyModifiers::CONTROL;
const S: KeyModifiers = KeyModifiers::SHIFT;
const A: KeyModifiers = KeyModifiers::ALT;
const NONE: KeyModifiers = KeyModifiers::NONE;

fn chord(code: KeyCode, mods: KeyModifiers) -> Chord { Chord::new(code, mods) }
fn ch(c: char) -> KeyCode { KeyCode::Char(c) }

pub fn default_keymap() -> Keymap {
    let mut k = Keymap::new();

    // ---- manifest-driven bind_command bindings ----
    // T-74 retired the hand-maintained bind_command list here. The
    // manifest at crates/devix-core/manifests/builtin.json is the
    // single source of truth for chord → command id; this function
    // parses it once and registers every entry.
    let reg = crate::editor::commands::builtins::build_registry();
    let manifest = crate::manifest_loader::parse_manifest_bytes(
        crate::BUILTIN_MANIFEST.as_bytes(),
        std::path::Path::new("crates/devix-core/manifests/builtin.json"),
    )
    .expect("BUILTIN_MANIFEST always parses");
    crate::manifest_loader::register_keymap_contributions(&mut k, &manifest, &reg)
        .expect("BUILTIN_MANIFEST keymap entries must register");

    // ---- supplemental keymap bindings not yet in the manifest ----
    // Some bindings have characters reserved by the path grammar
    // (`-`, `[`, `]`, `{`, `}`, `\`) or carry macOS Terminal.app
    // workarounds (Ctrl+Shift+[ = '{' on shifted symbol). Until a
    // chord encoding for these lands (likely a Minus / Bracket /
    // Brace KeyCode variant), they keep the in-source binding form.
    k.bind_command(chord(KeyCode::Char('['), C | S),        cmd_id::TAB_PREV);
    k.bind_command(chord(KeyCode::Char(']'), C | S),        cmd_id::TAB_NEXT);
    k.bind_command(chord(KeyCode::Char('{'), C),            cmd_id::TAB_PREV);
    k.bind_command(chord(KeyCode::Char('}'), C),            cmd_id::TAB_NEXT);
    k.bind_command(chord(KeyCode::PageUp, C),               cmd_id::TAB_PREV);
    k.bind_command(chord(KeyCode::PageDown, C),             cmd_id::TAB_NEXT);
    k.bind_command(chord(ch('-'), C),                       cmd_id::SPLIT_HORIZONTAL);

    // ---- direct actions (continuous; not registry commands) ----
    // Motion — both extend variants per chord
    for &(extend, sm) in &[(false, NONE), (true, S)] {
        k.bind_action(chord(KeyCode::Left,  C | sm), Arc::new(cmd::MoveLineStart { extend }));
        k.bind_action(chord(KeyCode::Right, C | sm), Arc::new(cmd::MoveLineEnd   { extend }));
        k.bind_action(chord(KeyCode::Up,    C | sm), Arc::new(cmd::MoveDocStart  { extend }));
        k.bind_action(chord(KeyCode::Down,  C | sm), Arc::new(cmd::MoveDocEnd    { extend }));

        k.bind_action(chord(KeyCode::Left,  A | sm), Arc::new(cmd::MoveWordLeft  { extend }));
        k.bind_action(chord(KeyCode::Right, A | sm), Arc::new(cmd::MoveWordRight { extend }));

        k.bind_action(chord(KeyCode::Left,  sm), Arc::new(cmd::MoveLeft  { extend }));
        k.bind_action(chord(KeyCode::Right, sm), Arc::new(cmd::MoveRight { extend }));
        k.bind_action(chord(KeyCode::Up,    sm), Arc::new(cmd::MoveUp    { extend }));
        k.bind_action(chord(KeyCode::Down,  sm), Arc::new(cmd::MoveDown  { extend }));

        k.bind_action(chord(KeyCode::Home,     sm), Arc::new(cmd::MoveLineStart { extend }));
        k.bind_action(chord(KeyCode::End,      sm), Arc::new(cmd::MoveLineEnd   { extend }));
        k.bind_action(chord(KeyCode::PageUp,   sm), Arc::new(cmd::PageUp        { extend }));
        k.bind_action(chord(KeyCode::PageDown, sm), Arc::new(cmd::PageDown      { extend }));

        // Ctrl+Home / Ctrl+End — fallback for Ctrl+Up / Ctrl+Down which macOS
        // swallows for Mission Control unless those system shortcuts are off.
        k.bind_action(chord(KeyCode::Home, C | sm), Arc::new(cmd::MoveDocStart { extend }));
        k.bind_action(chord(KeyCode::End,  C | sm), Arc::new(cmd::MoveDocEnd   { extend }));
    }

    // ESC b / ESC f for Option+Left/Right on terminals (macOS Terminal.app)
    // that emit the legacy meta encoding. No shift bit in that encoding.
    k.bind_action(chord(ch('b'), A), Arc::new(cmd::MoveWordLeft  { extend: false }));
    k.bind_action(chord(ch('f'), A), Arc::new(cmd::MoveWordRight { extend: false }));

    // Directional focus traversal
    k.bind_action(chord(KeyCode::Left,  C | A), Arc::new(cmd::FocusDir(Direction::Left)));
    k.bind_action(chord(KeyCode::Right, C | A), Arc::new(cmd::FocusDir(Direction::Right)));
    k.bind_action(chord(KeyCode::Up,    C | A), Arc::new(cmd::FocusDir(Direction::Up)));
    k.bind_action(chord(KeyCode::Down,  C | A), Arc::new(cmd::FocusDir(Direction::Down)));

    // Edits
    k.bind_action(chord(KeyCode::Backspace, NONE), Arc::new(cmd::DeleteBack    { word: false }));
    k.bind_action(chord(KeyCode::Backspace, A),    Arc::new(cmd::DeleteBack    { word: true  }));
    k.bind_action(chord(KeyCode::Delete,    NONE), Arc::new(cmd::DeleteForward { word: false }));
    k.bind_action(chord(KeyCode::Delete,    A),    Arc::new(cmd::DeleteForward { word: true  }));
    k.bind_action(chord(KeyCode::Enter,     NONE), Arc::new(cmd::InsertNewline));
    k.bind_action(chord(KeyCode::Tab,       NONE), Arc::new(cmd::InsertTab));

    // Bind each `SidebarSlot` so the keymap stays self-contained — the only
    // chord-bound place that uses the slot enum.
    let _ = SidebarSlot::Left;

    // Multi-cursor.
    //
    // `Shift+Ctrl+Up/Down` overrides the `MoveDocStart/End { extend: true }`
    // bindings the loop above installed for those chords — extend-to-doc
    // is still reachable via `Shift+Ctrl+Home/End`, which is the standard
    // cross-platform chord. `Esc` collapses multi-cursor back to primary
    // (and an active selection to a point) when no completion or modal is
    // intercepting it.
    k.bind_command(chord(KeyCode::Up,   C | S),             cmd_id::EDIT_ADD_CURSOR_ABOVE);
    k.bind_command(chord(KeyCode::Down, C | S),             cmd_id::EDIT_ADD_CURSOR_BELOW);
    k.bind_command(chord(KeyCode::Esc, NONE),               cmd_id::EDIT_COLLAPSE_SELECTION);

    k
}

/// Normalize a `KeyEvent` into a `Chord`. Lowercases ASCII alphabetic chars
/// (so Ctrl+s and Ctrl+S share a chord); preserves all modifier bits.
pub fn chord_from_key(code: KeyCode, mods: KeyModifiers) -> Chord {
    let code = match code {
        KeyCode::Char(c) if c.is_ascii_alphabetic() => KeyCode::Char(c.to_ascii_lowercase()),
        other => other,
    };
    Chord::new(code, mods)
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn binding_command_populates_bound_paths_cache() {
        let mut k = Keymap::new();
        let chord = Chord::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        k.bind_command(chord, CommandId::builtin("editor.save"));
        let p = Path::parse("/keymap/ctrl-s").unwrap();
        let dest = k.lookup(&p).unwrap();
        assert_eq!(dest.as_str(), "/cmd/editor.save");
    }

    #[test]
    fn binding_action_does_not_populate_paths() {
        let mut k = Keymap::new();
        let chord = Chord::new(KeyCode::Char('h'), KeyModifiers::NONE);
        k.bind_action(chord, Arc::new(cmd::Quit));
        let paths: Vec<Path> = k.paths().collect();
        assert!(paths.is_empty());
    }

    #[test]
    fn paths_enumerates_canonical_kebab_chords() {
        let mut k = Keymap::new();
        k.bind_command(
            Chord::new(KeyCode::Char('p'), KeyModifiers::CONTROL | KeyModifiers::SHIFT),
            CommandId::builtin("palette.open"),
        );
        let paths: Vec<String> = k.paths().map(|p| p.as_str().to_string()).collect();
        assert_eq!(paths, vec!["/keymap/ctrl-shift-p"]);
    }
}
