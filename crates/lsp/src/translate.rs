//! Buffer-edit → LSP `TextDocumentContentChangeEvent` translator.
//!
//! A buffer transaction holds a list of changes sorted ascending by `start`,
//! all expressed as char offsets into the rope **before** the transaction.
//! Sending those to LSP correctly requires care:
//!
//! - LSP applies the events in `content_changes` order, with each event's
//!   range interpreted relative to the document state *after the previous
//!   event was applied*.
//! - We translate each change's range from the **pre-edit** rope.
//! - To make those positions still valid at apply time, we send the events
//!   in **reverse** order — largest position first. Earlier (smaller)
//!   positions remain unmodified because no edit at a smaller offset has
//!   yet been applied.
//!
//! Single-change transactions (the only kind Phase 2 emits) collapse to a
//! single event and the order question is moot. Multi-change becomes
//! relevant once multi-cursor lands.
//!
//! Position encoding: utf-8 uses byte math directly via `Rope::char_to_byte`.
//! utf-16 walks the line up to the char position summing `c.len_utf16()`.
//! Anything else is treated as utf-32 (`character` = chars-in-line).

use lsp_types::{Position, PositionEncodingKind, Range, TextDocumentContentChangeEvent};
use ropey::Rope;

/// One edit, expressed as char offsets into the **pre-edit** rope.
#[derive(Clone, Debug)]
pub struct Edit<'a> {
    pub start_char: usize,
    pub end_char: usize,
    pub text: &'a str,
}

/// Translate a transaction's changes into LSP content-change events. The
/// returned vec is in apply order (reverse of the input).
pub fn translate_changes(
    pre_rope: &Rope,
    edits: &[Edit<'_>],
    encoding: &PositionEncodingKind,
) -> Vec<TextDocumentContentChangeEvent> {
    edits
        .iter()
        .rev()
        .map(|e| TextDocumentContentChangeEvent {
            range: Some(Range {
                start: position_in_rope(pre_rope, e.start_char, encoding),
                end: position_in_rope(pre_rope, e.end_char, encoding),
            }),
            range_length: None,
            text: e.text.to_owned(),
        })
        .collect()
}

/// Build an LSP `Position` from a char offset into `rope`, using the
/// negotiated encoding.
pub fn position_in_rope(
    rope: &Rope,
    char_idx: usize,
    encoding: &PositionEncodingKind,
) -> Position {
    let clamped = char_idx.min(rope.len_chars());
    let line = rope.char_to_line(clamped);
    let line_start = rope.line_to_char(line);
    let character = if encoding == &PositionEncodingKind::UTF8 {
        let line_byte_start = rope.char_to_byte(line_start);
        let byte = rope.char_to_byte(clamped);
        (byte - line_byte_start) as u32
    } else if encoding == &PositionEncodingKind::UTF32 {
        (clamped - line_start) as u32
    } else {
        // utf-16 (default per spec when not negotiated): sum surrogate-aware
        // code-unit lengths for chars in [line_start, clamped).
        let mut units: u32 = 0;
        for c in rope.slice(line_start..clamped).chars() {
            units += c.len_utf16() as u32;
        }
        units
    };
    Position { line: line as u32, character }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rope(s: &str) -> Rope {
        Rope::from_str(s)
    }

    #[test]
    fn position_utf8_handles_multibyte() {
        // 'é' is 2 bytes in UTF-8.
        let r = rope("héllo");
        // char index 1 is just after 'h'; line offset = 1 byte.
        let p = position_in_rope(&r, 1, &PositionEncodingKind::UTF8);
        assert_eq!(p, Position { line: 0, character: 1 });
        // char index 2 is just after 'é'; byte offset is 1 + 2 = 3.
        let p = position_in_rope(&r, 2, &PositionEncodingKind::UTF8);
        assert_eq!(p, Position { line: 0, character: 3 });
    }

    #[test]
    fn position_utf16_handles_surrogate_pair() {
        // U+1F600 is outside the BMP → 2 utf-16 code units, 1 char.
        let r = rope("a😀b");
        let p = position_in_rope(&r, 1, &PositionEncodingKind::UTF16);
        assert_eq!(p, Position { line: 0, character: 1 });
        let p = position_in_rope(&r, 2, &PositionEncodingKind::UTF16);
        assert_eq!(p, Position { line: 0, character: 3 });
        let p = position_in_rope(&r, 3, &PositionEncodingKind::UTF16);
        assert_eq!(p, Position { line: 0, character: 4 });
    }

    #[test]
    fn position_utf32_is_chars_in_line() {
        let r = rope("héllo");
        for i in 0..=5 {
            let p = position_in_rope(&r, i, &PositionEncodingKind::UTF32);
            assert_eq!(p, Position { line: 0, character: i as u32 });
        }
    }

    #[test]
    fn position_resets_per_line() {
        let r = rope("ab\ncde");
        let p = position_in_rope(&r, 4, &PositionEncodingKind::UTF8);
        assert_eq!(p, Position { line: 1, character: 1 });
    }

    #[test]
    fn translate_single_insert() {
        let r = rope("hello");
        let edits = [Edit { start_char: 5, end_char: 5, text: " world" }];
        let events = translate_changes(&r, &edits, &PositionEncodingKind::UTF8);
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.text, " world");
        let r = e.range.unwrap();
        assert_eq!(r.start, Position { line: 0, character: 5 });
        assert_eq!(r.end, Position { line: 0, character: 5 });
    }

    #[test]
    fn translate_single_replace() {
        let r = rope("hello");
        let edits = [Edit { start_char: 1, end_char: 4, text: "EY" }];
        let events = translate_changes(&r, &edits, &PositionEncodingKind::UTF8);
        let e = &events[0];
        assert_eq!(e.text, "EY");
        assert_eq!(e.range.unwrap().start, Position { line: 0, character: 1 });
        assert_eq!(e.range.unwrap().end, Position { line: 0, character: 4 });
    }

    #[test]
    fn translate_multi_change_emits_reverse_order() {
        // Pre-rope: "abcdefgh" (8 chars). Two non-overlapping changes:
        //   c1: replace [1, 2) with "X"   — positions 1..2 in original
        //   c2: replace [5, 6) with "Y"   — positions 5..6 in original
        // Translator should emit c2 first, c1 second, so applying them
        // in order against the live document yields the same final state
        // as applying them right-to-left against the pre-rope.
        let r = rope("abcdefgh");
        let edits = [
            Edit { start_char: 1, end_char: 2, text: "X" },
            Edit { start_char: 5, end_char: 6, text: "Y" },
        ];
        let events = translate_changes(&r, &edits, &PositionEncodingKind::UTF8);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].text, "Y");
        assert_eq!(events[0].range.unwrap().start.character, 5);
        assert_eq!(events[1].text, "X");
        assert_eq!(events[1].range.unwrap().start.character, 1);
    }

    #[test]
    fn position_clamps_past_end() {
        let r = rope("hi");
        let p = position_in_rope(&r, 99, &PositionEncodingKind::UTF8);
        assert_eq!(p, Position { line: 0, character: 2 });
    }
}
