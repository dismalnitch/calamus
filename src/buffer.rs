//! buffer
//!
//! The text being edited: a mamre [`Text`], so a rope to read, and a
//! history that keeps what the rope can't say. Positions are UTF-8 byte
//! offsets, as for `str`, and must fall on `char` boundaries; one that
//! doesn't is a [`BufferError`], not a panic.
//!
//! What the history keeps for now is the text's isolates: the stretches
//! set in a direction of their own (see [`Buffer::isolates`]). They're
//! markers in the history, not characters, so nothing here that reads or
//! counts the text sees them.
use std::{fmt, range::Range};

use mamre_rope::Rope;
pub use mamre_rope::{Edge, Unit};
use mamre_text::Text;
pub use mamre_text::{Direction, Isolate, IsolateId, Kind, Paragraph, Section, SectionId};

use crate::units::{Heading, Offset};

pub struct Buffer {
    text: Text,
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
    /// There's no such isolate, or it doesn't reach where it was asked
    /// to.
    NoSuchIsolate,
    /// The range takes part of an isolate and not the rest.
    CrossesIsolate,
    /// There's no such section now: unmarked, or its first character
    /// deleted.
    NoSuchSection,
    /// The paragraph has no line break before it to carry a kind: it's
    /// the text before the first one.
    NoBreak,
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
            BufferError::NoSuchIsolate => write!(f, "no such isolate there"),
            BufferError::CrossesIsolate => write!(f, "the range takes part of an isolate"),
            BufferError::NoSuchSection => write!(f, "no such section"),
            BufferError::NoBreak => write!(f, "the paragraph has no line break before it"),
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
    /// An empty buffer. Its history is written as one writer's, named
    /// "calamus": right for text no one else edits. Text shared between
    /// replicas needs [`Buffer::with_writer`].
    pub fn new() -> Self {
        Self::with_writer("calamus")
    }

    /// An empty buffer edited by `writer`: a name no other replica of the
    /// same text uses (a user on a device, say).
    pub fn with_writer(writer: &str) -> Self {
        Self { text: Text::new(writer) }
    }

    /// The buffer a history holds: its text, and everything the history
    /// keeps beside it, edited from now on by `writer`. `None` if the
    /// bytes aren't a history ([`Buffer::history`] wrote).
    pub fn open(writer: &str, history: &[u8]) -> Option<Self> {
        Text::open(writer, history).ok().map(|text| Self { text })
    }

    /// The buffer's whole history, to keep: the text, every edit that
    /// made it, and its isolates. [`Buffer::open`] reads it back.
    pub fn history(&mut self) -> Vec<u8> {
        let mut file = Vec::new();
        self.text.save(&mut file);
        file
    }

    /// The text, to read.
    fn rope(&self) -> &Rope {
        self.text.rope()
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.rope().len()
    }

    pub fn is_empty(&self) -> bool {
        self.rope().is_empty()
    }

    pub fn prev_grapheme_boundary(&self, offset: usize) -> Option<Offset> {
        let position: Option<usize> = self.rope().prev_grapheme_boundary(offset);
        {
            let position = position?;
            Some(Offset {
                offset: position,
                heading: Heading::East,
            })
        }
    }

    pub fn next_grapheme_boundary(&self, offset: usize) -> Option<Offset> {
        let position: Option<usize> = self.rope().next_grapheme_boundary(offset);
        {
            let position = position?;
            Some(Offset {
                offset: position,
                heading: Heading::East,
            })
        }
    }

    /// The first `edge` of a `unit` after byte `at`: where the next word
    /// starts, say. `None` if there's none before the end.
    ///
    /// "After" is a higher offset; which way that is on the page is the
    /// layout's business. Words and sentences are found by scanning near
    /// `at`; so are paragraphs and sections, as the rope counts nothing.
    pub fn next_boundary(&self, at: usize, unit: Unit, edge: Edge) -> Result<Option<usize>, BufferError> {
        self.check_offset(at)?;
        Ok(self.rope().next_boundary(at, unit, edge))
    }

    /// The last `edge` of a `unit` before byte `at`: where the word the
    /// cursor is in (or the one before) starts, say.
    pub fn prev_boundary(&self, at: usize, unit: Unit, edge: Edge) -> Result<Option<usize>, BufferError> {
        self.check_offset(at)?;
        Ok(self.rope().prev_boundary(at, unit, edge))
    }

    /// The `unit` holding byte `at`, start to just past its end; `None`
    /// between units (on a space between words, say) and at the end.
    ///
    /// The rope's `unit_at` finds units by number, which needs a measure
    /// that counts them; this one counts nothing, so this goes by
    /// neighbours: the last start at or before `at`, then the end after it.
    pub fn unit_range_at(&self, at: usize, unit: Unit) -> Result<Option<Range<usize>>, BufferError> {
        self.check_offset(at)?;
        if at == self.len() {
            return Ok(None);
        }
        let next_char = self.rope().char_to_offset(self.rope().offset_to_char(at) + 1);
        let Some(start) = self.rope().prev_boundary(next_char, unit, Edge::Start) else {
            return Ok(None);
        };
        Ok(self
            .rope()
            .next_boundary(start, unit, Edge::End)
            .filter(|&end| end > at)
            .map(|end| Range { start, end }))
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
        self.text
            .replace(range.start..range.end, text)
            .expect("the range was checked");
        Ok(text.len())
    }

