# Task T-42 — InputEvent + Chord + KeyCode (canonical-kebab serde)
Stage: 4
Status: complete
Depends on: T-30, T-40
Blocks:     T-44, T-54, T-62

## Goal
Implement `InputEvent`, `Chord`, `KeyCode`, `Modifiers`, `MouseKind`,
`MouseButton` in `devix-protocol::input` with the **canonical
kebab-case serde** for `Chord` and `KeyCode` matching the
`/keymap/<chord>` form in `namespace.md`.

## In scope
- `InputEvent` enum: Key, Mouse, Scroll, Paste, FocusGained,
  FocusLost.
- `Chord { key: KeyCode, modifiers: Modifiers }`.
- `KeyCode`: Char, Enter, Tab, BackTab, Esc, Backspace, Delete,
  Insert, Left, Right, Up, Down, Home, End, PageUp, PageDown, F(u8).
- `Modifiers { ctrl, alt, shift, super_key }`.
- Custom serde for `Chord` + `KeyCode` (not derived). Wire form
  uses fixed modifier order (ctrl-alt-shift-super) and lowercase
  named keys per `namespace.md` *Chord segments*.
- `Chord::text` policy (per `frontend.md` Q5, lean: always set when
  printable). Confirmed inline in code comments + tests.
- Tests: parse table per `namespace.md`; rejects out-of-order
  modifiers, uppercase modifiers, unknown keys.

## Out of scope
- Keymap registration (T-54).
- crossterm → InputEvent translation in tui (T-44).

## Files touched
- `crates/devix-protocol/src/input.rs`: full impl

## Acceptance criteria
- [ ] `"ctrl-shift-p"` round-trips through `Chord { Char('p'),
      ctrl+shift }`.
- [ ] `"shift-tab"` round-trips through `Chord { Tab, shift }`.
- [ ] `"f12"` round-trips through `Chord { F(12), no-mods }`.
- [ ] `"shift-ctrl-p"` (wrong order) fails deserialize.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/frontend.md` — *InputEvent*, *Chord serialization*,
  *Open Q5*.
- `docs/specs/namespace.md` — *Chord segments*.
