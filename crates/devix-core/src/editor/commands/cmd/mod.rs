//! Editor commands: discrete `Action` impls grouped by concern.
//!
//! Each submodule owns one cohesive group of commands. The shared trait
//! alias and modal-helper functions live here at the root.

use crate::Action;

use crate::editor::commands::context::Context;

pub mod clipboard;
pub mod edit;
pub mod file;
pub mod motion;
pub mod mouse;
pub mod palette;
pub mod split;
pub mod tab;

pub use clipboard::{Copy, Cut, Paste};
pub use edit::{
    AddCursorAbove, AddCursorBelow, CollapseSelection, DeleteBack, DeleteForward, InsertChar,
    InsertNewline, InsertTab, Redo, SelectAll, Undo,
};
pub use file::{KeepBufferIgnoreDisk, OpenPath, Quit, ReloadFromDisk, Save};
pub use motion::{
    MoveDocEnd, MoveDocStart, MoveDown, MoveLeft, MoveLineEnd, MoveLineStart, MoveRight, MoveUp,
    MoveWordLeft, MoveWordRight, PageDown, PageUp,
};
pub use mouse::{ClickAt, DragAt, ScrollBy};
pub use palette::{ClosePalette, CloseModal, OpenPalette, PaletteAccept, PaletteMove, PaletteSetQuery};
pub use split::{CloseFrame, FocusDir, SplitHorizontal, SplitVertical, ToggleSidebar};
pub use tab::{CloseTab, ForceCloseTab, NewTab, NextTab, PrevTab};

/// HRTB trait alias for actions that take the editor's `Context<'_>`.
/// Storage shape: `Box<dyn EditorCommand>` / `Arc<dyn EditorCommand>`.
pub trait EditorCommand: for<'a> Action<Context<'a>> {}
impl<T> EditorCommand for T where T: for<'a> Action<Context<'a>> {}

/// Resolve a built-in command id to its Rust handler. Returns `None`
/// for unknown ids — callers (manifest loader at T-71) treat the
/// missing-handler case as a load-time error so a built-in manifest
/// entry that no longer has a handler doesn't silently disappear from
/// the registry.
pub fn handler_for_builtin_id(id: &str) -> Option<std::sync::Arc<dyn EditorCommand>> {
    use std::sync::Arc;
    use crate::SidebarSlot;
    let arc: Arc<dyn EditorCommand> = match id {
        "palette.open" => Arc::new(palette::OpenPalette),
        "palette.close" => Arc::new(palette::ClosePalette),
        "palette.move_down" => Arc::new(palette::PaletteMove(1)),
        "palette.move_up" => Arc::new(palette::PaletteMove(-1)),
        "palette.accept" => Arc::new(palette::PaletteAccept),

        "file.save" => Arc::new(file::Save),
        "file.reload" => Arc::new(file::ReloadFromDisk),
        "file.keep_buffer" => Arc::new(file::KeepBufferIgnoreDisk),

        "edit.undo" => Arc::new(edit::Undo),
        "edit.redo" => Arc::new(edit::Redo),
        "edit.select_all" => Arc::new(edit::SelectAll),
        "edit.copy" => Arc::new(clipboard::Copy),
        "edit.cut" => Arc::new(clipboard::Cut),
        "edit.paste" => Arc::new(clipboard::Paste),
        "edit.add_cursor_above" => Arc::new(edit::AddCursorAbove),
        "edit.add_cursor_below" => Arc::new(edit::AddCursorBelow),
        "edit.collapse_selection" => Arc::new(edit::CollapseSelection),

        "tab.new" => Arc::new(tab::NewTab),
        "tab.close" => Arc::new(tab::CloseTab),
        "tab.force_close" => Arc::new(tab::ForceCloseTab),
        "tab.next" => Arc::new(tab::NextTab),
        "tab.prev" => Arc::new(tab::PrevTab),

        "split.vertical" => Arc::new(split::SplitVertical),
        "split.horizontal" => Arc::new(split::SplitHorizontal),
        "split.close" => Arc::new(split::CloseFrame),

        "sidebar.toggle_left" => Arc::new(split::ToggleSidebar(SidebarSlot::Left)),
        "sidebar.toggle_right" => Arc::new(split::ToggleSidebar(SidebarSlot::Right)),

        "app.quit" => Arc::new(file::Quit),

        _ => return None,
    };
    Some(arc)
}

