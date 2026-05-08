//! Command registry: a discoverable, id-keyed editor over the editor's
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

use devix_protocol::Lookup;
use devix_protocol::path::Path;

use crate::editor::commands::cmd::EditorCommand;

/// A command id. Wire form on paths is `/cmd/<dotted-id>` per
/// `docs/specs/namespace.md` § *Migration table*; the dotted form
/// (`edit.copy`, `palette.open`) is preserved as a single segment.
#[derive(Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct CommandId(pub &'static str);

impl CommandId {
    /// Encode this id into its canonical path (`/cmd/<dotted-id>`).
    pub fn to_path(self) -> Path {
        Path::parse(&format!("/cmd/{}", self.0)).expect("/cmd/<dotted-id> is canonical")
    }

    /// Decode a `/cmd/<dotted-id>` path back into a command-id
    /// segment slice. Returns `None` for any other shape.
    pub fn segment_from_path(path: &Path) -> Option<&str> {
        let mut segs = path.segments();
        if segs.next()? != "cmd" {
            return None;
        }
        let id_seg = segs.next()?;
        if segs.next().is_some() {
            return None;
        }
        Some(id_seg)
    }
}

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

impl Lookup for CommandRegistry {
    type Resource = dyn EditorCommand;

    fn lookup(&self, path: &Path) -> Option<&dyn EditorCommand> {
        let segment = CommandId::segment_from_path(path)?;
        // Look up by dotted-segment string. Iterate `by_id` since
        // CommandId wraps `&'static str` (no fast borrow-as-str
        // hashmap key path); v0 catalog is small enough that the
        // linear scan is a non-issue.
        for (id, cmd) in &self.by_id {
            if id.0 == segment {
                return Some(&*cmd.action);
            }
        }
        None
    }

    /// Commands are stored as `Arc<dyn EditorCommand>` (so
    /// `resolve` can hand out cheap clones), which means
    /// `lookup_mut` cannot hand out an exclusive `&mut`. Per the
    /// 2026-05-07 lookup_mut resolution, mutating ops on the
    /// registry use direct API (`register`) rather than
    /// `lookup_mut`. This impl always returns `None`.
    fn lookup_mut(&mut self, _path: &Path) -> Option<&mut dyn EditorCommand> {
        None
    }

    fn paths(&self) -> Box<dyn Iterator<Item = Path> + '_> {
        Box::new(self.order.iter().map(|id| id.to_path()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::commands::cmd::Quit;

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
    fn command_id_round_trips_through_path() {
        let id = CommandId("edit.copy");
        let path = id.to_path();
        assert_eq!(path.as_str(), "/cmd/edit.copy");
        assert_eq!(CommandId::segment_from_path(&path), Some("edit.copy"));
        // Reject non-cmd roots.
        let p = Path::parse("/buf/3").unwrap();
        assert_eq!(CommandId::segment_from_path(&p), None);
        // Reject extra segments.
        let p = Path::parse("/cmd/a/b").unwrap();
        assert_eq!(CommandId::segment_from_path(&p), None);
    }

    #[test]
    fn registry_lookup_returns_action_via_path() {
        let mut reg = CommandRegistry::new();
        reg.register(Command {
            id: CommandId("editor.quit"),
            label: "Quit",
            category: Some("App"),
            action: Arc::new(Quit),
        });
        let p = Path::parse("/cmd/editor.quit").unwrap();
        assert!(reg.lookup(&p).is_some());
        assert!(reg.lookup(&Path::parse("/cmd/missing").unwrap()).is_none());
        // lookup_mut is always None for the Arc-backed registry.
        assert!(reg.lookup_mut(&p).is_none());
        let paths: Vec<String> = reg.paths().map(|p| p.as_str().to_string()).collect();
        assert_eq!(paths, vec!["/cmd/editor.quit"]);
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
