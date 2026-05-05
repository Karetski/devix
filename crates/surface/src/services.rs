//! `RenderServices` — the read-only handle the structural Pane tree
//! borrows from during paint.
//!
//! Lives in `surface` (not `core`) because its fields are
//! editor-specific. Plumbed from the host into the structural Panes'
//! `render` impls via a *scoped thread-local*: the host opens a
//! `RenderServices::scope(|| root.render(...))` block, and structural
//! Panes (`LayoutFrame`, `LayoutSidebar`) inside that scope call
//! `RenderServices::with(|s| ...)` to get the borrow.
//!
//! Why scoped TLS rather than a `RenderCtx::services: dyn Any`
//! field: `dyn Any` requires `'static`, which would force every
//! borrow inside `RenderServices` to be `'static` too — exactly what
//! we *don't* want. Scoped TLS is the standard Rust pattern for
//! threading borrowed context through a callback boundary you
//! don't control (here, `Pane::render`'s signature in `core`).
//!
//! Plugin-supplied sidebar content is resolved through a closure so
//! `surface` doesn't need to depend on the binary's plugin world.

use std::cell::Cell;
use std::ptr::NonNull;

use devix_core::{Pane, Theme};
use slotmap::SlotMap;

use crate::cursor::{Cursor, CursorId};
use devix_workspace::{DocId, Document};

use crate::layout::SidebarSlot;
use crate::surface::{LeafRef, RenderCache};

pub struct RenderServices<'a> {
    pub documents: &'a SlotMap<DocId, Document>,
    pub cursors: &'a SlotMap<CursorId, Cursor>,
    pub theme: &'a Theme,
    pub render_cache: &'a RenderCache,
    /// Which leaf currently holds focus, in tree-identity terms. The
    /// structural Panes consult this to decide whether to draw their
    /// "active" chrome variant (cursor placement, focused border).
    pub focused_leaf: Option<LeafRef>,
    pub plugin_sidebar: &'a dyn Fn(SidebarSlot) -> Option<Box<dyn Pane>>,
}

thread_local! {
    /// Lifetime-erased pointer to the active `RenderServices`.
    /// Non-null only inside a `RenderServices::scope` callback.
    static ACTIVE: Cell<Option<NonNull<()>>> = const { Cell::new(None) };
}

impl<'a> RenderServices<'a> {
    /// Run `f` with `self` installed as the active services. Restores
    /// the previous value (typically `None`) on exit so nested scopes
    /// behave like a stack and unrelated panic-recovered renders
    /// don't see a stale pointer.
    pub fn scope<R>(&self, f: impl FnOnce() -> R) -> R {
        // Lifetime-erase by raw pointer. The pointer is valid for the
        // duration of `f` because `&self` is borrowed for at least
        // that long. `with` re-attaches a borrow lifetime that does
        // not outlive `f`.
        let raw = NonNull::from(self).cast::<()>();
        let prev = ACTIVE.with(|c| c.replace(Some(raw)));
        // Use a guard to restore even on panic — render is not
        // expected to panic, but a stale TLS pointer would be a
        // dangerous footgun if it did.
        struct Restore(Option<NonNull<()>>);
        impl Drop for Restore {
            fn drop(&mut self) {
                ACTIVE.with(|c| c.set(self.0));
            }
        }
        let _g = Restore(prev);
        f()
    }

    /// Borrow the active `RenderServices` for the duration of `f`.
    /// Returns `None` if called outside any `scope` — in that case
    /// callers (structural Panes' `render` impls) treat it as a
    /// no-op render. The lifetime returned is bounded by `f`.
    pub fn with<R>(f: impl for<'b> FnOnce(&'b RenderServices<'b>) -> R) -> Option<R> {
        let ptr = ACTIVE.with(|c| c.get())?;
        // SAFETY: `ptr` was set by `scope` from a `&RenderServices`
        // that is borrowed for at least the duration of the closure
        // currently executing inside `scope`. We're called from
        // *inside* such a closure (TLS is non-null), so the pointer
        // is dereferenceable. We cast to a fresh `'b` lifetime that
        // is local to `f` — `f` cannot leak it past its own return
        // because of the HRTB on the closure.
        let services = unsafe { &*(ptr.as_ptr() as *const RenderServices<'_>) };
        Some(f(services))
    }
}
