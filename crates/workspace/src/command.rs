//! Command registry: a discoverable, id-keyed surface over [`Action`].
//!
//! `Action` is the dispatcher's typed input — closed enum, exhaustive match.
//! `CommandId` is the *identity* used by keymap, palette, and (later) plugins.
//! The registry maps id → `Command { label, category, action }`.
//!
//! Three layers, no locks: the registry is built once at app startup and
//! read-only thereafter. When plugin contributions land we can wrap it in
//! `Arc<RwLock<...>>` without touching call sites that read through the API.

use std::collections::HashMap;

use crate::action::Action;

#[derive(Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct CommandId(pub &'static str);

#[derive(Clone, Debug)]
pub struct Command {
    pub id: CommandId,
    pub label: &'static str,
    pub category: Option<&'static str>,
    pub action: Action,
}

#[derive(Default)]
pub struct CommandRegistry {
    by_id: HashMap<CommandId, Command>,
    order: Vec<CommandId>,
}

impl CommandRegistry {
    pub fn new() -> Self { Self::default() }

    /// Register a command. Re-registering the same id replaces the previous
    /// entry but preserves its insertion position in `order`.
    pub fn register(&mut self, cmd: Command) {
        if !self.by_id.contains_key(&cmd.id) {
            self.order.push(cmd.id);
        }
        self.by_id.insert(cmd.id, cmd);
    }

    pub fn get(&self, id: CommandId) -> Option<&Command> {
        self.by_id.get(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Command> {
        self.order.iter().filter_map(|id| self.by_id.get(id))
    }

    pub fn len(&self) -> usize { self.order.len() }
    pub fn is_empty(&self) -> bool { self.order.is_empty() }

    /// Resolve an id to its action. Returns `None` for unknown ids — callers
    /// should treat this as a no-op, not a panic, since plugins or stale
    /// keymaps may refer to ids that no longer exist.
    pub fn resolve(&self, id: CommandId) -> Option<Action> {
        self.by_id.get(&id).map(|c| c.action.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;

    #[test]
    fn register_and_resolve() {
        let mut reg = CommandRegistry::new();
        reg.register(Command {
            id: CommandId("editor.save"),
            label: "Save",
            category: Some("File"),
            action: Action::Save,
        });
        assert_eq!(reg.len(), 1);
        assert!(matches!(reg.resolve(CommandId("editor.save")), Some(Action::Save)));
        assert!(reg.resolve(CommandId("missing")).is_none());
    }

    #[test]
    fn re_register_preserves_order() {
        let mut reg = CommandRegistry::new();
        reg.register(Command {
            id: CommandId("a"), label: "A", category: None, action: Action::Quit,
        });
        reg.register(Command {
            id: CommandId("b"), label: "B", category: None, action: Action::Quit,
        });
        reg.register(Command {
            id: CommandId("a"), label: "A2", category: None, action: Action::Quit,
        });
        let labels: Vec<&str> = reg.iter().map(|c| c.label).collect();
        assert_eq!(labels, vec!["A2", "B"]);
    }
}
