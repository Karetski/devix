//! Built-in command definitions. Registered into `CommandRegistry` at app
//! startup. Only commands that make sense as discoverable, palette-listable
//! entries belong here — motion, character insertion, mouse, and other
//! continuously-fired actions are dispatched directly via the keymap and
//! never appear in the palette.

use devix_workspace::{Action, Command, CommandId, CommandRegistry, SidebarSlot};

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
    let r = |reg: &mut CommandRegistry, id, label, category, action| {
        reg.register(Command { id, label, category: Some(category), action });
    };

    r(reg, PALETTE_OPEN,      "Open Command Palette",     "Palette", Action::OpenPalette);
    r(reg, PALETTE_CLOSE,     "Close Command Palette",    "Palette", Action::ClosePalette);
    r(reg, PALETTE_MOVE_DOWN, "Palette: Next Match",      "Palette", Action::PaletteMove(1));
    r(reg, PALETTE_MOVE_UP,   "Palette: Previous Match",  "Palette", Action::PaletteMove(-1));
    r(reg, PALETTE_ACCEPT,    "Palette: Accept Selection","Palette", Action::PaletteAccept);

    r(reg, FILE_SAVE,         "Save File",                "File",    Action::Save);
    r(reg, FILE_RELOAD,       "Reload from Disk",         "File",    Action::ReloadFromDisk);
    r(reg, FILE_KEEP_BUFFER,  "Keep Buffer (Ignore Disk Change)", "File", Action::KeepBufferIgnoreDisk);

    r(reg, EDIT_UNDO,         "Undo",                     "Edit",    Action::Undo);
    r(reg, EDIT_REDO,         "Redo",                     "Edit",    Action::Redo);
    r(reg, EDIT_SELECT_ALL,   "Select All",               "Edit",    Action::SelectAll);
    r(reg, EDIT_COPY,         "Copy",                     "Edit",    Action::Copy);
    r(reg, EDIT_CUT,          "Cut",                      "Edit",    Action::Cut);
    r(reg, EDIT_PASTE,        "Paste",                    "Edit",    Action::Paste);

    r(reg, TAB_NEW,           "New Tab",                  "Tab",     Action::NewTab);
    r(reg, TAB_CLOSE,         "Close Tab",                "Tab",     Action::CloseTab);
    r(reg, TAB_FORCE_CLOSE,   "Close Tab (Discard Changes)", "Tab",  Action::ForceCloseTab);
    r(reg, TAB_NEXT,          "Next Tab",                 "Tab",     Action::NextTab);
    r(reg, TAB_PREV,          "Previous Tab",             "Tab",     Action::PrevTab);

    r(reg, SPLIT_VERTICAL,    "Split Vertical",           "Split",   Action::SplitVertical);
    r(reg, SPLIT_HORIZONTAL,  "Split Horizontal",         "Split",   Action::SplitHorizontal);
    r(reg, SPLIT_CLOSE,       "Close Split",              "Split",   Action::CloseFrame);

    r(reg, SIDEBAR_LEFT,      "Toggle Left Sidebar",      "View",    Action::ToggleSidebar(SidebarSlot::Left));
    r(reg, SIDEBAR_RIGHT,     "Toggle Right Sidebar",     "View",    Action::ToggleSidebar(SidebarSlot::Right));

    r(reg, APP_QUIT,          "Quit",                     "App",     Action::Quit);
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
        // Sanity: no duplicate-id collisions silently shrunk the registry.
        assert!(reg.len() >= 25);
    }
}