    /// Inserts `text` at byte `at`. Returns the number of bytes added.
    pub fn insert(&mut self, at: usize, text: &str) -> Result<usize, BufferError> {
        let len = self.len();
        if at > len {
            return Err(BufferError::OutOfBounds { at, len });
        }
        self.check_boundary(at)?;
        self.text.insert(at, text).expect("the offset was checked");
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
            text: self.rope().slice(at..end),
        })
    }

    /// Makes the text what `text` is, changed somewhere that isn't this
    /// buffer. What's the same keeps its place in the history, so its
    /// isolates; the rest is edits like any other.
    pub fn reconcile(&mut self, text: &str) {
        self.text.reconcile(text);
    }

    /// The stretches set in a direction of their own, in order of where
    /// they start, each before the ones nested in it.
    pub fn isolates(&self) -> Vec<Isolate> {
        self.text.isolates()
    }

    /// Makes the text in `range` an isolate set in `direction`; on an
    /// isolate's own range, sets its direction.
    pub fn mark_isolate(&mut self, range: Range<usize>, direction: Direction) -> Result<IsolateId, BufferError> {
        self.check_range(&range)?;
        match self.text.mark_isolate(range.start..range.end, direction) {
            Ok((id, _)) => Ok(id),
            Err(mamre_text::TextError::CrossesIsolate) => Err(BufferError::CrossesIsolate),
            Err(_) => Err(BufferError::RangeOutOfBounds { at: range.start, end: range.end, len: self.len() }),
        }
    }

    /// Makes isolate `id` no isolate: its text stays.
    pub fn remove_isolate(&mut self, id: IsolateId) -> Result<(), BufferError> {
        self.text.remove_isolate(id).map(|_| ()).map_err(|_| BufferError::NoSuchIsolate)
    }

    /// Inserts `text` at byte `at`, inside isolate `id`: at its very start
    /// or end, where [`Buffer::insert`] would put it outside. Returns the
    /// number of bytes added.
    pub fn insert_in(&mut self, at: usize, text: &str, id: IsolateId) -> Result<usize, BufferError> {
        self.check_offset(at)?;
        self.text.insert_in(at, text, id).map(|_| text.len()).map_err(|_| BufferError::NoSuchIsolate)
    }

    /// Every paragraph, in order, with what it is. A paragraph's kind is
    /// a mark on the line break that starts it, so the text before the
    /// first break is always plain.
    pub fn paragraphs(&self) -> Vec<Paragraph> {
        self.text.paragraphs()
    }

    /// What the paragraph a cursor at byte `at` is in is.
    pub fn kind_at(&self, at: usize) -> Kind {
        self.text.kind_at(at)
    }

    /// Makes the paragraph a cursor at byte `at` is in a `kind`.
    pub fn set_kind(&mut self, at: usize, kind: Kind) -> Result<(), BufferError> {
        self.check_offset(at)?;
        self.text.set_kind(at, kind).map(|_| ()).map_err(|_| BufferError::NoBreak)
    }

    /// The sections the text shows, in order: each a mark on the
    /// character that starts it, with a level (1 the outermost).
    pub fn sections(&self) -> Vec<Section> {
        self.text.sections()
    }

    /// Makes the character at byte `at` start a section at `level` (0: no
    /// section). Its id is the same through every edit, move and merge.
    pub fn mark_section(&mut self, at: usize, level: u8) -> Result<SectionId, BufferError> {
        let len = self.len();
        if at >= len {
            return Err(BufferError::OutOfBounds { at, len });
        }
        self.check_boundary(at)?;
        self.text.mark_section(at, level).map(|(id, _)| id).map_err(|_| BufferError::OutOfBounds { at, len })
    }

    /// Where section `id` runs: from its first character to the next
    /// section at its level or above, or the end.
    pub fn section_range(&self, id: SectionId) -> Option<Range<usize>> {
        self.text.section_range(id).map(|r| Range { start: r.start, end: r.end })
    }

    /// Moves section `id`, subsections and all, to byte `to` (an offset
    /// before the move). To inside itself, or where it is, does nothing.
    pub fn move_section(&mut self, id: SectionId, to: usize) -> Result<(), BufferError> {
        let range = self.section_range(id).ok_or(BufferError::NoSuchSection)?;
        self.move_text(range, to)
    }

    /// Moves the text in `range` to byte `to` (an offset before the
    /// move): the same characters somewhere else, so sections and
    /// isolates in them go along.
    pub fn move_text(&mut self, range: Range<usize>, to: usize) -> Result<(), BufferError> {
        self.check_range(&range)?;
        self.check_offset(to)?;
        self.text
            .move_text(range.start..range.end, to)
            .map(|_| ())
            .map_err(|_| BufferError::RangeOutOfBounds { at: range.start, end: range.end, len: self.len() })
    }

    fn check_range(&self, range: &Range<usize>) -> Result<(), BufferError> {
        let len = self.len();
        if range.start > range.end || range.end > len {
            return Err(BufferError::RangeOutOfBounds { at: range.start, end: range.end, len });
        }
        self.check_boundary(range.start)?;
        self.check_boundary(range.end)
    }

    /// Errors unless `at` is in bounds (the end counts) and a char
    /// boundary.
    fn check_offset(&self, at: usize) -> Result<(), BufferError> {
        let len = self.len();
        if at > len {
            return Err(BufferError::OutOfBounds { at, len });
        }
        self.check_boundary(at)
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
        let (mut lo, mut hi) = (0, self.rope().summary().chars);
        // The first char whose offset is at or past `at`.
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.rope().char_to_offset(mid) < at {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        self.rope().char_to_offset(lo) == at
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

    #[test]
    fn word_boundaries() {
        let mut b = Buffer::new();
        b.insert(0, "hello  wide world").unwrap();
        assert_eq!(b.next_boundary(0, Unit::Word, Edge::End), Ok(Some(5)));
        assert_eq!(b.next_boundary(5, Unit::Word, Edge::Start), Ok(Some(7)));
        assert_eq!(b.prev_boundary(9, Unit::Word, Edge::Start), Ok(Some(7)));
        assert_eq!(b.prev_boundary(0, Unit::Word, Edge::Start), Ok(None));
        assert_eq!(b.next_boundary(12, Unit::Word, Edge::Start), Ok(None));
    }

    #[test]
    fn unit_range_at_a_word_and_between() {
        let mut b = Buffer::new();
        b.insert(0, "hello  wide world").unwrap();
        assert_eq!(b.unit_range_at(7, Unit::Word), Ok(Some(Range { start: 7, end: 11 })));
        assert_eq!(b.unit_range_at(10, Unit::Word), Ok(Some(Range { start: 7, end: 11 })));
        assert_eq!(b.unit_range_at(5, Unit::Word), Ok(None));
        assert_eq!(b.unit_range_at(b.len(), Unit::Word), Ok(None));
    }

    #[test]
    fn isolates_are_kept_and_arent_text() {
        // אבג is bytes 4..10.
        let mut b = Buffer::new();
        b.insert(0, "abc אבג def").unwrap();
        let id = b.mark_isolate(Range { start: 4, end: 10 }, Direction::RightToLeft).unwrap();
        assert_eq!(b.len(), 14);
        assert_eq!(b.read(0, 14).unwrap().text.to_string(), "abc אבג def");
        // At its end: outside, unless it's put in.
        b.insert(10, "X").unwrap();
        b.insert_in(10, "ד", id).unwrap();
        let all = b.isolates();
        assert_eq!((all.len(), all[0].range.clone()), (1, 4..12));
        assert_eq!(b.unit_range_at(6, Unit::Word), Ok(Some(Range { start: 4, end: 13 })));
        // Text changed elsewhere keeps the isolates of what's the same.
        b.reconcile("so abc אבגדX def.");
        assert_eq!(b.isolates()[0].id, id);
        assert_eq!(b.isolates()[0].range, 7..15);
        assert_eq!(
            b.mark_isolate(Range { start: 5, end: 9 }, Direction::LeftToRight),
            Err(BufferError::CrossesIsolate)
        );
        assert_eq!(b.remove_isolate(id), Ok(()));
        assert_eq!(b.isolates(), []);
        assert_eq!(b.remove_isolate(id), Err(BufferError::NoSuchIsolate));
    }

    #[test]
    fn a_history_brings_everything_back() {
        let mut b = Buffer::with_writer("me");
        b.insert(0, "abc אבג def").unwrap();
        let id = b.mark_isolate(Range { start: 4, end: 10 }, Direction::RightToLeft).unwrap();
        b.insert_in(10, " x", id).unwrap();
        let kept = b.history();
        // The same writer carries on where they left off.
        let mut back = Buffer::open("me", &kept).unwrap();
        assert_eq!(back.read(0, back.len()).unwrap().text.to_string(), "abc אבג x def");
        assert_eq!(back.isolates(), b.isolates());
        back.insert_in(12, "y", id).unwrap();
        assert_eq!(back.isolates()[0].range, 4..13);
        // And what they add is kept in turn.
        let again = Buffer::open("me", &back.history()).unwrap();
        assert_eq!(again.isolates(), back.isolates());
        assert_eq!(again.len(), back.len());
        // Bytes that aren't a history are nothing.
        assert!(Buffer::open("me", b"abc").is_none());
        assert!(Buffer::open("me", b"").is_none());
    }

    #[test]
    fn hebrew_words() {
        // Two bytes a letter: שלום is 0..8, the space 8, עולם 9..17.
        let mut b = Buffer::new();
        b.insert(0, "שלום עולם").unwrap();
        assert_eq!(b.unit_range_at(2, Unit::Word), Ok(Some(Range { start: 0, end: 8 })));
        assert_eq!(b.next_boundary(0, Unit::Word, Edge::Start), Ok(Some(9)));
        assert_eq!(b.next_boundary(1, Unit::Word, Edge::Start), Err(BufferError::NotCharBoundary { at: 1 }));
    }
}
