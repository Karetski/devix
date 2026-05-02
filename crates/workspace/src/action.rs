//! Closed enum of every editor command. The dispatcher's only input.

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

    // app
    Quit,

    // mouse
    ClickAt { col: u16, row: u16, extend: bool },
    DragAt { col: u16, row: u16 },
    ScrollUp,
    ScrollDown,
}
