# Task T-30 — Namespace skeleton (`Path`, `PathError`, `Lookup`)
Stage: 3
Status: complete
Depends on: T-13, T-21
Blocks:     T-50, T-51, T-52, T-53, T-54, T-55, T-56, T-57

## Goal
Implement the namespace primitives from `namespace.md` in
`devix-protocol::path`. After this task, `Path` can be parsed,
serialized, prefix-tested, and used as a `HashMap` key; `Lookup` is
available for registries to implement (Stage 5 does the migrations).

## In scope
- `Path(Arc<str>)` with parse, segments, root, parent, join,
  starts_with, as_str.
- `PathError` enum with the five variants from `namespace.md`.
- `Lookup` trait (associated `type Resource: ?Sized`, `lookup`,
  `lookup_mut`, `paths`).
- Custom `Serialize` / `Deserialize` for `Path` going through the
  canonical string form (locked by `foundations-review.md` —
  *String-canonical serialization pattern*).
- Unit tests: grammar acceptance + rejection table; round-trip
  serde; `starts_with` segment-aware semantics; `Hash` consistent
  with canonical string.

## Out of scope
- `Lookup` impls for any registry (Stage 5).
- `lookup_two_mut` helper (deferred per `namespace.md` Q1 →
  T-50 decision).
- Globbing / patterns (deferred per `namespace.md` Q2).
- Per-root id-from-path parsers (Stage 5; locked per `namespace.md`
  Q3).

## Files touched
- `crates/devix-protocol/src/path.rs`: full implementation
- `crates/devix-protocol/src/lib.rs`: `pub use path::*;`

## Acceptance criteria
- [ ] All grammar examples in `namespace.md` parse round-trip.
- [ ] `Path::starts_with` rejects byte-prefix-but-not-segment
      cases (`/buf/4` ⊄ `/buf/42`).
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/namespace.md` — *Path grammar*, *The `Path` type*,
  *The `Lookup` trait*, *Resolved during initial review*.
- `docs/specs/foundations-review.md` — *String-canonical serialization
  pattern*.
