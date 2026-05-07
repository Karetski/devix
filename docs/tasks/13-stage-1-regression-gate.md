# Task T-13 — Stage-1 regression gate
Stage: 1
Status: pending
Depends on: T-10, T-11, T-12
Blocks:     all of Stage 2+

## Goal
Verify the five-crate layout (`devix-text`, `devix-syntax`,
`devix-protocol`, `devix-core`, `devix-tui`) is in place, the
workspace builds clean, every existing test passes, and the binary
launches a working editor end-to-end with no user-visible behavior
change. After this lands, Stage 1 is complete.

## In scope
- Clean build: `cargo clean && cargo build --workspace`.
- Full test: `cargo test --workspace`.
- Manual sanity: `cargo run --bin devix <test-file>`. Verify open,
  edit, save, undo, palette open, plugin sidebar (if a plugin is
  configured). No visual or behavioral regression vs. main.
- Resolve any leftover dead deps, dangling re-exports, or
  unreferenced files surfaced during the gate.
- Update workspace docs (`docs/roadmap.md` only if user explicitly
  asks; spec-locked otherwise — do not modify in this task).

## Out of scope
- New features, new abstractions, refactor work beyond cleanups
  surfaced by build warnings.
- Behavior changes.

## Files touched
- (no new code; possibly minor lint/cleanup edits across the
  workspace)

## Acceptance criteria
- [ ] Workspace contains exactly five crates: text, syntax,
      protocol, core, tui.
- [ ] `cargo build --workspace` passes with zero warnings.
- [ ] `cargo test  --workspace` passes.
- [ ] Binary launches and edits a file end-to-end without
      behavior change.
- [ ] No `// TODO: stage-1` markers left behind.

## Spec references
- `docs/specs/crates.md` — *Stage-1 sequencing T-13*.
