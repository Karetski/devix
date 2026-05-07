//! Multi-region selection model.
//!
//! A `Range` is anchor + head. They're equal for a plain cursor. Shift-extend
//! motion moves only the head; non-extending motion moves both.
//!
//! `Selection` holds a non-empty list of ranges plus a primary index.
//! Phase 7 wires multi-cursor on top: `with_ranges` constructs from a list,
//! `push_range` appends + normalizes, `normalize` sorts and merges
//! overlaps while keeping the primary index pointing at whichever merged
//! range absorbed the original primary.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub anchor: usize,
    pub head: usize,
}

impl Range {
    pub const fn point(at: usize) -> Self {
        Self { anchor: at, head: at }
    }

    pub const fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    pub fn len(&self) -> usize {
        self.end() - self.start()
    }

    pub fn collapse_to_head(self) -> Self {
        Self { anchor: self.head, head: self.head }
    }

    pub fn put_head(self, head: usize, extend: bool) -> Self {
        if extend {
            Self { anchor: self.anchor, head }
        } else {
            Self { anchor: head, head }
        }
    }

    pub fn contains(&self, idx: usize) -> bool {
        idx >= self.start() && idx < self.end()
    }
}

#[derive(Clone, Debug)]
pub struct Selection {
    ranges: Vec<Range>,
    primary: usize,
}

impl Selection {
    pub fn point(at: usize) -> Self {
        Self { ranges: vec![Range::point(at)], primary: 0 }
    }

    pub fn single(range: Range) -> Self {
        Self { ranges: vec![range], primary: 0 }
    }

    /// Construct from an explicit range list. Sorts and merges overlapping
    /// ranges; the primary tracks whichever merged range absorbed the
    /// caller-supplied primary. Panics on empty input or out-of-bounds
    /// primary — both indicate caller bugs.
    pub fn with_ranges(ranges: Vec<Range>, primary: usize) -> Self {
        assert!(!ranges.is_empty(), "Selection::with_ranges requires at least one range");
        assert!(primary < ranges.len(), "primary index out of bounds");
        let mut s = Self { ranges, primary };
        s.normalize();
        s
    }

    /// Add `r` and re-normalize. The new range becomes the primary unless
    /// it gets merged into an existing range, in which case the merged
    /// range inherits primary. Use this for "add cursor above/below".
    pub fn push_range(&mut self, r: Range) {
        self.ranges.push(r);
        self.primary = self.ranges.len() - 1;
        self.normalize();
    }

    /// Drop every range except the primary. Used by Esc to leave
    /// multi-cursor mode.
    pub fn collapse_to_primary(&mut self) {
        let p = self.ranges[self.primary];
        self.ranges.clear();
        self.ranges.push(p);
        self.primary = 0;
    }

    /// Number of ranges. A `Selection` is never empty by construction —
    /// `is_empty` would always return `false`, which is why it isn't
    /// implemented; the clippy lint is silenced explicitly here.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize { self.ranges.len() }
    pub fn is_multi(&self) -> bool { self.ranges.len() > 1 }

    /// Sort by start and merge overlapping or touching ranges. Two
    /// ranges merge when the next one's start is `<=` the previous one's
    /// end — so adjacent point cursors at the same char position
    /// collapse, but a 0-length range and a non-empty range that just
    /// abut do not (touching at a single boundary leaves them distinct
    /// regions, matching what users see visually). The primary index is
    /// rewritten to point at whichever merged range absorbed the original.
    pub fn normalize(&mut self) {
        if self.ranges.len() == 1 {
            self.primary = 0;
            return;
        }
        let n = self.ranges.len();
        let mut tagged: Vec<(Range, bool)> = std::mem::take(&mut self.ranges)
            .into_iter()
            .enumerate()
            .map(|(i, r)| (r, i == self.primary))
            .collect();
        tagged.sort_by_key(|(r, _)| r.start());

        let mut merged: Vec<(Range, bool)> = Vec::with_capacity(n);
        for (r, is_primary) in tagged {
            if let Some(last) = merged.last_mut() {
                let strictly_overlap = r.start() < last.0.end();
                let touch_at_boundary = r.start() == last.0.end()
                    && (r.is_empty() || last.0.is_empty());
                if strictly_overlap || touch_at_boundary {
                    let new_start = last.0.start().min(r.start());
                    let new_end = last.0.end().max(r.end());
                    last.0 = Range::new(new_start, new_end);
                    last.1 = last.1 || is_primary;
                    continue;
                }
            }
            merged.push((r, is_primary));
        }
        let primary = merged.iter().position(|(_, p)| *p).unwrap_or(0);
        self.ranges = merged.into_iter().map(|(r, _)| r).collect();
        self.primary = primary;
    }

    pub fn ranges(&self) -> &[Range] {
        &self.ranges
    }

    pub fn primary(&self) -> Range {
        self.ranges[self.primary]
    }

    pub fn primary_mut(&mut self) -> &mut Range {
        &mut self.ranges[self.primary]
    }

    pub fn primary_index(&self) -> usize {
        self.primary
    }

    /// Apply `f` to every range; useful for motions in multi-cursor mode.
    pub fn transform(&mut self, mut f: impl FnMut(Range) -> Range) {
        for r in &mut self.ranges {
            *r = f(*r);
        }
    }

    /// Clamp every head/anchor into `[0, max]`. Used after external reloads.
    pub fn clamp(&mut self, max: usize) {
        for r in &mut self.ranges {
            r.anchor = r.anchor.min(max);
            r.head = r.head.min(max);
        }
    }

    /// Collapse all ranges to their heads.
    pub fn collapse(&mut self) {
        for r in &mut self.ranges {
            *r = r.collapse_to_head();
        }
    }
}

impl Default for Selection {
    fn default() -> Self {
        Self::point(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_ranges_sorts_by_start_and_tracks_primary() {
        let s = Selection::with_ranges(
            vec![Range::point(10), Range::point(2), Range::point(5)],
            0, // original primary at index 0 = point(10)
        );
        assert_eq!(s.len(), 3);
        assert_eq!(s.ranges()[0].head, 2);
        assert_eq!(s.ranges()[1].head, 5);
        assert_eq!(s.ranges()[2].head, 10);
        assert_eq!(s.primary().head, 10);
    }

    #[test]
    fn normalize_merges_overlapping_ranges() {
        let s = Selection::with_ranges(
            vec![Range::new(0, 5), Range::new(3, 8)],
            1,
        );
        assert_eq!(s.len(), 1);
        assert_eq!(s.ranges()[0].start(), 0);
        assert_eq!(s.ranges()[0].end(), 8);
    }

    #[test]
    fn normalize_collapses_duplicate_point_cursors() {
        let s = Selection::with_ranges(
            vec![Range::point(5), Range::point(5)],
            0,
        );
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn normalize_keeps_abutting_non_empty_ranges_distinct() {
        let s = Selection::with_ranges(
            vec![Range::new(0, 3), Range::new(3, 6)],
            0,
        );
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn push_range_marks_new_range_as_primary_when_disjoint() {
        let mut s = Selection::point(0);
        s.push_range(Range::point(10));
        assert_eq!(s.len(), 2);
        assert_eq!(s.primary().head, 10);
    }

    #[test]
    fn collapse_to_primary_drops_other_ranges() {
        let mut s = Selection::with_ranges(
            vec![Range::point(0), Range::point(5), Range::point(10)],
            1,
        );
        s.collapse_to_primary();
        assert_eq!(s.len(), 1);
        assert_eq!(s.primary().head, 5);
    }
}
