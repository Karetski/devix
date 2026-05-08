//! Built-in command definitions. Registered into `CommandRegistry` at app
//! startup. Only commands that make sense as discoverable, palette-listable
//! entries belong here — motion, character insertion, mouse, and other
//! continuously-fired actions are dispatched directly via the keymap and
//! never appear in the palette.

use crate::editor::commands::cmd;
use crate::editor::commands::registry::{CommandId, CommandRegistry};

pub const PALETTE_OPEN:        CommandId = CommandId("palette.open");
pub const PALETTE_CLOSE:       CommandId = CommandId("palette.close");
pub const PALETTE_MOVE_DOWN:   CommandId = CommandId("palette.move_down");
pub const PALETTE_MOVE_UP:     CommandId = CommandId("palette.move_up");
pub const PALETTE_ACCEPT:      CommandId = CommandId("palette.accept");

pub const FILE_SAVE:           CommandId = CommandId("file.save");
pub const FILE_RELOAD:         CommandId = CommandId("file.reload");
pub const FILE_KEEP_BUFFER:    CommandId = CommandId("file.keep_buffer");

pub const EDIT_UNDO:           CommandId = CommandId("edit.undo");
pub const EDIT_REDO:           CommandId = CommandId("edit.redo");
pub const EDIT_SELECT_ALL:     CommandId = CommandId("edit.select_all");
pub const EDIT_COPY:           CommandId = CommandId("edit.copy");
pub const EDIT_CUT:            CommandId = CommandId("edit.cut");
pub const EDIT_PASTE:          CommandId = CommandId("edit.paste");
pub const EDIT_ADD_CURSOR_ABOVE:  CommandId = CommandId("edit.add_cursor_above");
pub const EDIT_ADD_CURSOR_BELOW:  CommandId = CommandId("edit.add_cursor_below");
pub const EDIT_COLLAPSE_SELECTION: CommandId = CommandId("edit.collapse_selection");

pub const TAB_NEW:             CommandId = CommandId("tab.new");
pub const TAB_CLOSE:           CommandId = CommandId("tab.close");
pub const TAB_FORCE_CLOSE:     CommandId = CommandId("tab.force_close");
pub const TAB_NEXT:            CommandId = CommandId("tab.next");
pub const TAB_PREV:            CommandId = CommandId("tab.prev");

pub const SPLIT_VERTICAL:      CommandId = CommandId("split.vertical");
pub const SPLIT_HORIZONTAL:    CommandId = CommandId("split.horizontal");
pub const SPLIT_CLOSE:         CommandId = CommandId("split.close");

pub const SIDEBAR_LEFT:        CommandId = CommandId("sidebar.toggle_left");
pub const SIDEBAR_RIGHT:       CommandId = CommandId("sidebar.toggle_right");

pub const APP_QUIT:            CommandId = CommandId("app.quit");

/// Register every built-in command from the embedded
/// `BUILTIN_MANIFEST`. T-74 retired the old hand-maintained
/// `register_builtins` table — the manifest at
/// `crates/devix-core/manifests/builtin.json` is the single source
/// of truth; `cmd::handler_for_builtin_id` resolves each id to its
/// Rust handler.
pub fn register_builtins(reg: &mut CommandRegistry) {
    use crate::manifest_loader::{parse_manifest_bytes, register_command_contributions};
    let manifest = parse_manifest_bytes(
        crate::BUILTIN_MANIFEST.as_bytes(),
        std::path::Path::new("crates/devix-core/manifests/builtin.json"),
    )
    .expect("BUILTIN_MANIFEST always parses; tested in builtin_manifest::builtin_manifest_validates");
    register_command_contributions(reg, &manifest, cmd::handler_for_builtin_id)
        .expect("BUILTIN_MANIFEST has handlers for every id");
}

pub fn build_registry() -> CommandRegistry {
    let mut reg = CommandRegistry::new();
    register_builtins(&mut reg);
    reg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_all_builtins() {
        let reg = build_registry();
        assert!(reg.get(FILE_SAVE).is_some());
        assert!(reg.get(PALETTE_OPEN).is_some());
        assert!(reg.get(SIDEBAR_RIGHT).is_some());
        assert!(reg.len() >= 25);
    }
}
