//! buffer
//!
//! The text being edited, on mamre's rope. Positions are UTF-8 byte
//! offsets, as for `str`, and must fall on `char` boundaries; one that
//! doesn't is a [`BufferError`], not a panic.
use std::{fmt, range::Range};

use mamre_rope::Rope;

use crate::units::{Heading, Offset};

pub struct Buffer {
    rope: Rope,
}

/// A view into the buffer: a rope of its own that shares the buffer's
/// chunks, so taking one is cheap.
pub struct Slice {
    pub text: Rope,
}

impl Slice {
    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BufferError {
    /// `at` is past the end of the buffer.
    OutOfBounds { at: usize, len: usize },
    /// `at..end` runs past the end of the buffer (or overflows).
    RangeOutOfBounds { at: usize, end: usize, len: usize },
    /// `at` falls inside a `char`'s UTF-8 encoding.
    NotCharBoundary { at: usize },
}

impl fmt::Display for BufferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BufferError::OutOfBounds { at, len } => {
                write!(f, "byte {at} out of bounds for buffer of {len} bytes")
            }
            BufferError::RangeOutOfBounds { at, end, len } => {
                write!(
                    f,
                    "range {at}..{end} out of bounds for buffer of {len} bytes"
                )
            }
            BufferError::NotCharBoundary { at } => {
                write!(f, "byte {at} is not a char boundary")
            }
        }
    }
}

impl std::error::Error for BufferError {}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Buffer {
    pub fn new() -> Self {
        Self { rope: Rope::new() }
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.rope.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rope.is_empty()
    }

    pub fn prev_grapheme_boundary(&self, offset: usize) -> Option<Offset> {
        let position: Option<usize> = self.rope.prev_grapheme_boundary(offset);
        {
            let position = position?;
            Some(Offset {
                offset: position,
                heading: Heading::East,
            })
        }
    }

    pub fn next_grapheme_boundary(&self, offset: usize) -> Option<Offset> {
        let position: Option<usize> = self.rope.next_grapheme_boundary(offset);
        {
            let position = position?;
            Some(Offset {
                offset: position,
                heading: Heading::East,
            })
        }
    }

    /// Replaces the bytes in `range` with `text`. Returns the number of
    /// bytes added.
    pub fn replace(&mut self, range: Range<usize>, text: &str) -> Result<usize, BufferError> {
        let len = self.len();
        if range.start > range.end || range.end > len {
            return Err(BufferError::RangeOutOfBounds {
                at: range.start,
                end: range.end,
                len,
            });
        }
        self.check_boundary(range.start)?;
        self.check_boundary(range.end)?;
        self.rope.replace(range.start..range.end, text);
        Ok(text.len())
    }

    /// Inserts `text` at byte `at`. Returns the number of bytes added.
    pub fn insert(&mut self, at: usize, text: &str) -> Result<usize, BufferError> {
        let len = self.len();
        if at > len {
            return Err(BufferError::OutOfBounds { at, len });
        }
        self.check_boundary(at)?;
        self.rope.insert(at, text);
        Ok(text.len())
    }

    /// Reads `len` bytes starting at byte `at`.
    pub fn read(&self, at: usize, len: usize) -> Result<Slice, BufferError> {
        let buf_len = self.len();
        let end = at.saturating_add(len);
        if end > buf_len {
            return Err(BufferError::RangeOutOfBounds {
                at,
                end,
                len: buf_len,
            });
        }
        self.check_boundary(at)?;
        self.check_boundary(end)?;
        Ok(Slice {
            text: self.rope.slice(at..end),
        })
    }

    /// Errors unless `at`, already known to be in bounds, is a char
    /// boundary.
    fn check_boundary(&self, at: usize) -> Result<(), BufferError> {
        if self.is_char_boundary(at) {
            Ok(())
        } else {
            Err(BufferError::NotCharBoundary { at })
        }
    }

    /// Whether byte `at` (at most the length) starts a `char`, or is the
    /// end.
    ///
    /// The rope has this check but keeps it private, and everything public
    /// that takes a byte offset panics off a boundary. So: binary search the
    /// char offsets, each found in O(log n), for one landing on `at`. Swap
    /// for the rope's own O(log n) check once it's public.
    fn is_char_boundary(&self, at: usize) -> bool {
        let (mut lo, mut hi) = (0, self.rope.summary().chars);
        // The first char whose offset is at or past `at`.
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.rope.char_to_offset(mid) < at {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        self.rope.char_to_offset(lo) == at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_returns_bytes_added() {
        let mut b = Buffer::new();
        assert_eq!(b.insert(0, "héllo"), Ok(6));
        assert_eq!(b.insert(6, " 👍"), Ok(5));
    }

    #[test]
    fn insert_out_of_bounds() {
        let mut b = Buffer::new();
        assert_eq!(
            b.insert(1, "x"),
            Err(BufferError::OutOfBounds { at: 1, len: 0 })
        );
    }

    #[test]
    fn read_slice() {
        let mut b = Buffer::new();
        b.insert(0, "hello world").unwrap();
        let s = b.read(6, 5).unwrap();
        assert_eq!(s.text.to_string(), "world");
        assert_eq!(s.len(), 5);
    }

    #[test]
    fn read_out_of_bounds() {
        let mut b = Buffer::new();
        b.insert(0, "abc").unwrap();
        assert!(matches!(
            b.read(2, 5),
            Err(BufferError::RangeOutOfBounds {
                at: 2,
                end: 7,
                len: 3
            })
        ));
        assert!(matches!(
            b.read(1, usize::MAX),
            Err(BufferError::RangeOutOfBounds { .. })
        ));
    }

    #[test]
    fn char_boundaries() {
        // a: 1 byte, é: 2, ✓: 3, 👍: 4. Boundaries at 0, 1, 3, 6, 10.
        let mut b = Buffer::new();
        b.insert(0, "aé✓👍").unwrap();
        let boundaries: Vec<usize> = (0..=b.len()).filter(|&at| b.is_char_boundary(at)).collect();
        assert_eq!(boundaries, [0, 1, 3, 6, 10]);
    }

    #[test]
    fn char_boundaries_across_chunks() {
        // Enough multi-byte text to span many 512-byte chunks.
        let text = "é✓👍".repeat(200);
        let mut b = Buffer::new();
        b.insert(0, &text).unwrap();
        for at in 0..=text.len() {
            assert_eq!(
                b.is_char_boundary(at),
                text.is_char_boundary(at),
                "byte {at}"
            );
        }
    }
}
