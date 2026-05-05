//! Built-in command definitions. Registered into `CommandRegistry` at app
//! startup. Only commands that make sense as discoverable, palette-listable
//! entries belong here — motion, character insertion, mouse, and other
//! continuously-fired actions are dispatched directly via the keymap and
//! never appear in the palette.

use std::sync::Arc;

use crate::cmd::{self, EditorCommand};
use crate::registry::{Command, CommandId, CommandRegistry};
use devix_core::layout::SidebarSlot;

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

pub fn register_builtins(reg: &mut CommandRegistry) {
    let r = |reg: &mut CommandRegistry,
             id,
             label,
             category,
             action: Arc<dyn EditorCommand>| {
        reg.register(Command { id, label, category: Some(category), action });
    };

    r(reg, PALETTE_OPEN,      "Open Command Palette",     "Palette", Arc::new(cmd::OpenPalette));
    r(reg, PALETTE_CLOSE,     "Close Command Palette",    "Palette", Arc::new(cmd::ClosePalette));
    r(reg, PALETTE_MOVE_DOWN, "Palette: Next Match",      "Palette", Arc::new(cmd::PaletteMove(1)));
    r(reg, PALETTE_MOVE_UP,   "Palette: Previous Match",  "Palette", Arc::new(cmd::PaletteMove(-1)));
    r(reg, PALETTE_ACCEPT,    "Palette: Accept Selection","Palette", Arc::new(cmd::PaletteAccept));

    r(reg, FILE_SAVE,         "Save File",                "File",    Arc::new(cmd::Save));
    r(reg, FILE_RELOAD,       "Reload from Disk",         "File",    Arc::new(cmd::ReloadFromDisk));
    r(reg, FILE_KEEP_BUFFER,  "Keep Buffer (Ignore Disk Change)", "File", Arc::new(cmd::KeepBufferIgnoreDisk));

    r(reg, EDIT_UNDO,         "Undo",                     "Edit",    Arc::new(cmd::Undo));
    r(reg, EDIT_REDO,         "Redo",                     "Edit",    Arc::new(cmd::Redo));
    r(reg, EDIT_SELECT_ALL,   "Select All",               "Edit",    Arc::new(cmd::SelectAll));
    r(reg, EDIT_COPY,         "Copy",                     "Edit",    Arc::new(cmd::Copy));
    r(reg, EDIT_CUT,          "Cut",                      "Edit",    Arc::new(cmd::Cut));
    r(reg, EDIT_PASTE,        "Paste",                    "Edit",    Arc::new(cmd::Paste));
    r(reg, EDIT_ADD_CURSOR_ABOVE,    "Add Cursor Above",   "Edit",    Arc::new(cmd::AddCursorAbove));
    r(reg, EDIT_ADD_CURSOR_BELOW,    "Add Cursor Below",   "Edit",    Arc::new(cmd::AddCursorBelow));
    r(reg, EDIT_COLLAPSE_SELECTION,  "Collapse Selection", "Edit",    Arc::new(cmd::CollapseSelection));

    r(reg, TAB_NEW,           "New Tab",                  "Tab",     Arc::new(cmd::NewTab));
    r(reg, TAB_CLOSE,         "Close Tab",                "Tab",     Arc::new(cmd::CloseTab));
    r(reg, TAB_FORCE_CLOSE,   "Close Tab (Discard Changes)", "Tab",  Arc::new(cmd::ForceCloseTab));
    r(reg, TAB_NEXT,          "Next Tab",                 "Tab",     Arc::new(cmd::NextTab));
    r(reg, TAB_PREV,          "Previous Tab",             "Tab",     Arc::new(cmd::PrevTab));

    r(reg, SPLIT_VERTICAL,    "Split Vertical",           "Split",   Arc::new(cmd::SplitVertical));
    r(reg, SPLIT_HORIZONTAL,  "Split Horizontal",         "Split",   Arc::new(cmd::SplitHorizontal));
    r(reg, SPLIT_CLOSE,       "Close Split",              "Split",   Arc::new(cmd::CloseFrame));

    r(reg, SIDEBAR_LEFT,      "Toggle Left Sidebar",      "View",    Arc::new(cmd::ToggleSidebar(SidebarSlot::Left)));
    r(reg, SIDEBAR_RIGHT,     "Toggle Right Sidebar",     "View",    Arc::new(cmd::ToggleSidebar(SidebarSlot::Right)));

    r(reg, APP_QUIT,          "Quit",                     "App",     Arc::new(cmd::Quit));
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
