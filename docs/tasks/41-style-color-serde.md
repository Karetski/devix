# Task T-41 — Style + Color (canonical-string serde)
Stage: 4
Status: complete
Depends on: T-40
Blocks:     T-44, T-55, T-73, T-112

## Goal
Implement `Style`, `Color`, `NamedColor` in `devix-protocol::view`
with the **canonical string-form serde** locked in
`foundations-review.md` (`"default"`, `"#rrggbb"`, `"@<n>"`, named).

## In scope
- `Color` enum: `Default`, `Rgb(u8, u8, u8)`, `Indexed(u8)`,
  `Named(NamedColor)`.
- `NamedColor` enum: full v0 set per `frontend.md`.
- `Style` struct: `fg`, `bg`, `bold`, `italic`, `underline`, `dim`,
  `reverse`. Default impl returns all-default.
- Custom `Serialize` / `Deserialize` impls for `Color` (not derived)
  matching the four wire forms.
- Tests: parse table covers every wire form; rejects malformed
  input with deserialize error (no fallback to default).

## Out of scope
- Theme registry / `Theme::scopes` lookup (T-55).
- Quantization on TUI side (T-44 interpreter task touches this).

## Files touched
- `crates/devix-protocol/src/view.rs` (extended): Style, Color, NamedColor

## Acceptance criteria
- [ ] `Color::Rgb(0xaa, 0xaa, 0xaa)` serde-round-trips through
      `"#aaaaaa"` (and accepts `"#AAAAAA"` on input).
- [ ] `Color::Indexed(42)` round-trips through `"@42"`.
- [ ] `"#xyz"`, `"@256"`, and unknown named colors fail deserialize.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/frontend.md` — *Style*, *Color serialization*.
- `docs/specs/foundations-review.md` — *String-canonical
  serialization pattern*.
