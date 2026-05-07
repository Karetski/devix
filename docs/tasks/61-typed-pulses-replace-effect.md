# Task T-61 — Convert closure-as-message dispatch to typed Pulses
Stage: 6
Status: pending
Depends on: T-60
Blocks:     T-63

## Goal
Drop the closure-as-message pattern. Every dispatch that carries a
`Box<dyn FnOnce(&mut ...) -> ...>` (today's Effect / EventSink
tail) becomes either a typed `Pulse::*` variant or a direct method
call (when no late binding is wanted).

## In scope
- Audit every `Effect::*` variant; map each to either:
  - A typed `Pulse::*` already in the v0 catalog (T-31), or
  - A direct method call inside core (synchronous, well-owned), or
  - A new typed Pulse variant — adding requires a
    `pulse-bus.md` minor-version bump and an Amendment-log entry
    (per `foundations-review.md` Spec-to-implementation feedback
    loop).
- Subscribers that needed closures now register typed handlers.
- Document any new Pulse variants in the amendment log if applicable.

## Out of scope
- Removing the `Effect` type itself (T-63).
- Frontend-originated pulses (T-62).

## Files touched
- `crates/devix-core/src/**/*.rs`: dispatch rewrites
- `docs/specs/foundations-review.md`: Amendment-log entry **only if**
  a new Pulse variant is added (else untouched)
- `docs/specs/pulse-bus.md`: variant additions **only if**

## Acceptance criteria
- [ ] No `Box<dyn FnOnce(...)>` dispatch survives in core.
- [ ] If new Pulse variants landed, the amendment log carries a
      dated entry citing this task.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/pulse-bus.md` — *Catalog (v0)*, *Versioning*.
- `docs/specs/foundations-review.md` — *Spec-to-implementation
  feedback loop*, *Amendment log*.