pub(crate) fn modal_is<T: 'static>(ctx: &Context<'_>) -> bool {
    ctx.editor
        .modal
        .as_ref()
        .and_then(|m| m.as_any())
        .map(|a| a.is::<T>())
        .unwrap_or(false)
}

pub(crate) fn downcast_modal_mut<'a, T: 'static>(ctx: &'a mut Context<'_>) -> Option<&'a mut T> {
    ctx.editor
        .modal
        .as_mut()?
        .as_any_mut()?
        .downcast_mut::<T>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::commands::context::Viewport;
    use crate::editor::commands::registry::CommandRegistry;
    use crate::editor::editor::RenderCache;
    use crate::editor::Editor;

    fn make_ctx<'a>(
        ws: &'a mut Editor,
        clipboard: &'a mut dyn crate::Clipboard,
        quit: &'a mut bool,
        commands: &'a CommandRegistry,
        layout_cache: &'a RenderCache,
    ) -> Context<'a> {
        Context {
            editor: ws,
            clipboard,
            quit,
            viewport: Viewport::default(),
            commands,
            layout_cache,
        }
    }

    #[test]
    fn quit_sets_the_quit_flag_through_the_trait() {
        let mut ws = Editor::open(None).unwrap();
        let mut clipboard = crate::NoClipboard;
        let mut quit = false;
        let commands = CommandRegistry::default();
        let cache = RenderCache::default();
        let mut ctx = make_ctx(&mut ws, &mut clipboard, &mut quit, &commands, &cache);
        Quit.invoke(&mut ctx);
        assert!(quit, "Quit action should set the quit flag");
    }

    #[test]
    fn quit_can_be_stored_as_box_dyn_editor_command() {
        let _: Box<dyn EditorCommand> = Box::new(Quit);
    }

    #[test]
    fn parameterized_commands_dispatch_through_trait_objects() {
        let mut ws = Editor::open(None).unwrap();
        let mut clipboard = crate::NoClipboard;
        let mut quit = false;
        let commands = CommandRegistry::default();
        let cache = RenderCache::default();

        let actions: Vec<Box<dyn EditorCommand>> =
            vec![Box::new(NewTab), Box::new(NextTab), Box::new(NewTab)];

        for action in &actions {
            let mut ctx = make_ctx(&mut ws, &mut clipboard, &mut quit, &commands, &cache);
            action.invoke(&mut ctx);
        }
        let fid = ws.active_frame().unwrap();
        let frame = ws.panes.find_frame(fid).unwrap();
        assert_eq!(frame.tabs.len(), 3);
    }

    #[test]
    fn open_palette_populates_modal_slot() {
        let mut ws = Editor::open(None).unwrap();
        let mut clipboard = crate::NoClipboard;
        let mut quit = false;
        let commands = CommandRegistry::default();
        let cache = RenderCache::default();
        let mut ctx = make_ctx(&mut ws, &mut clipboard, &mut quit, &commands, &cache);
        OpenPalette.invoke(&mut ctx);
        assert!(ws.modal.is_some());
        assert!(
            ws.modal
                .as_ref()
                .and_then(|m| m.as_any())
                .map(|a| a.is::<crate::editor::commands::modal::PalettePane>())
                .unwrap_or(false),
            "modal slot should hold a PalettePane",
        );
    }

    fn surface_with_text(text: &str) -> Editor {
        use devix_text::{Selection, replace_selection_tx};
        let mut ws = Editor::open(None).unwrap();
        let did = ws.active_cursor().unwrap().doc;
        let tx = replace_selection_tx(&ws.documents[did].buffer, &Selection::point(0), text);
        ws.documents[did].buffer.apply(tx);
        let cid = ws.active_ids().unwrap().1;
        ws.cursors[cid].selection = Selection::point(0);
        ws
    }

    #[test]
    fn add_cursor_below_inserts_at_each_cursor() {
        let mut ws = surface_with_text("aa\nbb\ncc");
        let mut clipboard = crate::NoClipboard;
        let mut quit = false;
        let commands = CommandRegistry::default();
        let cache = RenderCache::default();
        let mut ctx = make_ctx(&mut ws, &mut clipboard, &mut quit, &commands, &cache);
        AddCursorBelow.invoke(&mut ctx);
        AddCursorBelow.invoke(&mut ctx);
        InsertChar('x').invoke(&mut ctx);
        let did = ws.active_cursor().unwrap().doc;
        assert_eq!(ws.documents[did].buffer.rope().to_string(), "xaa\nxbb\nxcc");
        let cid = ws.active_ids().unwrap().1;
        let sel = &ws.cursors[cid].selection;
        assert_eq!(sel.len(), 3);
        for r in sel.ranges() {
            let buf = &ws.documents[did].buffer;
            assert_eq!(buf.col_of_char(r.head), 1);
        }
    }

    #[test]
    fn add_cursor_above_at_line_zero_is_noop() {
        let mut ws = surface_with_text("aa\nbb");
        let mut clipboard = crate::NoClipboard;
        let mut quit = false;
        let commands = CommandRegistry::default();
        let cache = RenderCache::default();
        let mut ctx = make_ctx(&mut ws, &mut clipboard, &mut quit, &commands, &cache);
        AddCursorAbove.invoke(&mut ctx);
        let cid = ws.active_ids().unwrap().1;
        assert_eq!(ws.cursors[cid].selection.len(), 1);
    }

    #[test]
    fn motion_transforms_every_cursor() {
        let mut ws = surface_with_text("aaaa\nbbbb");
        let mut clipboard = crate::NoClipboard;
        let mut quit = false;
        let commands = CommandRegistry::default();
        let cache = RenderCache::default();
        let mut ctx = make_ctx(&mut ws, &mut clipboard, &mut quit, &commands, &cache);
        AddCursorBelow.invoke(&mut ctx);
        MoveRight { extend: false }.invoke(&mut ctx);
        MoveRight { extend: false }.invoke(&mut ctx);
        let cid = ws.active_ids().unwrap().1;
        let did = ws.cursors[cid].doc;
        let buf = &ws.documents[did].buffer;
        let cols: Vec<usize> = ws.cursors[cid]
            .selection
            .ranges()
            .iter()
            .map(|r| buf.col_of_char(r.head))
            .collect();
        assert_eq!(cols, vec![2, 2]);
    }

    #[test]
    fn delete_back_removes_one_char_per_cursor() {
        let mut ws = surface_with_text("aa\nbb");
        let cid0 = ws.active_ids().unwrap().1;
        ws.cursors[cid0].selection = devix_text::Selection::with_ranges(
            vec![devix_text::Range::point(2), devix_text::Range::point(5)],
            0,
        );
        let mut clipboard = crate::NoClipboard;
        let mut quit = false;
        let commands = CommandRegistry::default();
        let cache = RenderCache::default();
        let mut ctx = make_ctx(&mut ws, &mut clipboard, &mut quit, &commands, &cache);
        DeleteBack { word: false }.invoke(&mut ctx);
        let did = ws.active_cursor().unwrap().doc;
        assert_eq!(ws.documents[did].buffer.rope().to_string(), "a\nb");
    }

    #[test]
    fn collapse_selection_drops_secondary_cursors() {
        let mut ws = surface_with_text("aa\nbb\ncc");
        let mut clipboard = crate::NoClipboard;
        let mut quit = false;
        let commands = CommandRegistry::default();
        let cache = RenderCache::default();
        let mut ctx = make_ctx(&mut ws, &mut clipboard, &mut quit, &commands, &cache);
        AddCursorBelow.invoke(&mut ctx);
        AddCursorBelow.invoke(&mut ctx);
        CollapseSelection.invoke(&mut ctx);
        let cid = ws.active_ids().unwrap().1;
        assert_eq!(ws.cursors[cid].selection.len(), 1);
    }

    #[test]
    fn close_modal_clears_any_modal() {
        let mut ws = Editor::open(None).unwrap();
        ws.open_modal(
            Box::new(crate::editor::commands::modal::PalettePane::from_registry(
                &CommandRegistry::default(),
            )),
            devix_protocol::pulse::ModalKind::Palette,
        );
        let mut clipboard = crate::NoClipboard;
        let mut quit = false;
        let commands = CommandRegistry::default();
        let cache = RenderCache::default();
        let mut ctx = make_ctx(&mut ws, &mut clipboard, &mut quit, &commands, &cache);
        CloseModal.invoke(&mut ctx);
        assert!(ws.modal.is_none());
    }
}
