# Task T-80 — Tree-sitter highlighter as supervised actor
Stage: 8
Status: pending
Depends on: T-63, T-82
Blocks:     T-95

## Goal
Move the tree-sitter highlighter onto a dedicated thread with a
restart policy. Communicates with the rest of core via Pulses
(BufferChanged in; HighlightsReady out — variant added if not
already present, with an Amendment-log entry).

## In scope
- New module `crates/devix-core/src/supervise/highlighter.rs` (or
  parallel structure under `supervise`).
- Actor handle: takes a `BufferChanged` pulse, parses, emits a
  highlight result. Lives off the main thread.
- Supervisor restarts on panic per the policy from T-82.
- If a new pulse variant is needed (e.g.
  `Pulse::HighlightsReady { path, version }`), record the variant
  addition in `pulse-bus.md` and the amendment log.
- Tests: kill the worker mid-parse; supervisor restarts; output
  resumes.

## Out of scope
- LSP integration (future).
- Plugin actor (T-81).

## Files touched
- `crates/devix-core/src/supervise/highlighter.rs`
- `crates/devix-core/src/supervise/mod.rs`
- `docs/specs/pulse-bus.md` (only if a new variant lands)
- `docs/specs/foundations-review.md` Amendment log (only if)

## Acceptance criteria
- [ ] Highlighter runs off the main thread.
- [ ] Forced panic recovers under the supervisor.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/principles.md` — *Erlang/OTP — supervised isolation, let it
  crash*.
- `docs/specs/pulse-bus.md` — *What does not flow over the bus →
  Tree-sitter parses*.
