//! Theme commands. Exposes the runtime theme-switch entrypoint
//! plus a built-in "cycle through registered themes" action so the
//! user has a keyboard-reachable theme switch without a palette UI.

use crate::Action;
use crate::editor::commands::context::Context;

/// Activate the theme identified by `id`. No-op if the id isn't
/// registered in `editor.theme_store`. Used by plugins (via the
/// Lua bridge `devix.set_theme(id)`) and by the built-in `theme.cycle`
/// action below.
pub struct SetTheme(pub String);
impl<'a> Action<Context<'a>> for SetTheme {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.editor.set_theme(&self.0);
    }
}

/// Cycle to the next registered theme in `theme_store::ids` order.
/// Wraps. The built-in keymap binds this so the user can switch
/// themes without typing an id; per-theme `SetTheme(id)` actions are
/// available for plugins that want a direct switch.
pub struct CycleTheme;
impl<'a> Action<Context<'a>> for CycleTheme {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let ids: Vec<String> = ctx
            .editor
            .theme_store
            .ids()
            .map(|s| s.to_string())
            .collect();
        if ids.is_empty() {
            return;
        }
        let active = ctx.editor.active_theme_id.clone();
        let next = match active.and_then(|a| ids.iter().position(|i| *i == a)) {
            Some(idx) => ids[(idx + 1) % ids.len()].clone(),
            None => ids[0].clone(),
        };
        ctx.editor.set_theme(&next);
    }
}
