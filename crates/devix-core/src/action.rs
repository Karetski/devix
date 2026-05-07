//! `Action` — invocable behavior, first-class.
//!
//! Today's editor command set lives in a 50-variant enum the dispatcher
//! match-arms over (`devix_surface::Action`). This trait is the
//! Lattner-shaped replacement: each command becomes a struct that
//! implements `Action`. Keymaps map chords to `Box<dyn Action<Ctx>>`,
//! the palette stores them too, and plugins contribute new commands by
//! adding new types — not by growing a central enum.
//!
//! Generic over `Ctx` so different hosts can supply different mutable
//! contexts (the editor's `Context<'_>`, a future plugin host, a test
//! harness). The host picks one `Ctx` and stores actions as
//! `Box<dyn for<'a> Action<HostCtx<'a>>>` via HRTB; plugin authors just
//! `impl Action<HostCtx<'_>> for MyCmd` and the storage works.
//!
//! `core` does not define a concrete `Ctx` — it would either be empty
//! (useless) or surface-typed (closed). Keeping `Ctx` generic is what
//! lets `core` stay the stable plugin surface.

/// One editor command. Self-describing, value-typed, plugin-extendable.
///
/// `'static` lets the dispatcher store actions as `Box<dyn Action<Ctx>>`.
/// Per-action data (e.g. the `delta` of a scroll, the path of an open)
/// lives on the struct's fields.
pub trait Action<Ctx: ?Sized>: 'static {
    fn invoke(&self, ctx: &mut Ctx);
}
