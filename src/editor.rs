//! editor
//!
//! A walking skeleton of the editor: the text, the selection and marked
//! text, and edits to them, with a [`Display`] of the text set into lines
//! for the answers about where things are. Ported from tolege's
//! editing-on-Idem proof of concept (tolege#216) so a host app can type in
//! it while the real pieces are built.
//!
//! Offsets are UTF-8 byte offsets into the whole text, as in
//! [`Buffer`]; lengths are scaled points, as in [`layout`](crate::layout).
//! [`Editor::to_utf16`] and [`Editor::from_utf16`] are for bridges to
//! platforms that count in UTF-16.
//!
//! What's rough, and where the real thing is meant to come from (and see
//! [`display`](crate::display)'s list):
//! - Every edit copies the buffer out to a `String` and hands the whole
//!   text to the display again.
//! - No undo.
//! - Line ends are `\n` only: `\r\n` and `\r` become `\n` on the way in.

use std::range::Range;
use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation;

use crate::buffer::{Buffer, Edge, Unit};
pub use crate::display::{Affinity, Display, Line, Place, Rect, Stop, StopIndex};
use crate::layout::Font;

pub struct Editor {
    buffer: Buffer,
    /// The buffer's text, copied out at the last layout.
    flat: String,
    /// For every byte offset that starts a character (and the end), its
    /// UTF-16 offset.
    utf16_of_byte: Vec<u32>,
    /// For every UTF-16 offset (and the end), the byte offset of the
    /// character it's in.
    byte_of_utf16: Vec<usize>,
    display: Display,
    selection: Range<usize>,
    /// Which of its offset's places the cursor is at, while the selection
    /// is empty.
    affinity: Affinity,
    marked: Option<Range<usize>>,
    /// Every grapheme cluster boundary in the text, in order.
    boundaries: Vec<usize>,
}

impl Editor {
    /// An editor for `text`, set in `font` (and `fallback` for what it
    /// lacks) at `size` and `line_height`, `width` wide, all in scaled
    /// points.
    pub fn new(
        text: &str,
        font: Arc<Font>,
        fallback: Vec<Arc<Font>>,
        size: i64,
        line_height: i64,
        width: i64,
    ) -> Self {
        let mut editor = Editor {
            buffer: Buffer::new(),
            flat: String::new(),
            utf16_of_byte: Vec::new(),
            byte_of_utf16: Vec::new(),
            display: Display::new(font, fallback, size, line_height, width),
            selection: Range { start: 0, end: 0 },
            affinity: Affinity::Downstream,
            marked: None,
            boundaries: Vec::new(),
        };
        editor.splice(Range { start: 0, end: 0 }, text);
        editor
    }

    // pub fn measure() -> Measure {
    //     let m = Measure:
    //
    // }

    // The text

    pub fn text(&self) -> &str {
        &self.flat
    }

    /// The text in `range` (clamped to the text and to whole characters).
    pub fn text_in(&self, range: Range<usize>) -> &str {
        let r = self.clamp(range);
        &self.flat[r.start..r.end]
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.flat.len()
    }

    pub fn is_empty(&self) -> bool {
        self.flat.is_empty()
    }

    /// Length in UTF-16 code units.
    pub fn len_utf16(&self) -> usize {
        self.byte_of_utf16.len() - 1
    }

    /// Replace everything, as when the text changed somewhere else. The
    /// cursor stays where it was if it can, and marked text is forgotten.
    pub fn set_text(&mut self, text: &str) {
        let selection = self.selection;
        // Not a retyping of the whole text: what's the same keeps its
        // place in the buffer's history, and so its isolates.
        self.buffer.reconcile(&line_ends(text));
        self.layout();
        self.marked = None;
        self.set_selection(selection);
    }

    /// Replace `range` with `text`; the cursor goes after it, and any
    /// marked text is forgotten.
    pub fn replace(&mut self, range: Range<usize>, text: &str) {
        let (start, added) = self.splice(range, text);
        self.selection = Range {
            start: start + added,
            end: start + added,
        };
        self.marked = None;
    }

    /// Delete the grapheme cluster before the cursor, or the selection.
    pub fn delete_backward(&mut self) {
        let sel = self.selection;
        let range = if sel.start != sel.end {
            sel
        } else if sel.start == 0 {
            return;
        } else {
            Range {
                start: self.boundary_before(sel.start),
                end: sel.start,
            }
        };
        let (start, _) = self.splice(range, "");
        self.selection = Range { start, end: start };
        self.marked = None;
    }

    // Selection and marked text

    pub fn selection(&self) -> Range<usize> {
        self.selection
    }

    /// Where the cursor is: the selection's end, and while the selection
    /// is empty, which of that offset's places.
    pub fn caret(&self) -> Place {
        Place { offset: self.selection.end, affinity: self.affinity }
    }

