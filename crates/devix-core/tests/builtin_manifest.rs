use devix_core::manifest_loader::parse_manifest_bytes;
use std::path::Path;

#[test]
fn builtin_manifest_validates() {
    let m = parse_manifest_bytes(
        devix_core::BUILTIN_MANIFEST.as_bytes(),
        Path::new("crates/devix-core/manifests/builtin.json"),
    )
    .expect("builtin.json must parse + validate");
    assert_eq!(m.name, "devix-builtin");
    assert!(!m.contributes.commands.is_empty());
    assert!(!m.contributes.keymaps.is_empty());
    assert_eq!(m.contributes.themes.len(), 1);
}
