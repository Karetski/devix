//! Multi-region selection model.
//!
//! A `Range` is anchor + head. They're equal for a plain cursor. Shift-extend
//! motion moves only the head; non-extending motion moves both.
//!
//! `Selection` holds a non-empty list of ranges plus a primary index. Phase 2
//! only ever produces single-range selections, but the structure is sized for
//! Phase 7 multi-cursor without a rewrite.

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
