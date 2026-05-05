//! Command registry: a discoverable, id-keyed surface over the editor's
//! action trait.
//!
//! Phase 5: actions are values implementing `EditorCommand`. The registry
//! owns one `Box<dyn EditorCommand>` per id; `resolve(id)` hands back a
//! borrow so callers (palette, dispatcher) can invoke without taking
//! ownership.
//!
//! Three layers, no locks: the registry is built once at app startup and
//! read-only thereafter. When plugin contributions land we can wrap it in
//! `Arc<RwLock<...>>` without touching call sites that read through the API.

use std::collections::HashMap;
use std::sync::Arc;

use crate::cmd::EditorCommand;

#[derive(Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct CommandId(pub &'static str);

pub struct Command {
    pub id: CommandId,
    pub label: &'static str,
    pub category: Option<&'static str>,
    /// `Arc<dyn EditorCommand>` rather than `Box<...>` so callers can
    /// clone the trait-object cheaply and drop the registry borrow
    /// before calling `invoke(&mut Context)` — otherwise the immutable
    /// registry borrow and the mutable context borrow overlap.
    pub action: Arc<dyn EditorCommand>,
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
    /// keymaps may refer to ids that no longer exist. Returns an `Arc`
    /// clone so the caller can release the registry borrow and then
    /// pass `&mut Context` to `invoke`.
    pub fn resolve(&self, id: CommandId) -> Option<Arc<dyn EditorCommand>> {
        self.by_id.get(&id).map(|c| c.action.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::Quit;

    #[test]
    fn register_and_resolve() {
        let mut reg = CommandRegistry::new();
        reg.register(Command {
            id: CommandId("editor.quit"),
            label: "Quit",
            category: Some("App"),
            action: Arc::new(Quit),
        });
        assert_eq!(reg.len(), 1);
        assert!(reg.resolve(CommandId("editor.quit")).is_some());
        assert!(reg.resolve(CommandId("missing")).is_none());
    }

    #[test]
    fn re_register_preserves_order() {
        let mut reg = CommandRegistry::new();
        reg.register(Command {
            id: CommandId("a"), label: "A", category: None, action: Arc::new(Quit),
        });
        reg.register(Command {
            id: CommandId("b"), label: "B", category: None, action: Arc::new(Quit),
        });
        reg.register(Command {
            id: CommandId("a"), label: "A2", category: None, action: Arc::new(Quit),
        });
        let labels: Vec<&str> = reg.iter().map(|c| c.label).collect();
        assert_eq!(labels, vec!["A2", "B"]);
    }
}
