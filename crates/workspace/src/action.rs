//! Closed enum of every editor command. The dispatcher's only input.

use std::path::PathBuf;

use crate::layout::Direction;
use crate::layout::SidebarSlot;

#[derive(Clone, Debug)]
pub enum Action {
    // motion
    MoveLeft { extend: bool },
    MoveRight { extend: bool },
    MoveUp { extend: bool },
    MoveDown { extend: bool },
    MoveWordLeft { extend: bool },
    MoveWordRight { extend: bool },
    MoveLineStart { extend: bool },
    MoveLineEnd { extend: bool },
    MoveDocStart { extend: bool },
    MoveDocEnd { extend: bool },
    PageUp { extend: bool },
    PageDown { extend: bool },

    // edits
    InsertChar(char),
    InsertNewline,
    InsertTab,
    DeleteBack { word: bool },
    DeleteForward { word: bool },

    // history
    Undo,
    Redo,

    // selection
    SelectAll,

    // clipboard
    Copy,
    Cut,
    Paste,

    // file / disk
    Save,
    ReloadFromDisk,
    KeepBufferIgnoreDisk,

    // tabs
    NewTab,
    CloseTab,
    ForceCloseTab,
    NextTab,
    PrevTab,
    OpenPath(PathBuf),

    // splits / frames
    SplitVertical,
    SplitHorizontal,
    CloseFrame,
    ToggleSidebar(SidebarSlot),
    FocusDir(Direction),

    // app
    Quit,

    // command palette overlay
    OpenPalette,
    ClosePalette,
    PaletteMove(isize),
    PaletteSetQuery(String),
    PaletteAccept,

    // language server
    Hover,
    GotoDefinition,
    /// Manual / trigger-character invocation of completion. Sends a
    /// `LspCommand::Completion` and parks `CompletionState::Pending` on
    /// the active view.
    TriggerCompletion,
    /// Adjust the highlighted completion item by `delta`. No-op when the
    /// popup is closed.
    CompletionMove(isize),
    /// Accept the highlighted completion item: apply its text edit (or
    /// fallback ident replacement), then close the popup.
    CompletionAccept,
    /// Dismiss the completion popup without inserting anything.
    CompletionDismiss,

    // mouse
    ClickAt { col: u16, row: u16, extend: bool },
    DragAt { col: u16, row: u16 },
    /// Move the viewport by `delta` lines (negative = up, positive = down).
    /// Scroll events from the input layer are coalesced into a single
    /// `ScrollBy` per drain so a 200-event inertia burst becomes one dispatch.
    ScrollBy(isize),
}