    /// Put the cursor at `at` (moved back to a whole grapheme cluster),
    /// selecting nothing.
    pub fn set_caret(&mut self, at: Place) {
        self.set_selection(Range { start: at.offset, end: at.offset });
        self.affinity = at.affinity;
    }

    /// Select `range`, widened to whole grapheme clusters: a selection never
    /// starts or ends inside one. The cursor is downstream; [`set_caret`]
    /// says otherwise.
    ///
    /// [`set_caret`]: Editor::set_caret
    pub fn set_selection(&mut self, range: Range<usize>) {
        self.affinity = Affinity::Downstream;
        let len = self.len();
        let (a, b) = (
            range.start.min(range.end).min(len),
            range.start.max(range.end).min(len),
        );
        let start = self.boundary_at_or_before(a);
        let end = if a == b {
            start
        } else {
            self.boundary_at_or_after(b)
        };
        self.selection = Range { start, end };
    }

    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked
    }

    /// What an input method that composes is in the middle of: `text`
    /// replaces the marked text (or the selection, if none), is marked, and
    /// `selected` (byte offsets within `text`) is selected.
    pub fn set_marked_text(&mut self, text: &str, selected: Range<usize>) {
        let range = self.marked.unwrap_or(self.selection);
        let (start, added) = self.splice(range, text);
        self.marked = (added > 0).then_some(Range {
            start,
            end: start + added,
        });
        let inside = |at: usize| {
            let mut at = start + at.min(added);
            while !self.flat.is_char_boundary(at) {
                at -= 1;
            }
            at
        };
        self.selection = Range {
            start: inside(selected.start),
            end: inside(selected.end),
        };
    }

    /// Keep the marked text as it is, unmarked.
    pub fn unmark_text(&mut self) {
        if let Some(m) = self.marked.take() {
            self.selection = Range {
                start: m.end,
                end: m.end,
            };
        }
    }

    // Moving

    /// The grapheme boundary `count` clusters away from `offset` (negative
    /// is back), clamped to the text.
    pub fn offset_by_clusters(&self, offset: usize, count: isize) -> usize {
        let i = self.boundaries.partition_point(|&b| b < offset) as isize;
        let j = (i + count).clamp(0, self.boundaries.len() as isize - 1);
        self.boundaries[j as usize]
    }

    /// The `edge` of a `unit` `count` away from `offset` (negative is
    /// back), stopping at the text's ends: option-arrow is
    /// `(Unit::Word, Edge::End, 1)` forward and `(Unit::Word, Edge::Start,
    /// -1)` back. Lands on a grapheme boundary, rounding the way it moves.
    pub fn offset_by_units(&self, offset: usize, unit: Unit, edge: Edge, count: isize) -> usize {
        let mut at = self
            .clamp(Range {
                start: offset,
                end: offset,
            })
            .start;
        for _ in 0..count.unsigned_abs() {
            let next = if count > 0 {
                self.buffer.next_boundary(at, unit, edge)
            } else {
                self.buffer.prev_boundary(at, unit, edge)
            }
            .expect("clamped to the text, on a char boundary");
            match next {
                Some(b) => at = b,
                None => {
                    at = if count > 0 { self.len() } else { 0 };
                    break;
                }
            }
        }
        if count > 0 {
            self.boundary_at_or_after(at)
        } else {
            self.boundary_at_or_before(at)
        }
    }

    /// The `unit` holding `offset`, widened to whole grapheme clusters: the
    /// word a double click selects, say. `None` between units.
    pub fn unit_range_at(&self, offset: usize, unit: Unit) -> Option<Range<usize>> {
        let at = self
            .clamp(Range {
                start: offset,
                end: offset,
            })
            .start;
        let r = self
            .buffer
            .unit_range_at(at, unit)
            .expect("clamped to the text, on a char boundary")?;
        Some(Range {
            start: self.boundary_at_or_before(r.start),
            end: self.boundary_at_or_after(r.end),
        })
    }

    /// The place `lines` lines up (negative) or down from `at`, nearest
    /// `goal_x` (or the cursor's own x). `None` past the first or last line.
    pub fn place_by_lines(&self, at: Place, lines: isize, goal_x: Option<i64>) -> Option<Place> {
        self.display.place_by_lines(at, lines, goal_x)
    }

    // Geometry, from the display

    /// The text as set, and where everything in it is.
    pub fn display(&self) -> &Display {
        &self.display
    }

    /// Where the cursor stands at `at`; see [`Display::caret_rect`].
    pub fn caret_rect(&self, at: Place) -> Rect {
        self.display.caret_rect(at)
    }

    /// The place under (`x`, `y`); see [`Display::closest_place`].
    pub fn closest_place(&self, x: i64, y: i64) -> Place {
        self.display.closest_place(x, y)
    }

    /// The rectangles covering `range`; see [`Display::selection_rects`].
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        self.display.selection_rects(range)
    }

    /// The text as set. Never empty: an empty text is one empty line.
    pub fn lines(&self) -> &[Line] {
        self.display.lines()
    }

    /// The lines' heights added up.
    pub fn height(&self) -> i64 {
        self.display.height()
    }

    pub fn width(&self) -> i64 {
        self.display.width()
    }

    /// Set the text again `width` scaled points wide.
    pub fn set_width(&mut self, width: i64) {
        self.display.set_width(width, &self.flat);
    }

    // UTF-16, for platforms that count in it

    /// The UTF-16 offset of byte `offset` (clamped, and moved back to the
    /// start of its character).
    pub fn to_utf16(&self, offset: usize) -> usize {
        let mut at = offset.min(self.len());
        while !self.flat.is_char_boundary(at) {
            at -= 1;
        }
        self.utf16_of_byte[at] as usize
    }

    /// The byte offset of UTF-16 offset `units` (clamped, and moved back
    /// off the middle of a surrogate pair).
    pub fn from_utf16(&self, units: usize) -> usize {
        self.byte_of_utf16[units.min(self.len_utf16())]
    }

    // Inside

    /// `range`, put in order, clamped to the text and moved back to whole
    /// characters.
    fn clamp(&self, range: Range<usize>) -> Range<usize> {
        let len = self.len();
        let floor = |at: usize| {
            let mut at = at.min(len);
            while !self.flat.is_char_boundary(at) {
                at -= 1;
            }
            at
        };
        let (a, b) = (floor(range.start), floor(range.end));
        Range {
            start: a.min(b),
            end: a.max(b),
        }
    }

    /// Put `text` in place of `range` in the buffer and set the text again.
    /// Returns where it went and how many bytes it came to.
    fn splice(&mut self, range: Range<usize>, text: &str) -> (usize, usize) {
        let range = self.clamp(range);
        let text = line_ends(text);
        let added = self
            .buffer
            .replace(range, &text)
            .expect("the range is clamped to the buffer and to whole characters");
        self.layout();
        // What was typed is before the cursor; what comes next goes after.
        self.affinity = Affinity::Downstream;
        (range.start, added)
    }

    fn boundary_at_or_before(&self, offset: usize) -> usize {
        let i = self.boundaries.partition_point(|&b| b <= offset);
        self.boundaries[i.saturating_sub(1)]
    }

    fn boundary_at_or_after(&self, offset: usize) -> usize {
        let i = self.boundaries.partition_point(|&b| b < offset);
        self.boundaries[i.min(self.boundaries.len() - 1)]
    }

    fn boundary_before(&self, offset: usize) -> usize {
        let i = self.boundaries.partition_point(|&b| b < offset);
        self.boundaries[i.saturating_sub(1)]
    }

    /// Copy the buffer out, find its grapheme boundaries, and hand it to
    /// the display to set.
    fn layout(&mut self) {
        self.flat = self
            .buffer
            .read(0, self.buffer.len())
            .expect("the whole buffer is in bounds")
            .text
            .to_string();
        self.index_utf16();
        self.boundaries = self
            .flat
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(self.flat.len()))
            .collect();
        self.display.set(&self.flat);
    }

    fn index_utf16(&mut self) {
        let text = &self.flat;
        let mut of_byte = vec![0u32; text.len() + 1];
        let mut of_unit = Vec::with_capacity(text.len() + 1);
        let mut units = 0u32;
        for (i, c) in text.char_indices() {
            of_byte[i] = units;
            for _ in 0..c.len_utf16() {
                of_unit.push(i);
            }
            units += c.len_utf16() as u32;
        }
        of_byte[text.len()] = units;
        of_unit.push(text.len());
        self.utf16_of_byte = of_byte;
        self.byte_of_utf16 = of_unit;
    }
}

