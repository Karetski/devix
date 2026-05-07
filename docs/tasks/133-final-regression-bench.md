# Task T-133 — Final regression + baseline benchmark + sign-off
Stage: 13
Status: pending
Depends on: T-130, T-131, T-132
Blocks:     —

## Goal
Final sign-off across the foundations refactor. Capture a baseline
benchmark for hot paths so future regressions are detectable.
Verify every Stage 1–12 acceptance criterion still holds. Confirm
no `// DEVIATES FROM SPEC` markers exist anywhere in the workspace.

## In scope
- `cargo build --workspace` (zero warnings).
- `cargo test  --workspace`.
- `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo run --bin devix` manual sanity (open / split / palette /
  plugin pane / theme switch / save / quit).
- Baseline benchmark numbers checked into
  `crates/devix-core/benches/baseline.txt` with date.
- Confirm `foundations-review.md` Amendment log lists every
  amendment landed across stages.

## Out of scope
- New features.
- v1 work.

## Files touched
- `crates/devix-core/benches/baseline.txt`: new

## Acceptance criteria
- [ ] All four commands pass clean.
- [ ] No `// DEVIATES FROM SPEC` strings in the workspace.
- [ ] Baseline benchmark file committed with current numbers.
- [ ] Amendment log review surfaces no missing entries.

## Spec references
- `docs/specs/foundations-review.md` — *Spec-to-implementation
  feedback loop*, *Amendment log*.
- `docs/principles.md` — every star.
