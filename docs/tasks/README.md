# devix — Implementation tasks

Per-task scope files for the thirteen-stage implementation of the
foundations specs in `docs/specs/`. Each task file is a working contract:
goal, in/out of scope, files touched, acceptance criteria, spec
references.

## Numbering scheme

**`T-NM`** where **N** = stage number (1–13) and **M** = task index
within the stage (0-indexed). Stage 10 uses three-digit ids (`T-100`..);
stages 1–9 use two-digit (`T-10`..`T-95`).

This matches the explicit `T-10..T-13` citation in `docs/specs/crates.md`
for Stage 1 and extends consistently for Stages 2–13. Locked at Phase 1;
not amendable without re-numbering every downstream task file.

## Legacy task-id cross-walk

`docs/specs/foundations-review.md` references a few task ids using a
pre-staging scheme. They map to the new scheme as follows:

| Legacy id (in foundations-review) | Subject | New id |
|---|---|---|
| T-21 | Pulse-bus skeleton | T-31 |
| T-22 | Protocol skeleton | T-32 |
| T-23 | Manifest reader skeleton | T-33 |
| T-30 | Migrate documents onto namespace | T-50 |
| T-71 | LayoutNode → Pane collapse | T-91 |

The legacy ids are left in `foundations-review.md` verbatim — editing
that doc is a spec amendment per its own policy, and the open-questions
gates remain meaningful read with this cross-walk. New work cites the
new ids only.

## Stages

| Stage | Theme | Tasks |
|---|---|---|
| 1 | Crate split | [T-10](10-create-devix-protocol.md), [T-11](11-create-devix-core.md), [T-12](12-rename-app-to-tui.md), [T-13](13-stage-1-regression-gate.md) |
| 2 | Mechanical wins | [T-20](20-workspace-deps-hygiene.md), [T-21](21-add-skeleton-deps.md), [T-22](22-drop-dead-modules.md), [T-23](23-test-reorganization.md), [T-24](24-clippy-baseline.md), [T-25](25-rename-substrate-dirs.md) |
| 3 | Foundation skeletons | [T-30](30-namespace-skeleton.md), [T-31](31-pulse-bus-skeleton.md), [T-32](32-protocol-skeleton.md), [T-33](33-manifest-skeleton.md) |
| 4 | View-tree IR adoption | [T-40](40-view-enum.md), [T-41](41-style-color-serde.md), [T-42](42-input-event-serde.md), [T-43](43-core-view-producer.md), [T-44](44-tui-view-interpreter.md) |
| 5 | Namespace migration | [T-50](50-buffers-onto-namespace.md), [T-51](51-cursors-onto-namespace.md), [T-52](52-panes-onto-namespace.md), [T-53](53-commands-onto-namespace.md), [T-54](54-keymap-onto-namespace.md), [T-55](55-themes-onto-namespace.md), [T-56](56-plugin-callbacks-onto-namespace.md), [T-57](57-pulses-carry-paths.md) |
| 6 | Pulse-bus migration | [T-60](60-replace-event-sink.md), [T-61](61-typed-pulses-replace-effect.md), [T-62](62-frontend-pulses.md), [T-63](63-drop-effect-eventsink.md) |
| 7 | Built-ins to manifest | [T-70](70-author-builtin-manifest.md), [T-71](71-load-builtin-commands.md), [T-72](72-load-builtin-keymap.md), [T-73](73-load-builtin-theme.md), [T-74](74-drop-source-builtins.md) |
| 8 | Supervise actors | [T-80](80-tree-sitter-actor.md), [T-81](81-plugin-actor.md), [T-82](82-supervisor-primitive.md) |
| 9 | LayoutNode → Pane | [T-90](90-synthetic-id-strategy.md), [T-91](91-collapse-layoutnode.md), [T-92](92-rect-cache-to-tui.md), [T-93](93-confirm-pane-trait-home.md), [T-94](94-fold-composites.md), [T-95](95-stage-9-regression-gate.md) |
| 10 | Editor split | [T-100](100-editor-split-pane-registry.md), [T-101](101-focus-chain-owner.md), [T-102](102-ops-owner.md), [T-103](103-modal-slot-owner.md), [T-104](104-stage-10-regression-gate.md) |
| 11 | Extension surface | [T-110](110-plugin-contributes-commands.md), [T-111](111-plugin-contributes-panes.md), [T-112](112-plugin-contributes-themes.md), [T-113](113-plugin-contributes-settings.md) |
| 12 | SICP combinators | [T-120](120-view-combinators.md), [T-121](121-pulse-filter-combinators.md), [T-122](122-manifest-composition.md) |
| 13 | DOD finishing pass | [T-130](130-hot-path-locality.md), [T-131](131-pulse-delivery-alloc.md), [T-132](132-view-allocation.md), [T-133](133-final-regression-bench.md) |

## Dependency graph (stage-level)

```
1 ──► 2 ──► 3 ──► 4 ──► 5 ──► 6 ──► 7 ──► 8 ──► 9 ──► 10 ──► 11 ──► 12 ──► 13
```

Strictly serial. No stage interleaves. Within a stage, intra-stage
dependencies are recorded in each task file's `Depends on` /
`Blocks` lines.

## Conventions

- Status starts at `pending`; flip to `in-progress` on start, `complete`
  on commit.
- Acceptance always requires `cargo build --workspace` and
  `cargo test --workspace` green.
- Every task file's `Spec references` cites at least one
  `docs/specs/*.md` section. Code that contradicts a cited section is
  rejected (per `docs/specs/foundations-review.md` Spec-to-implementation
  feedback loop).
- Work commits to the `refactor/foundations` branch with messages of the
  form `stage-<N>/T-<NM>: <title>`.
