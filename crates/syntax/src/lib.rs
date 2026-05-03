//! Syntax = tree-sitter wrapper, one parser per document.
//!
//! Phase 4 — orthogonal primitives:
//! * [`Language`] — closed enum of supported grammars (Rust today).
//! * [`Highlighter`] — owns a `Parser`, the current `Tree`, and the compiled
//!   highlights `Query`. Drives incremental reparse from buffer transactions.
//! * [`HighlightSpan`] — `(byte_range, scope)` produced by the highlights
//!   query for a viewport-sized byte range. The renderer walks visible lines
//!   and converts spans into styled cells via the theme.
//!
//! The Document layer is responsible for calling `edit` *before* each `parse`
//! so tree-sitter can reuse unchanged subtrees. A full parse without a prior
//! `edit` call is also valid (e.g. on first open or after disk reload).

use std::path::Path;

use anyhow::{Result, anyhow};
use ropey::Rope;
use streaming_iterator::StreamingIterator;
use tree_sitter::{
    InputEdit, Language as TsLanguage, Node, Parser, Point, Query, QueryCursor, TextProvider, Tree,
};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Language {
    Rust,
}

impl Language {
    /// Resolve a language from a file path's extension. Returns `None` for
    /// unknown extensions; callers treat that as plain-text (no highlighter).
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "rs" => Some(Language::Rust),
            _ => None,
        }
    }

    /// LSP `languageId` for `textDocument/didOpen`. Must match what the
    /// server expects; rust-analyzer wants `"rust"`.
    pub fn lsp_id(self) -> &'static str {
        match self {
            Language::Rust => "rust",
        }
    }

    fn ts_language(self) -> TsLanguage {
        match self {
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        }
    }

    fn highlights_query_source(self) -> &'static str {
        match self {
            Language::Rust => tree_sitter_rust::HIGHLIGHTS_QUERY,
        }
    }
}

/// One highlight span emitted by the highlights query. Tree-sitter is allowed
/// to emit overlapping captures (a node and one of its descendants both
/// matching different patterns); the renderer applies them in source order so
/// the most-specific capture wins via last-write paint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighlightSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub scope: String,
}

pub struct Highlighter {
    language: Language,
    parser: Parser,
    query: Query,
    tree: Option<Tree>,
}

impl Highlighter {
    pub fn new(language: Language) -> Result<Self> {
        let mut parser = Parser::new();
        let ts_lang = language.ts_language();
        parser
            .set_language(&ts_lang)
            .map_err(|e| anyhow!("set_language: {e}"))?;
        let query = Query::new(&ts_lang, language.highlights_query_source())
            .map_err(|e| anyhow!("compiling highlights query: {e}"))?;
        Ok(Self {
            language,
            parser,
            query,
            tree: None,
        })
    }

    pub fn language(&self) -> Language {
        self.language
    }

    /// Parse `rope` from scratch (or incrementally if `edit` was called since
    /// the last parse). The previous tree, if any, is fed back to tree-sitter
    /// so unchanged subtrees can be reused — this is the cheap path.
    pub fn parse(&mut self, rope: &Rope) {
        let new_tree = self.parser.parse_with(
            &mut |byte: usize, _: Point| -> &[u8] {
                if byte >= rope.len_bytes() {
                    return &[];
                }
                let (chunk, chunk_byte_idx, _, _) = rope.chunk_at_byte(byte);
                &chunk.as_bytes()[byte - chunk_byte_idx..]
            },
            self.tree.as_ref(),
        );
        self.tree = new_tree;
    }

    /// Apply a buffer edit to the current tree. Must be called *before* the
    /// next `parse` so tree-sitter can localise the reparse.
    pub fn edit(&mut self, edit: &InputEdit) {
        if let Some(t) = self.tree.as_mut() {
            t.edit(edit);
        }
    }

    /// Drop the current tree. Call after a non-incremental change (disk
    /// reload, undo across many edits) so the next `parse` is a full reparse.
    pub fn invalidate(&mut self) {
        self.tree = None;
    }

    /// Run the highlights query restricted to `[start_byte, end_byte)`.
    /// Empty when the highlighter hasn't parsed yet.
    pub fn highlights(
        &self,
        rope: &Rope,
        start_byte: usize,
        end_byte: usize,
    ) -> Vec<HighlightSpan> {
        let Some(tree) = self.tree.as_ref() else {
            return Vec::new();
        };
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(start_byte..end_byte);
        let capture_names = self.query.capture_names();
        let provider = RopeProvider(rope);
        let mut out = Vec::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), provider);
        while let Some(m) = matches.next() {
            for cap in m.captures {
                let scope = capture_names[cap.index as usize];
                let node = cap.node;
                out.push(HighlightSpan {
                    start_byte: node.start_byte(),
                    end_byte: node.end_byte(),
                    scope: scope.to_string(),
                });
            }
        }
        out
    }
}

