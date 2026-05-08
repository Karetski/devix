//! Namespace primitives — `Path`, `PathError`, `Lookup`.
//!
//! Implements `docs/specs/namespace.md`. The full grammar:
//!
//! ```text
//! Path     := "/" Segment ("/" Segment)*
//! Segment  := SegChar+
//! SegChar  := ALPHA | DIGIT | "-" | "_" | "."
//! ```
//!
//! `Path` is `Arc<str>`-backed so cloning is two atomic ops; the canonical
//! string form is the wire form via custom serde (see
//! `foundations-review.md` § *String-canonical serialization pattern*).
//!
//! `Lookup` is single-resource per call. Ops needing disjoint mutable
//! borrows on the same store reach for `std::mem::{take, swap, replace}`
//! on the store's internal storage; no `lookup_two_mut` helper, no
//! per-registry split API (locked during T-30; see amendment log
//! 2026-05-07).

use std::sync::Arc;

use serde::de::{self, Deserializer, Visitor};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

/// A path-shaped resource address. Always begins with `/`, has at least
/// one segment, never has empty segments, and never has a trailing `/`.
#[derive(Clone, Eq, PartialEq, Hash)]
pub struct Path(Arc<str>);

impl Path {
    /// Parse a string into a `Path`. Returns `Err` on grammar violations.
    pub fn parse(s: &str) -> Result<Self, PathError> {
        if s.is_empty() {
            return Err(PathError::Empty);
        }
        if !s.starts_with('/') {
            return Err(PathError::MissingLeadingSlash);
        }
        if s.len() > 1 && s.ends_with('/') {
            return Err(PathError::TrailingSlash);
        }
        // Skip the leading `/` and validate every segment.
        let body = &s[1..];
        if body.is_empty() {
            return Err(PathError::EmptySegment);
        }
        for seg in body.split('/') {
            if seg.is_empty() {
                return Err(PathError::EmptySegment);
            }
            if !seg.bytes().all(is_seg_byte) {
                return Err(PathError::InvalidSegment(seg.to_string()));
            }
        }
        Ok(Path(Arc::from(s)))
    }

    /// Iterate segments (without the leading slash). Always yields at
    /// least one segment by grammar.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        // SAFETY of unwrap: every constructed Path begins with `/` and
        // has at least one non-empty segment (validated by `parse`).
        self.0.split('/').filter(|s| !s.is_empty())
    }

    /// First segment (e.g., `"buf"` for `"/buf/42"`). Always present.
    pub fn root(&self) -> &str {
        self.segments().next().expect("path has at least one segment")
    }

    /// Parent path, or `None` if this is a single-segment path
    /// (single-segment paths have no parent because `/` is not a valid
    /// path).
    pub fn parent(&self) -> Option<Path> {
        let s: &str = &self.0;
        let last = s.rfind('/')?;
        // Strip everything from the last `/` onward.
        if last == 0 {
            // Single segment — `/buf` has no parent.
            return None;
        }
        Some(Path(Arc::from(&s[..last])))
    }

    /// Append a segment. Returns `Err` if `segment` violates the
    /// segment grammar.
    pub fn join(&self, segment: &str) -> Result<Path, PathError> {
        if segment.is_empty() {
            return Err(PathError::EmptySegment);
        }
        if !segment.bytes().all(is_seg_byte) {
            return Err(PathError::InvalidSegment(segment.to_string()));
        }
        let mut s = String::with_capacity(self.0.len() + 1 + segment.len());
        s.push_str(&self.0);
        s.push('/');
        s.push_str(segment);
        Ok(Path(Arc::from(s)))
    }

    /// True if `self`'s segment sequence starts with `other`'s. The check
    /// is segment-aware, not byte-level — `/buf/4` does *not* start with
    /// `/buf/42`.
    pub fn starts_with(&self, other: &Path) -> bool {
        let mine = self.0.as_ref();
        let theirs = other.0.as_ref();
        // Bytes must match for the prefix length.
        if !mine.starts_with(theirs) {
            return false;
        }
        // And the next byte (if any) must be the segment separator. If
        // `mine == theirs`, the prefix is the whole thing — also a match.
        matches!(mine.as_bytes().get(theirs.len()), None | Some(&b'/'))
    }

    /// Borrow as `&str` (canonical form, leading slash, no trailing
    /// slash).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_seg_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.'
}

impl std::fmt::Debug for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Path({:?})", self.0.as_ref())
    }
}

impl std::fmt::Display for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Grammar-violation errors from `Path::parse` / `Path::join`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PathError {
    /// Input string was empty (`""`).
    #[error("path is empty")]
    Empty,
    /// Input did not start with `/`.
    #[error("path must start with `/`")]
    MissingLeadingSlash,
    /// Input ended with `/`.
    #[error("path must not end with `/`")]
    TrailingSlash,
    /// At least one segment was empty (e.g., `"/"` or `"/buf//42"`).
    #[error("empty segment in path")]
    EmptySegment,
    /// A segment contained a reserved character (whitespace, `:`, `*`,
    /// `?`, non-ASCII, etc.).
    #[error("segment `{0}` contains reserved character")]
    InvalidSegment(String),
}

impl Serialize for Path {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Path {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct PathVisitor;
        impl<'de> Visitor<'de> for PathVisitor {
            type Value = Path;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a canonical-form devix path string (e.g. `/buf/42`)")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Path, E> {
                Path::parse(v).map_err(de::Error::custom)
            }
        }
        d.deserialize_str(PathVisitor)
    }
}