/// `text` with every line end a `\n`.
fn line_ends(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n").into()
    } else {
        text.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::PT;

    fn editor(text: &str, width: i64) -> Editor {
        let inter =
            Font::from_bytes(include_bytes!("../fonts/Inter-Regular.ttf").to_vec()).unwrap();
        Editor::new(text, Arc::new(inter), vec![], 17 * PT, 24 * PT, width * PT)
    }

    fn r(start: usize, end: usize) -> Range<usize> {
        Range { start, end }
    }

    #[test]
    fn typing_and_deleting_by_cluster() {
        let mut e = editor("", 300);
        // e + accent: 3 bytes; the emoji: 4; the family: 18.
        e.replace(r(0, 0), "e\u{301}😀👨‍👩‍👧");
        assert_eq!(e.selection(), r(25, 25));
        e.delete_backward(); // the family, all of it
        assert_eq!(e.text(), "e\u{301}😀");
        e.delete_backward(); // the emoji
        e.delete_backward(); // e and its accent
        assert_eq!(e.text(), "");
        e.delete_backward();
        assert_eq!(e.text(), "");
    }

    #[test]
    fn selections_widen_to_whole_clusters() {
        // The family is bytes 1..19; the girl starts at 15.
        let mut e = editor("a👨‍👩‍👧b", 300);
        e.set_selection(r(15, 19));
        assert_eq!(e.selection(), r(1, 19));
        e.delete_backward();
        assert_eq!(e.text(), "ab");
        e.set_selection(r(1, 1));
        assert_eq!(e.selection(), r(1, 1));
    }

    #[test]
    fn marked_text_composes_and_commits() {
        let mut e = editor("ab", 300);
        e.set_selection(r(1, 1));
        e.set_marked_text("k", r(1, 1));
        e.set_marked_text("か", r(3, 3));
        e.set_marked_text("漢字", r(6, 6));
        assert_eq!(e.text(), "a漢字b");
        assert_eq!(e.marked(), Some(r(1, 7)));
        e.unmark_text();
        assert_eq!((e.marked(), e.selection()), (None, r(7, 7)));
    }

    #[test]
    fn utf16_offsets_both_ways() {
        // a: 1 byte, 1 unit; é: 2 bytes, 1 unit; 😀: 4 bytes, 2 units.
        let e = editor("aé😀b", 300);
        assert_eq!([0, 1, 3, 7, 8].map(|b| e.to_utf16(b)), [0, 1, 2, 4, 5]);
        assert_eq!([0, 1, 2, 4, 5].map(|u| e.from_utf16(u)), [0, 1, 3, 7, 8]);
        // Inside the emoji, either way, is its start.
        assert_eq!(e.from_utf16(3), 3);
        assert_eq!(e.to_utf16(5), 2);
        assert_eq!((e.len_utf16(), e.from_utf16(99), e.to_utf16(99)), (5, 8, 5));
    }

    #[test]
    fn set_text_keeps_the_cursor_in_bounds() {
        let mut e = editor("hello world", 300);
        e.set_selection(r(11, 11));
        e.set_text("hi");
        assert_eq!((e.text(), e.selection()), ("hi", r(2, 2)));
        e.set_text("one\r\ntwo\rthree");
        assert_eq!(e.text(), "one\ntwo\nthree");
        assert_eq!(e.lines().len(), 3);
    }

    #[test]
    fn moves_by_words() {
        let e = editor("one two  three", 300);
        assert_eq!(e.offset_by_units(0, Unit::Word, Edge::End, 1), 3);
        assert_eq!(e.offset_by_units(0, Unit::Word, Edge::End, 2), 7);
        assert_eq!(e.offset_by_units(0, Unit::Word, Edge::End, 9), e.len());
        assert_eq!(e.offset_by_units(10, Unit::Word, Edge::Start, -1), 9);
        assert_eq!(e.offset_by_units(10, Unit::Word, Edge::Start, -2), 4);
        assert_eq!(e.offset_by_units(10, Unit::Word, Edge::Start, -9), 0);
        assert_eq!(e.offset_by_units(5, Unit::Word, Edge::End, 0), 5);
    }

    #[test]
    fn unit_moves_land_on_clusters() {
        // Vim's word ends between the e and its combining accent; the
        // cursor can't, so it goes on past the cluster.
        let e = editor("e\u{301}x y", 300);
        assert_eq!(e.offset_by_units(0, Unit::VimWord, Edge::End, 1), 3);
        assert_eq!(
            e.unit_range_at(0, Unit::VimWord),
            Some(Range { start: 0, end: 3 })
        );
    }

    #[test]
    fn selects_a_word_or_a_paragraph() {
        let e = editor("first line\nstill first\n\nsecond", 300);
        assert_eq!(
            e.unit_range_at(2, Unit::Word),
            Some(Range { start: 0, end: 5 })
        );
        assert_eq!(e.unit_range_at(5, Unit::Word), None);
        assert_eq!(
            e.unit_range_at(13, Unit::Paragraph).map(|r| r.start),
            Some(0)
        );
        assert_eq!(e.offset_by_units(0, Unit::Paragraph, Edge::Start, 1), 24);
    }

    #[test]
    fn its_clusters_are_the_displays_stops() {
        // The editor finds boundaries in the whole text, the display a
        // paragraph at a time; a cluster never spans a line end, so they
        // agree.
        let e = editor("naïve e\u{301} 😀 office\n\u{301}x\n\nlast 👨‍👩‍👧", 80);
        let mut stops: Vec<usize> = e
            .lines()
            .iter()
            .flat_map(|l| l.stops.iter().map(|s| s.offset))
            .collect();
        // An offset with two places is two stops.
        stops.dedup();
        assert_eq!(stops, e.boundaries);
    }
}
