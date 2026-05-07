# Task T-132 — View IR allocation pass
Stage: 13
Status: pending
Depends on: T-130
Blocks:     T-133

## Goal
Reduce per-render allocation in the View producer + interpreter.
Small-vec or arena for hot-path `Vec<View>` / `Vec<TextSpan>` /
highlight spans; `&[View]` slices where ownership permits.

## In scope
- Identify the worst offenders during a typical render (mostly
  Stack/Split/List children + Buffer highlights).
- Add a `smallvec` or arena-backed alternative behind the same API.
- Benchmarks before/after.

## Out of scope
- Changing the public View API shape (must round-trip with serde).

## Files touched
- `crates/devix-protocol/src/view.rs`: internal Vec → smallvec
  (transparent to consumers if API preserved)
- `crates/devix-core/src/editor/view.rs`
- `crates/devix-tui/src/interpreter.rs`

## Acceptance criteria
- [ ] Render alloc count drops on the benchmark scene.
- [ ] Serde round-trip still passes.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/principles.md` — *Data-oriented design*.
- `docs/specs/frontend.md` — *View IR*.