/// One interface every per-resource registry implements. Local — there
/// is no global multi-resource lookup that crosses kinds. A
/// `BufferStore` is `Lookup<Resource = Document>`; a `CommandRegistry`
/// is `Lookup<Resource = dyn Command>`. Consumers always know which
/// registry they're addressing.
pub trait Lookup {
    /// The resource type this registry serves.
    type Resource: ?Sized;

    /// Resolve `path` to a borrow of the resource it names, or `None`
    /// if no such resource exists at this path inside this registry.
    fn lookup(&self, path: &Path) -> Option<&Self::Resource>;

    /// Mutable variant. Locked single-resource: the trait does not
    /// expose a two-paths helper. Ops needing disjoint mutation use
    /// `std::mem::{take, swap, replace}` on the store's internal
    /// storage. (See namespace.md Q1; resolved 2026-05-07.)
    fn lookup_mut(&mut self, path: &Path) -> Option<&mut Self::Resource>;

    /// Iterate every path this registry currently holds. Order is
    /// implementation-defined; consumers use this to enumerate
    /// resources of a kind (e.g., the palette listing every
    /// `/cmd/...`).
    fn paths(&self) -> Box<dyn Iterator<Item = Path> + '_>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_grammar_examples() {
        let good = [
            "/buf/42",
            "/cur/3",
            "/pane",
            "/pane/0/1",
            "/cmd/edit.copy",
            "/keymap/ctrl-s",
            "/keymap/ctrl-shift-p",
            "/theme/keyword.control",
            "/plugin/file-tree",
            "/plugin/file-tree/cmd/refresh",
            "/plugin/file-tree/pane/main",
        ];
        for s in good {
            let p = Path::parse(s).expect(s);
            assert_eq!(p.as_str(), s);
        }
    }

    #[test]
    fn parse_rejects_grammar_violations() {
        assert_eq!(Path::parse(""), Err(PathError::Empty));
        assert_eq!(Path::parse("buf/42"), Err(PathError::MissingLeadingSlash));
        assert_eq!(Path::parse("/buf/42/"), Err(PathError::TrailingSlash));
        assert_eq!(Path::parse("/"), Err(PathError::EmptySegment));
        assert_eq!(Path::parse("/buf//42"), Err(PathError::EmptySegment));
        // Reserved chars: whitespace, `:`, `*`, `?`, non-ASCII.
        assert!(matches!(
            Path::parse("/buf/4 2"),
            Err(PathError::InvalidSegment(_))
        ));
        assert!(matches!(
            Path::parse("/buf/foo:bar"),
            Err(PathError::InvalidSegment(_))
        ));
        assert!(matches!(
            Path::parse("/buf/*"),
            Err(PathError::InvalidSegment(_))
        ));
        assert!(matches!(
            Path::parse("/buf/é"),
            Err(PathError::InvalidSegment(_))
        ));
    }

    #[test]
    fn segments_iterates_correctly() {
        let p = Path::parse("/pane/0/1").unwrap();
        let segs: Vec<&str> = p.segments().collect();
        assert_eq!(segs, vec!["pane", "0", "1"]);
    }

    #[test]
    fn root_returns_first_segment() {
        assert_eq!(Path::parse("/buf/42").unwrap().root(), "buf");
        assert_eq!(Path::parse("/cmd/edit.copy").unwrap().root(), "cmd");
        assert_eq!(Path::parse("/pane").unwrap().root(), "pane");
    }

    #[test]
    fn parent_drops_last_segment() {
        let p = Path::parse("/pane/0/1").unwrap();
        assert_eq!(p.parent().unwrap().as_str(), "/pane/0");
        let p = Path::parse("/pane/0").unwrap();
        assert_eq!(p.parent().unwrap().as_str(), "/pane");
        // Single-segment path has no parent.
        let p = Path::parse("/buf").unwrap();
        assert!(p.parent().is_none());
    }

    #[test]
    fn join_appends_segment() {
        let base = Path::parse("/plugin/file-tree").unwrap();
        let joined = base.join("cmd").unwrap().join("refresh").unwrap();
        assert_eq!(joined.as_str(), "/plugin/file-tree/cmd/refresh");
    }

    #[test]
    fn join_rejects_invalid_segment() {
        let base = Path::parse("/buf").unwrap();
        assert!(matches!(base.join(""), Err(PathError::EmptySegment)));
        assert!(matches!(
            base.join("hello world"),
            Err(PathError::InvalidSegment(_))
        ));
    }

    #[test]
    fn starts_with_is_segment_aware() {
        let p42 = Path::parse("/buf/42").unwrap();
        let p4 = Path::parse("/buf/4").unwrap();
        let buf = Path::parse("/buf").unwrap();
        // Byte-prefix-but-not-segment rejected.
        assert!(!p42.starts_with(&p4));
        // Whole path is a prefix of itself.
        assert!(p42.starts_with(&p42));
        // Proper prefix.
        assert!(p42.starts_with(&buf));
        assert!(p4.starts_with(&buf));
    }

    #[test]
    fn serde_round_trip_uses_canonical_string() {
        let p = Path::parse("/plugin/file-tree/cmd/refresh").unwrap();
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, "\"/plugin/file-tree/cmd/refresh\"");
        let back: Path = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn deserialize_rejects_invalid_strings() {
        let bad = serde_json::from_str::<Path>("\"buf/42\"");
        assert!(bad.is_err());
        let bad = serde_json::from_str::<Path>("\"\"");
        assert!(bad.is_err());
    }

    #[test]
    fn hash_uses_canonical_string() {
        use std::collections::HashMap;
        let mut m: HashMap<Path, u32> = HashMap::new();
        m.insert(Path::parse("/buf/42").unwrap(), 1);
        // Same path parsed independently must hash equal.
        assert_eq!(m.get(&Path::parse("/buf/42").unwrap()), Some(&1));
    }
}
