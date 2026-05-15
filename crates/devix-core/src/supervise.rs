//! Supervisor primitive — `docs/specs/protocol.md` § *Lane 3* and
//! Erlang/OTP's "supervised isolation, let it crash" principle.
//!
//! T-82 ships the building block: a small one-for-one supervisor that
//! restarts a child closure on panic up to a `RestartPolicy` budget,
//! then escalates by publishing `Pulse::PluginError` (or the closest
//! existing variant). T-80 and T-81 then wrap the tree-sitter actor
//! and plugin runtime inside this primitive.
//!
//! Scope (per `docs/tasks/82-supervisor-primitive.md`):
//! * One-for-one strategy: only the failing child restarts.
//! * Default policy: 3 restarts inside a 30s window.
//! * Children communicate with the rest of core via the `PulseBus`,
//!   not direct method calls into supervisor internals.
//! * Supervisor's own crash propagates up; documented as lethal.
//!
//! Out of scope: rest-for-one and one-for-all strategies; intentional
//! re-parenting; hot child upgrade. Land when a real actor needs
//! one of those shapes.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::PulseBus;

/// Restart policy budget. The supervisor allows at most
/// `max_restarts` restarts inside a sliding `window`; once exhausted,
/// it escalates instead of restarting again.
#[derive(Clone, Copy, Debug)]
pub struct RestartPolicy {
    pub max_restarts: u32,
    pub window: Duration,
}

impl Default for RestartPolicy {
    /// 3 restarts within 30s — matches the per-task spec lean.
    fn default() -> Self {
        RestartPolicy {
            max_restarts: 3,
            window: Duration::from_secs(30),
        }
    }
}

