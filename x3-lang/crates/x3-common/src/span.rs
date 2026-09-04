//! Source span and position tracking for X3.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::Range;

/// A byte position in source code.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct BytePos(pub u32);

impl BytePos {
    pub const ZERO: BytePos = BytePos(0);

    #[inline]
    pub fn new(pos: u32) -> Self {
        BytePos(pos)
    }

    #[inline]
    pub fn as_usize(self) -> usize {
        self.0 as usize
    }

    #[inline]
    pub fn offset(self, delta: i32) -> BytePos {
        BytePos((self.0 as i32 + delta) as u32)
    }
}

impl fmt::Debug for BytePos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BytePos({})", self.0)
    }
}

impl fmt::Display for BytePos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u32> for BytePos {
    fn from(pos: u32) -> Self {
        BytePos(pos)
    }
}

impl From<usize> for BytePos {
    fn from(pos: usize) -> Self {
        BytePos(pos as u32)
    }
}

/// A span of source code, represented as a start and end byte position.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Span {
    pub start: BytePos,
    pub end: BytePos,
    /// Source file ID for multi-file compilation
    pub file_id: u32,
}

impl Span {
    pub const DUMMY: Span = Span {
        start: BytePos::ZERO,
        end: BytePos::ZERO,
        file_id: 0,
    };

    #[inline]
    pub fn new(start: BytePos, end: BytePos, file_id: u32) -> Self {
        Span { start, end, file_id }
    }

    #[inline]
    pub fn from_range(range: Range<usize>, file_id: u32) -> Self {
        Span {
            start: BytePos(range.start as u32),
            end: BytePos(range.end as u32),
            file_id,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        (self.end.0 - self.start.0) as usize
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    #[inline]
    pub fn is_dummy(&self) -> bool {
        *self == Self::DUMMY
    }

    /// Create a span that covers both spans.
    #[inline]
    pub fn merge(self, other: Span) -> Span {
        debug_assert_eq!(self.file_id, other.file_id);
        Span {
            start: std::cmp::min(self.start, other.start),
            end: std::cmp::max(self.end, other.end),
            file_id: self.file_id,
        }
    }

    /// Create a span between two spans (exclusive).
    #[inline]
    pub fn between(self, other: Span) -> Span {
        debug_assert_eq!(self.file_id, other.file_id);
        Span {
            start: self.end,
            end: other.start,
            file_id: self.file_id,
        }
    }

    #[inline]
    pub fn to_range(self) -> Range<usize> {
        self.start.as_usize()..self.end.as_usize()
    }

    #[inline]
    pub fn contains(self, pos: BytePos) -> bool {
        self.start <= pos && pos < self.end
    }

    #[inline]
    pub fn shrink_to_start(self) -> Span {
        Span {
            start: self.start,
            end: self.start,
            file_id: self.file_id,
        }
    }

    #[inline]
    pub fn shrink_to_end(self) -> Span {
        Span {
            start: self.end,
            end: self.end,
            file_id: self.file_id,
        }
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}@{}", self.start.0, self.end.0, self.file_id)
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start.0, self.end.0)
    }
}

/// A value with an associated span.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    #[inline]
    pub fn new(node: T, span: Span) -> Self {
        Spanned { node, span }
    }

    #[inline]
    pub fn dummy(node: T) -> Self {
        Spanned {
            node,
            span: Span::DUMMY,
        }
    }

    #[inline]
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Spanned<U> {
        Spanned {
            node: f(self.node),
            span: self.span,
        }
    }

    #[inline]
    pub fn as_ref(&self) -> Spanned<&T> {
        Spanned {
            node: &self.node,
            span: self.span,
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for Spanned<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} @ {:?}", self.node, self.span)
    }
}

impl<T: fmt::Display> fmt::Display for Spanned<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.node)
    }
}

impl<T> std::ops::Deref for Spanned<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.node
    }
}

impl<T> std::ops::DerefMut for Spanned<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.node
    }
}

/// Line and column information for human-readable error messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LineCol {
    pub line: u32, // 1-indexed
    pub col: u32,  // 1-indexed
}

impl LineCol {
    pub fn new(line: u32, col: u32) -> Self {
        LineCol { line, col }
    }
}

impl fmt::Display for LineCol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

/// A range with line/column information.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LineColRange {
    pub start: LineCol,
    pub end: LineCol,
}

impl fmt::Display for LineColRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.start.line == self.end.line {
            write!(f, "{}:{}-{}", self.start.line, self.start.col, self.end.col)
        } else {
            write!(f, "{}-{}", self.start, self.end)
        }
    }
}
