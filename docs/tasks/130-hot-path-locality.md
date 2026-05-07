# Task T-130 — Hot-path cache locality audit (DOD)
Stage: 13
Status: pending
Depends on: T-122
Blocks:     T-133

## Goal
Acton's principle in practice: walk the hot paths (rendering,
parsing, search) and replace `Box<dyn Trait>`-of-everything with
slotmaps / contiguous spans / typed enums where measurement shows
the indirection costs.

## In scope
- Identify hot paths via `cargo flamegraph` or equivalent on a
  representative file (large buffer + heavy syntax).
- Targeted rewrites: pick the top three offenders; report
  before/after timing in the task notes.
- Tests already passing remain green.

## Out of scope
- Premature optimization on cold paths.
- Allocator changes.

## Files touched
- (varies by audit findings)

## Acceptance criteria
- [ ] Three hot-path improvements with measured speedups.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/principles.md` — *Data-oriented design*.
