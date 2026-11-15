//! buffer
//!
//!
use std::{fmt, range::Range};

pub struct Buffer {
    buffer: ropey::Rope,
}

/// A borrowed view into the buffer. Indices are in chars.
pub struct Slice<'a> {
    pub text: ropey::RopeSlice<'a>,
}

impl Slice<'_> {
    pub fn len(&self) -> usize {
        self.text.len_chars()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BufferError {
    /// `at` is past the end of the buffer.
    OutOfBounds { at: usize, len: usize },
    /// `at..end` runs past the end of the buffer (or overflows).
    RangeOutOfBounds { at: usize, end: usize, len: usize },
}

impl fmt::Display for BufferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BufferError::OutOfBounds { at, len } => {
                write!(f, "index {at} out of bounds for buffer of length {len}")
            }
            BufferError::RangeOutOfBounds { at, end, len } => {
                write!(
                    f,
                    "range {at}..{end} out of bounds for buffer of length {len}"
                )
            }
        }
    }
}

impl std::error::Error for BufferError {}

impl Buffer {
    pub fn new() -> Self {
        Self {
            buffer: ropey::Rope::new(),
        }
    }

    pub fn replace(&mut self, range: Range<usize>, text: &str) -> Result<usize, BufferError> {
        let len = self.buffer.len_chars();

        if range.start > range.end || range.end > len {
            return Err(BufferError::RangeOutOfBounds {
                at: range.start,
                end: range.end,
                len,
            });
        }
        self.buffer.remove(range);
        self.insert(range.start, text)
    }

    /// Inserts `text` at char index `at`. Returns the number of chars added.
    pub fn insert(&mut self, at: usize, text: &str) -> Result<usize, BufferError> {
        let len = self.buffer.len_chars();
        if at > len {
            return Err(BufferError::OutOfBounds { at, len });
        }
        self.buffer.insert(at, text);
        Ok(text.chars().count())
    }

    /// Reads `len` chars starting at char index `at`.
    pub fn read(&self, at: usize, len: usize) -> Result<Slice<'_>, BufferError> {
        let buf_len = self.buffer.len_chars();
        let end = at.saturating_add(len);
        if end > buf_len {
            return Err(BufferError::RangeOutOfBounds {
                at,
                end,
                len: buf_len,
            });
        }
        Ok(Slice {
            text: self.buffer.slice(at..end),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_returns_chars_added() {
        let mut b = Buffer::new();
        assert_eq!(b.insert(0, "héllo"), Ok(5));
        assert_eq!(b.insert(5, " 👍"), Ok(2));
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
        assert_eq!(s.text, "world");
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
}
