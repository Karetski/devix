# Task T-32 — Protocol skeleton (envelopes, lanes, capabilities, handles)
Stage: 3
Status: complete
Depends on: T-30, T-31
Blocks:     T-43, T-110

## Goal
Implement the lane-message types from `protocol.md` in
`devix-protocol::protocol`. Implements the contract for client↔core
and plugin↔core message passing. The in-process bus is the
transport today; this skeleton lets lane handlers be written against
typed enums.

## In scope
- `Envelope<T>` (`protocol_version`, `seq`, `payload`).
- `ProtocolVersion { major, minor }` with custom serde to
  `"<major>.<minor>"` (locked per `foundations-review.md`).
- `Capability` enum — full v0 set per `protocol.md` *Capability
  negotiation*.
- Lane payload enums: `ClientToCore`, `CoreToClient`,
  `PluginToCore`, `CoreToPlugin`.
- `Request`, `Response`, `RequestError` enums (typed
  request/response correlation).
- `ViewResponse { root, view, version }` (uses `View` placeholder
  for now; concrete `View` lands in T-40).
- `ClientHello`, `ServerWelcome`, `PluginHello`, `PluginWelcome`
  structs.
- `ProtocolError` enum.
- Handle traits: `FrontendHandle`, `CoreHandle`, `PluginHandle`.
- `PathKind` enum for `Request::ListPaths` (closing `protocol.md`
  Q6): `Buffer`, `Cursor`, `Pane`, `Sidebar`, `Command`, `Theme`,
  `Plugin`. Documented inline so consumers know what they enumerate.
- Plugin capability mismatch policy (per Q2): warn-and-degrade with
  plugin opt-out — implemented as docs comment on `Capability` and
  `Welcome` types; concrete enforcement lands in T-110.
- Internal lane formalization (per Q3): `protocol.md` § *Lane 3*
  documents bus + direct calls only; this task adds an inline
  doc comment confirming "no internal envelope in v0."

## Out of scope
- Wire transport / framing (deferred per Q4).
- Streaming responses (deferred per Q1; v0 batches single
  Response::InvokeCommand).
- Concrete `View` definition (T-40).
- Concrete handler implementations in core (Stage 5+).

## Files touched
- `crates/devix-protocol/src/protocol.rs`: full skeleton
- `crates/devix-protocol/src/lib.rs`: re-exports

## Acceptance criteria
- [ ] Every locked v0 capability bit exists and round-trips serde.
- [ ] `ProtocolVersion` round-trips `"0.1"` ↔ `{ major: 0, minor: 1 }`.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/protocol.md` — *Message envelope*, *Lanes*,
  *Capability negotiation*, *Resolved during initial review*.
- `docs/specs/foundations-review.md` — *Gate T-22*, *String-canonical
  serialization pattern*.
