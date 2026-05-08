use devix_core::editor::commands::cmd::handler_for_builtin_id;
use devix_core::editor::commands::registry::CommandRegistry;
use devix_core::manifest_loader::{parse_manifest_bytes, register_command_contributions};
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