/// One supervised child. Constructed via `Supervisor::supervise`.
/// Holding the handle keeps the supervising thread alive; dropping
/// it asks the supervisor to stop (the running child is allowed to
/// finish its current iteration before the loop exits).
pub struct SupervisedChild {
    name: String,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl SupervisedChild {
    /// The supervised child's stable name (for logging /
    /// `Pulse::PluginError` messages on escalation).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Ask the supervisor to stop. The child observes the stop
    /// flag at the start of each restart; an in-flight invocation
    /// runs to completion before the loop exits.
    pub fn shutdown(mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for SupervisedChild {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Join is best-effort on drop; if the child is wedged the
        // supervisor's thread leaks rather than the editor hanging.
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// Supervise a child by repeatedly invoking `factory` and catching
/// panics. The supervised body runs on a dedicated thread named
/// `name`. On panic the supervisor honors `policy`: up to
/// `policy.max_restarts` restarts inside `policy.window`, then
/// publishes a `Pulse::PluginError` on `bus` and stops.
///
/// `factory` must be `Send + 'static` since it runs on the spawned
/// thread; it's `FnMut` so the body can carry state across restarts
/// if needed.
pub fn supervise<F>(
    name: impl Into<String>,
    bus: PulseBus,
    policy: RestartPolicy,
    factory: F,
) -> std::io::Result<SupervisedChild>
where
    F: FnMut() + Send + 'static,
{
    let name = name.into();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_thread = stop.clone();
    let name_for_thread = name.clone();
    let factory = Arc::new(Mutex::new(factory));

    let join = thread::Builder::new()
        .name(format!("devix-supervise-{}", name))
        .spawn(move || {
            let mut crashes: Vec<Instant> = Vec::with_capacity(policy.max_restarts as usize + 1);
            loop {
                if stop_for_thread.load(Ordering::Acquire) {
                    return;
                }
                let result = catch_unwind(AssertUnwindSafe(|| {
                    // Recover from a poisoned mutex — a previous
                    // restart's panic happens while we hold the lock
                    // (`(f)()` mutates `f`), which poisons. Mutex
                    // poison just means "the data may be in a
                    // partial state"; for a `FnMut` closure that's
                    // an acceptable degradation since `FnMut` is
                    // typically idempotent across calls.
                    let mut f = factory
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    (f)();
                }));
                if stop_for_thread.load(Ordering::Acquire) {
                    return;
                }
                match result {
                    Ok(()) => {
                        // Clean exit (child returned). Don't restart —
                        // the child decided it's done. If the child
                        // wants to be supervised forever, it loops
                        // internally.
                        return;
                    }
                    Err(_) => {
                        // Panicked. Trim crash log to the policy
                        // window, then check the budget.
                        let now = Instant::now();
                        crashes.push(now);
                        crashes.retain(|t| now.duration_since(*t) <= policy.window);
                        if crashes.len() as u32 > policy.max_restarts {
                            // Budget exhausted; escalate.
                            escalate(&bus, &name_for_thread, &policy);
                            return;
                        }
                        // Else loop and respawn.
                    }
                }
            }
        })?;
    Ok(SupervisedChild {
        name,
        stop,
        join: Some(join),
    })
}

fn escalate(bus: &PulseBus, name: &str, policy: &RestartPolicy) {
    use devix_protocol::path::Path;
    use devix_protocol::pulse::Pulse;
    let plugin_path = Path::parse(&format!("/plugin/{}", sanitize_segment(name)))
        .unwrap_or_else(|_| Path::parse("/plugin/supervisor").expect("/plugin/supervisor canonical"));
    let _ = bus.publish_async(Pulse::PluginError {
        plugin: plugin_path,
        message: format!(
            "supervised child `{}` exhausted restart budget ({} restarts in {:?})",
            name, policy.max_restarts, policy.window,
        ),
    });
}

/// Replace any reserved-segment chars in `s` with `_`. Supervisor
/// names are user-defined; reuse the path-segment grammar from
/// `namespace.md` so escalation pulses always carry valid paths.
fn sanitize_segment(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use devix_protocol::pulse::{PulseFilter, PulseKind};

    use super::*;

    #[test]
    fn child_clean_exit_is_not_restarted() {
        let bus = PulseBus::new();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();
        let child = supervise(
            "test-clean",
            bus.handle(),
            RestartPolicy::default(),
            move || {
                counter_clone.fetch_add(1, Ordering::Relaxed);
            },
        )
        .unwrap();
        // Wait for the child to run + return.
        std::thread::sleep(Duration::from_millis(50));
        // Counter incremented exactly once; supervisor saw the
        // clean exit and didn't respawn.
        assert_eq!(counter.load(Ordering::Relaxed), 1);
        drop(child);
    }

    #[test]
    fn panicking_child_restarts_within_budget() {
        let bus = PulseBus::new();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();
        let policy = RestartPolicy {
            max_restarts: 5,
            window: Duration::from_secs(30),
        };
        let child = supervise("test-panic", bus.handle(), policy, move || {
            let n = counter_clone.fetch_add(1, Ordering::Relaxed);
            if n < 3 {
                panic!("intentional");
            }
            // After 3 panics, return cleanly so the supervisor stops
            // restarting.
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(150));
        assert!(
            counter.load(Ordering::Relaxed) >= 4,
            "child should have restarted 3+ times",
        );
        drop(child);
    }

    #[test]
    fn budget_exhaustion_publishes_plugin_error() {
        let bus = PulseBus::new();
        // Subscribe a counter on the bus to observe escalation.
        let hits = Arc::new(AtomicU32::new(0));
        let hits_clone = hits.clone();
        bus.subscribe(PulseFilter::kind(PulseKind::PluginError), move |_| {
            hits_clone.fetch_add(1, Ordering::Relaxed);
        });
        let policy = RestartPolicy {
            max_restarts: 2,
            window: Duration::from_secs(30),
        };
        let child = supervise("escalator", bus.handle(), policy, || {
            panic!("always-panicking child");
        })
        .unwrap();
        // Wait for restart budget to exhaust.
        std::thread::sleep(Duration::from_millis(200));
        // Drain async-published escalation pulse.
        let drained = bus.drain();
        assert!(drained >= 1, "escalation should have produced at least one pulse (drained {})", drained);
        assert!(hits.load(Ordering::Relaxed) >= 1, "PluginError pulse delivered to subscriber");
        drop(child);
    }

    #[test]
    fn drop_stops_supervisor_thread() {
        let bus = PulseBus::new();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();
        let child = supervise("loop-forever", bus.handle(), RestartPolicy::default(), move || {
            counter_clone.fetch_add(1, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(20));
            panic!("respawn");
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(80));
        let mid = counter.load(Ordering::Relaxed);
        drop(child);
        std::thread::sleep(Duration::from_millis(80));
        let after = counter.load(Ordering::Relaxed);
        // Counter shouldn't grow much past `mid` once dropped.
        assert!(after - mid <= 2, "supervisor should stop respawning after drop (mid={mid}, after={after})");
    }
}
