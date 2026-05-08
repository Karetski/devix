//! Emit the manifest JSON Schema to stdout.
//!
//! Used to regenerate `crates/devix-core/manifests/manifest.schema.json`:
//!
//! ```bash
//! cargo run -p devix-protocol --example dump_manifest_schema \
//!   > crates/devix-core/manifests/manifest.schema.json
//! ```

fn main() {
    let schema = devix_protocol::manifest_json_schema();
    let pretty = serde_json::to_string_pretty(&schema)
        .expect("JSON Schema serializes as JSON");
    println!("{pretty}");
}
