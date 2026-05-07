//! Plugin sidebar renders into the editor pane tree.
//!
//! Boots an `Application<TestBackend>` with the bundled `file_tree.lua`
//! example installed via `PluginRuntime::install` (which registers
//! commands, binds chords, and installs sidebar panes onto the editor's
//! structural tree). Asserts that the rendered cells include a
//! recognizable file-tree marker.

use std::path::Path;
use std::sync::Mutex;

use devix_tui::Application;
use devix_core::{Editor, build_registry, default_keymap};
use devix_core::{NoClipboard, Theme};
use devix_core::PluginRuntime;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    key: &'static str,
    prev: Option<std::ffi::OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl EnvGuard {
    fn set(key: &'static str, val: &str) -> Self {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, val);
        }
        Self { key, prev, _lock }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            match self.prev.take() {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

#[test]
fn sidebar_renders_plugin_supplied_lines() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let example = manifest
        .parent()
        .unwrap()
        .join("devix-core/examples/file_tree.lua");
    let _g = EnvGuard::set("DEVIX_PLUGIN", &example.to_string_lossy());

    let mut editor = Editor::open(None).expect("editor opens");
    let mut commands = build_registry();
    let mut keymap = default_keymap();

    let mut runtime = PluginRuntime::load(&example).expect("plugin loads");
    runtime.install(&mut commands, &mut keymap, &mut editor);

    let mut app = Application::for_test(
        editor,
        commands,
        keymap,
        Theme::default(),
        Box::new(NoClipboard),
        (40, 10),
    );

    app.force_render();
    let buf = app.buffer();
    let mut all = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            all.push_str(buf[(x, y)].symbol());
        }
        all.push('\n');
    }
    assert!(
        all.contains('\u{25b8}') || all.contains("Cargo"),
        "expected file-tree content (▸ marker or Cargo entry) somewhere in:\n{all}",
    );

    // PluginRuntime drops here — its worker thread exits when channels close.
    drop(runtime);
}
