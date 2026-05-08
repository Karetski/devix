use devix_core::editor::commands::cmd::handler_for_builtin_id;
use devix_core::editor::commands::keymap::Keymap;
use devix_core::editor::commands::registry::CommandRegistry;
use devix_core::manifest_loader::{
    parse_manifest_bytes, register_command_contributions, register_keymap_contributions,
};
use std::path::Path;

fn builtin_manifest() -> devix_protocol::manifest::Manifest {
    parse_manifest_bytes(
        devix_core::BUILTIN_MANIFEST.as_bytes(),
        Path::new("crates/devix-core/manifests/builtin.json"),
    )
    .expect("builtin.json must parse + validate")
}

#[test]
fn builtin_manifest_validates() {
    let m = builtin_manifest();
    assert_eq!(m.name, "devix-builtin");
    assert!(!m.contributes.commands.is_empty());
    assert!(!m.contributes.keymaps.is_empty());
    assert_eq!(m.contributes.themes.len(), 1);
}

#[test]
fn every_manifest_command_id_has_a_rust_handler() {
    let m = builtin_manifest();
    for spec in &m.contributes.commands {
        assert!(
            handler_for_builtin_id(&spec.id).is_some(),
            "no Rust handler for command id `{}`",
            spec.id,
        );
    }
}

#[test]
fn register_keymap_contributions_binds_every_entry() {
    let m = builtin_manifest();
    let mut reg = CommandRegistry::new();
    register_command_contributions(&mut reg, &m, handler_for_builtin_id).unwrap();
    let mut km = Keymap::new();
    let n = register_keymap_contributions(&mut km, &m, &reg).unwrap();
    assert_eq!(n, m.contributes.keymaps.len());
    // Spot-check chords whose characters fit the path segment
    // grammar resolve via Lookup. Chords whose key uses reserved
    // characters (e.g., `\`) bind successfully but don't appear in
    // the Path-keyed cache; that's a known gap in T-54's
    // bound_paths shape and orthogonal to T-72's task.
    use devix_protocol::Lookup;
    let chord_path = devix_protocol::path::Path::parse("/keymap/ctrl-c").unwrap();
    let dest = km.lookup(&chord_path).unwrap();
    assert_eq!(dest.as_str(), "/cmd/edit.copy");
    let chord_path = devix_protocol::path::Path::parse("/keymap/ctrl-p").unwrap();
    let dest = km.lookup(&chord_path).unwrap();
    assert_eq!(dest.as_str(), "/cmd/palette.open");
    let chord_path = devix_protocol::path::Path::parse("/keymap/ctrl-shift-z").unwrap();
    let dest = km.lookup(&chord_path).unwrap();
    assert_eq!(dest.as_str(), "/cmd/edit.redo");
}

#[test]
fn register_command_contributions_loads_every_id() {
    let m = builtin_manifest();
    let mut reg = CommandRegistry::new();
    let n = register_command_contributions(&mut reg, &m, handler_for_builtin_id)
        .expect("registration should succeed");
    assert_eq!(n, m.contributes.commands.len());
    for spec in &m.contributes.commands {
        let path = devix_protocol::path::Path::parse(&format!("/cmd/{}", spec.id)).unwrap();
        assert!(
            <CommandRegistry as devix_protocol::Lookup>::lookup(&reg, &path).is_some(),
            "id `{}` registered but not findable via Lookup",
            spec.id,
        );
    }
}
