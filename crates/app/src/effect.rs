//! `Effect` — runtime-internal deferred work, drained between messages.
//!
//! The closed enum covers the runtime-coordination ops the loop needs to
//! know about (redraw, quit). `Effect::Run` is the open escape hatch — any
//! deferred mutation that doesn't deserve a named variant goes here as a
//! closure invoked on the main thread with a fresh `AppContext`.

use crate::context::AppContext;

pub type EffectFn = Box<dyn for<'a> FnOnce(&mut AppContext<'a>) + Send>;

pub enum Effect {
    Redraw,
    Quit,
    Run(EffectFn),
}