/// Translate a buffer-character edit into tree-sitter's byte+point InputEdit.
/// The buffer layer works in chars; tree-sitter wants bytes + line/col Points.
/// `before` is the rope state *before* the transaction, `after` is *after*.
/// Both are required because `new_end_byte`/`new_end_position` depend on the
/// post-edit content.
pub fn input_edit_for_range(
    before: &Rope,
    after: &Rope,
    start_char: usize,
    old_end_char: usize,
    new_end_char: usize,
) -> InputEdit {
    InputEdit {
        start_byte: char_to_byte(before, start_char),
        old_end_byte: char_to_byte(before, old_end_char),
        new_end_byte: char_to_byte(after, new_end_char),
        start_position: char_to_point(before, start_char),
        old_end_position: char_to_point(before, old_end_char),
        new_end_position: char_to_point(after, new_end_char),
    }
}

fn char_to_byte(rope: &Rope, char_idx: usize) -> usize {
    let clamped = char_idx.min(rope.len_chars());
    rope.char_to_byte(clamped)
}

fn char_to_point(rope: &Rope, char_idx: usize) -> Point {
    let clamped = char_idx.min(rope.len_chars());
    let line = rope.char_to_line(clamped);
    let line_start_char = rope.line_to_char(line);
    let col_chars = clamped - line_start_char;
    // Tree-sitter Points are byte-indexed within a row.
    let line_start_byte = rope.char_to_byte(line_start_char);
    let col_byte = rope.char_to_byte(line_start_char + col_chars) - line_start_byte;
    Point::new(line, col_byte)
}

struct RopeProvider<'a>(&'a Rope);

impl<'a> TextProvider<&'a [u8]> for RopeProvider<'a> {
    type I = RopeChunks<'a>;

    fn text(&mut self, node: Node) -> Self::I {
        RopeChunks {
            rope: self.0,
            pos: node.start_byte(),
            end: node.end_byte().min(self.0.len_bytes()),
        }
    }
}

struct RopeChunks<'a> {
    rope: &'a Rope,
    pos: usize,
    end: usize,
}

impl<'a> Iterator for RopeChunks<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.end {
            return None;
        }
        let (chunk, chunk_byte_idx, _, _) = self.rope.chunk_at_byte(self.pos);
        let chunk_bytes = chunk.as_bytes();
        let local_start = self.pos - chunk_byte_idx;
        let local_end = chunk_bytes.len().min(self.end - chunk_byte_idx);
        let slice = &chunk_bytes[local_start..local_end];
        self.pos = chunk_byte_idx + local_end;
        Some(slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_from_extension() {
        assert_eq!(
            Language::from_path(Path::new("foo.rs")),
            Some(Language::Rust)
        );
        assert_eq!(Language::from_path(Path::new("foo.txt")), None);
        assert_eq!(Language::from_path(Path::new("Cargo.toml")), None);
    }

    #[test]
    fn parse_rust_produces_highlights() {
        let rope = Rope::from_str("fn main() { let x = 1; }");
        let mut h = Highlighter::new(Language::Rust).unwrap();
        h.parse(&rope);
        let spans = h.highlights(&rope, 0, rope.len_bytes());
        assert!(
            !spans.is_empty(),
            "rust source should produce at least one highlight"
        );
    }

    #[test]
    fn highlights_include_keyword_for_fn() {
        let rope = Rope::from_str("fn main() {}");
        let mut h = Highlighter::new(Language::Rust).unwrap();
        h.parse(&rope);
        let spans = h.highlights(&rope, 0, rope.len_bytes());
        let kw = spans.iter().find(|s| s.start_byte == 0 && s.end_byte == 2);
        assert!(
            kw.is_some(),
            "expected a span at the `fn` keyword: {spans:?}"
        );
        assert!(
            kw.unwrap().scope.starts_with("keyword"),
            "fn should be tagged as a keyword, got {:?}",
            kw.unwrap().scope
        );
    }

    #[test]
    fn invalidate_then_reparse_works() {
        let mut h = Highlighter::new(Language::Rust).unwrap();
        let r1 = Rope::from_str("fn a() {}");
        h.parse(&r1);
        h.invalidate();
        let r2 = Rope::from_str("fn b() {}");
        h.parse(&r2);
        let spans = h.highlights(&r2, 0, r2.len_bytes());
        assert!(!spans.is_empty());
    }

    #[test]
    fn input_edit_translates_chars_to_bytes_and_points() {
        let before = Rope::from_str("fn a() {}\nfn b() {}");
        let mut after = before.clone();
        after.insert(10, "// hi\n");
        let edit = input_edit_for_range(&before, &after, 10, 10, 16);
        assert_eq!(edit.start_byte, 10);
        assert_eq!(edit.old_end_byte, 10);
        assert_eq!(edit.new_end_byte, 16);
        assert_eq!(edit.start_position.row, 1);
        assert_eq!(edit.start_position.column, 0);
        assert_eq!(edit.new_end_position.row, 2);
        assert_eq!(edit.new_end_position.column, 0);
    }
}
