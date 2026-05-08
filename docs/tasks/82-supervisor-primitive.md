# Task T-82 — Supervisor primitive + restart policy
Stage: 8
Status: complete
Depends on: T-31, T-63
Blocks:     T-80, T-81

## Goal
Implement a small supervision primitive at
`devix-core::supervise`. One-for-one restart strategy with a
default 3-restarts-then-give-up policy. Supervised children
communicate with the rest of core only via Pulses (no direct
method calls into supervisor internals; per `protocol.md` *Lane 3
internal lane formalization*).

## In scope
- `Supervisor`, `SupervisedChild`, `RestartPolicy { max_restarts,
  window }`. Defaults: 3 restarts in 30s; on exhaustion, supervisor
  escalates by publishing `Pulse::PluginError` (or a generic
  `Pulse::SupervisorGaveUp` if added — record in amendment log).
- One-for-one strategy: only the failing child restarts.
- Supervisor's own crash bubbles up; documented as a lethal error.
- Tests: child panic loop trips the policy; surface event observed.

## Out of scope
- One-for-all / rest-for-one strategies (future).
- LSP-specific actors (future).

## Files touched
- `crates/devix-core/src/supervise/mod.rs`
- `crates/devix-core/src/supervise/policy.rs`

## Acceptance criteria
- [ ] Supervised child with controlled panic restarts up to the
      policy limit, then escalates.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/principles.md` — *Erlang/OTP*.
- `docs/specs/protocol.md` — *Lane 3*, *Open Q3*.
