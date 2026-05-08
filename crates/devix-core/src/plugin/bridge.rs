//! Editor-side action wrapper for plugin commands.
//!
//! `LuaAction` makes a plugin-contributed command storable in the
//! regular `CommandRegistry` and dispatchable through `Keymap`. The
//! action body sends a handle through the editor's [`InvokeSender`];
//! the actual Lua work runs on the supervised plugin thread.

use std::sync::Arc;

use crate::Action;
use crate::editor::commands::context::Context;
use crate::editor::commands::cmd::EditorCommand;

use super::{CommandSpec, InvokeSender, send_invoke};

/// Action wrapper: a plugin-contributed command stored in the editor's
/// regular `CommandRegistry`. Invoking it sends a handle to the plugin
/// thread; the actual Lua work runs there.
pub struct LuaAction {
    handle: u64,
    sender: InvokeSender,
}

impl LuaAction {
    pub fn new(handle: u64, sender: InvokeSender) -> Self {
        Self { handle, sender }
    }
}

impl<'a> Action<Context<'a>> for LuaAction {
    fn invoke(&self, _ctx: &mut Context<'a>) {
        let _ = send_invoke(&self.sender, self.handle);
    }
}

/// Storage-typed alias so palette / keymap call sites can hold these
/// behind `Arc<dyn EditorCommand>` like every other command.
pub type PluginCommandAction = Arc<dyn EditorCommand>;

pub fn make_command_action(spec: &CommandSpec, sender: InvokeSender) -> PluginCommandAction {
    Arc::new(LuaAction::new(spec.handle, sender))
}
