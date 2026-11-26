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

use crate::bidi::{self, Stretch};
use crate::buffer::{Buffer, Edge, Isolate, Unit};
pub use crate::buffer::Direction;
pub use crate::display::{Affinity, Display, Line, Place, Rect, Stop, StopIndex};
use crate::layout::Font;

/// The side of its offset the cursor is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Side {
    /// Which way what's typed here will run: the isolate's the cursor is
    /// in, or the paragraph's.
    pub direction: Direction,
    /// Whether the offset has another side: the edge of an isolate, or
    /// two places on the page.
    pub choice: bool,
    /// Whether the writer chose this side, so typing won't leave it.
    pub held: bool,
}

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
    /// Whether the writer chose the cursor's side ([`Editor::flip_side`]):
    /// then what's typed stays on it, whatever its script, until the
    /// cursor's moved.
    held: bool,
    /// Whether only the writer moves the cursor between sides
    /// ([`Editor::set_stays_put`]).
    stays_put: bool,
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
        let mut editor = Self::over(Buffer::new(), font, fallback, size, line_height, width);
        editor.splice(Range { start: 0, end: 0 }, text);
        editor
    }

    /// An editor for `text` whose history was kept ([`Editor::history`]):
    /// the text as it was left, isolates and all, edited from now on by
    /// `writer` (a name no other device editing the same text uses).
    ///
    /// The history is what's kept beside the text; `text` is the text
    /// itself, as whatever stores it has it. If they differ (it was
    /// edited somewhere this history never heard of), the history is
    /// brought to the text: what's the same keeps its isolates. With no
    /// history, or bytes that aren't one, it's a new editor for `text`.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        history: &[u8],
        writer: &str,
        text: &str,
        font: Arc<Font>,
        fallback: Vec<Arc<Font>>,
        size: i64,
        line_height: i64,
        width: i64,
    ) -> Self {
        let buffer = Buffer::open(writer, history).unwrap_or_else(|| Buffer::with_writer(writer));
        let mut editor = Self::over(buffer, font, fallback, size, line_height, width);
        editor.buffer.reconcile(&line_ends(text));
        editor.layout();
        editor
    }

    /// The editor's whole history, to keep beside its text: every edit,
    /// and the isolates. [`Editor::open`] takes it.
    pub fn history(&mut self) -> Vec<u8> {
        self.buffer.history()
    }

    fn over(
        buffer: Buffer,
        font: Arc<Font>,
        fallback: Vec<Arc<Font>>,
        size: i64,
        line_height: i64,
        width: i64,
    ) -> Self {
        Editor {
            buffer,
            flat: String::new(),
            utf16_of_byte: Vec::new(),
            byte_of_utf16: Vec::new(),
            display: Display::new(font, fallback, size, line_height, width),
            selection: Range { start: 0, end: 0 },
            affinity: Affinity::Downstream,
            held: false,
            stays_put: false,
            marked: None,
            boundaries: Vec::new(),
        }
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
        // Told where it already is, it's still where the writer put it.
        let held = self.held && self.selection.start == self.selection.end && at == self.caret();
        self.set_selection(Range { start: at.offset, end: at.offset });
        self.affinity = at.affinity;
        self.held = held;
    }

    /// How the cursor behaves at the edge of text that runs against
    /// what's round it.
    ///
    /// Off, the way most editors do it: a letter that runs the other way
    /// from the cursor's side goes to the other side (English typed at
    /// the end of a Hebrew phrase leaves the phrase), and a change of
    /// keyboard turns the cursor ([`Editor::face`]). The letter, or the
    /// cursor, jumps to the phrase's other end.
    ///
    /// On, the cursor stays where the writer put it: everything typed
    /// goes to its side, whatever its script, and nothing but the writer
    /// ([`Editor::flip_side`], an arrow key, a touch) takes it to the
    /// other. English typed at the end of a Hebrew phrase is in the
    /// phrase until they step out.
    pub fn set_stays_put(&mut self, on: bool) {
        self.stays_put = on;
    }

    pub fn stays_put(&self) -> bool {
        self.stays_put
    }

    /// The cursor's side, for showing the writer: which way what's typed
    /// next will run, and whether there's another side here to choose.
    pub fn caret_side(&self) -> Side {
        let at = self.selection.end;
        let here = self.home(at, self.affinity);
        let caret = self.selection.start == self.selection.end;
        Side {
            direction: self.runs(&here, at),
            choice: caret && (self.display.twin(self.caret()).is_some() || self.apart()),
            held: self.held,
        }
    }

    /// Whether the cursor's offset is the edge of an isolate: in it on one
    /// side, out of it on the other.
    fn apart(&self) -> bool {
        let at = self.selection.end;
        let (here, there) = (self.home(at, self.affinity), self.home(at, self.other_side()));
        here.map(|i| i.id) != there.map(|i| i.id)
    }

    /// The isolate that what's typed is being kept in, whatever its
    /// script: the cursor's, while the writer holds its side or the
    /// cursor stays put. For showing them, since English typed into
    /// Hebrew this way is in the Hebrew and looks it. `None` when typing
    /// goes where its script takes it, or the cursor's in no isolate.
    pub fn kept_in(&self) -> Option<Isolate> {
        if self.selection.start != self.selection.end || !(self.held || self.stays_put) {
            return None;
        }
        self.home(self.selection.end, self.affinity)
    }

    /// The way text typed at `at` would run.
    fn runs_at(&self, at: Place) -> Direction {
        self.runs(&self.home(at.offset, at.affinity), at.offset)
    }

    /// Puts the cursor on the other side of where it is, without moving
    /// it on the page, and holds it there: what's typed next stays on
    /// that side whatever its script, until the cursor's moved.
    ///
    /// At a change of direction two places share the cursor's spot: at
    /// the right of Hebrew that ends an English line, "before the
    /// Hebrew's first letter" and "after everything, where English goes
    /// on". This goes from one to the other. Where an isolate's edge
    /// isn't a change of direction, it goes in or out of the isolate.
    /// `false`, and nothing changes, where there's one side.
    pub fn flip_side(&mut self) -> bool {
        if self.selection.start != self.selection.end {
            return false;
        }
        if let Some(twin) = self.display.twin(self.caret()) {
            self.selection = Range { start: twin.offset, end: twin.offset };
            self.affinity = twin.affinity;
        } else if self.apart() {
            self.affinity = self.other_side();
        } else {
            return false;
        }
        self.held = true;
        true
    }

    /// The cursor has just been put somewhere (a touch), and the keyboard
    /// types text that runs `direction`: where two places share its spot,
    /// it takes the one that runs the keyboard's way. A touch at the
    /// right of Hebrew that ends an English line is "before the Hebrew"
    /// with a Hebrew keyboard, "after it" with an English one. Says
    /// whether it changed. Not if the writer is holding a side.
    pub fn settle(&mut self, direction: Direction) -> bool {
        if self.held || self.selection.start != self.selection.end {
            return false;
        }
        let here = self.caret();
        let Some(twin) = self.display.twin(here) else {
            return false;
        };
        if self.runs_at(here) == direction || self.runs_at(twin) != direction {
            return false;
        }
        self.selection = Range { start: twin.offset, end: twin.offset };
        self.affinity = twin.affinity;
        true
    }

    /// The writer is about to type text that runs `direction` (they've
    /// changed keyboard): if the cursor's other side runs that way and
    /// this one doesn't, it goes over, to where that text will show.
    /// That's the other end of the phrase it's at, as other editors have
    /// it. Not if they're holding the side, or the cursor stays put.
    /// Says whether it moved.
    pub fn face(&mut self, direction: Direction) -> bool {
        let at = self.selection.end;
        if self.held || self.stays_put || self.selection.start != self.selection.end || !self.apart() {
            return false;
        }
        let (here, there) = (self.home(at, self.affinity), self.home(at, self.other_side()));
        if self.runs(&here, at) == direction || self.runs(&there, at) != direction {
            return false;
        }
        self.affinity = self.other_side();
        true
    }

    fn other_side(&self) -> Affinity {
        match self.affinity {
            Affinity::Upstream => Affinity::Downstream,
            Affinity::Downstream => Affinity::Upstream,
        }
    }

    /// Select `range`, widened to whole grapheme clusters: a selection never
    /// starts or ends inside one. The cursor is downstream; [`set_caret`]
    /// says otherwise.
    ///
    /// [`set_caret`]: Editor::set_caret
    pub fn set_selection(&mut self, range: Range<usize>) {
        self.affinity = Affinity::Downstream;
        self.held = false;
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

    /// The place beside `at` on the page, to its right or left: what an
    /// arrow key moves to. See [`Display::place_beside`].
    pub fn place_beside(&self, at: Place, right: bool) -> Option<Place> {
        self.display.place_beside(at, right)
    }

    /// The stretches of the text set in a direction of their own.
    pub fn isolates(&self) -> Vec<Isolate> {
        self.buffer.isolates()
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
        self.display.set_width(width, &self.flat, &self.buffer.isolates());
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
        // Typed at the cursor, text goes to the cursor's side. Typed or
        // pasted over text, it goes where that text began: in the isolate
        // its first character was in, if any of that isolate is left.
        let typed = range.start == range.end
            && self.selection.start == self.selection.end
            && range.start == self.selection.end;
        let began = (range.start < range.end).then(|| self.home(range.start, Affinity::Downstream).map(|i| i.id));
        self.buffer
            .replace(range, "")
            .expect("the range is clamped to the buffer and to whole characters");
        let side = match began {
            None => typed.then_some(self.affinity),
            Some(began) => [Affinity::Upstream, Affinity::Downstream]
                .into_iter()
                .find(|&side| self.home(range.start, side).map(|i| i.id) == began),
        };
        let added = self.put(range.start, &text, side);
        self.layout();
        // The cursor stays with what was just typed: where its offset has
        // two places, the one that's the end of the text before it.
        self.affinity = Affinity::Upstream;
        (range.start, added)
    }

    /// The innermost isolate on `side` of byte `at`: the one holding the
    /// character before it (upstream) or after it (downstream).
    fn home(&self, at: usize, side: Affinity) -> Option<Isolate> {
        self.buffer
            .isolates()
            .into_iter()
            .filter(|i| match side {
                Affinity::Upstream => i.range.start < at && at <= i.range.end,
                Affinity::Downstream => i.range.start <= at && at < i.range.end,
            })
            .min_by_key(|i| i.range.len())
    }

    /// The way text in `home` runs: the isolate's, or with none, the way
    /// the paragraph at `at` does.
    fn runs(&self, home: &Option<Isolate>, at: usize) -> Direction {
        if let Some(isolate) = home {
            return isolate.direction;
        }
        let text = self
            .buffer
            .read(0, self.buffer.len())
            .expect("the whole buffer is in bounds")
            .text
            .to_string();
        let from = text[..at].rfind('\n').map_or(0, |i| i + 1);
        let to = text[at..].find('\n').map_or(text.len(), |i| at + i);
        if bidi::base(&text[from..to]) % 2 == 1 {
            Direction::RightToLeft
        } else {
            Direction::LeftToRight
        }
    }

    /// Puts `text` in at `at`. Returns the bytes added.
    ///
    /// Typed at the cursor (`side` is the cursor's), it goes to the
    /// cursor's side: at an isolate's edge, into the isolate or outside
    /// it, as the cursor is. So a space typed after a Hebrew word stays
    /// with the word, and the cursor stays where it was going.
    ///
    /// A strong letter chooses for itself, unless the writer is holding
    /// the side or the cursor stays put ([`Editor::set_stays_put`]): one
    /// that runs against the cursor's side and with the
    /// other goes to the other. English typed at the end of a Hebrew
    /// phrase leaves the phrase, and takes the spaces at its end along:
    /// they were typed on the way out.
    ///
    /// Put in anywhere else (`side` is `None`), a strong letter at the
    /// edge of an isolate that runs its way joins it, and anything else
    /// goes outside.
    fn put(&mut self, at: usize, text: &str, side: Option<Affinity>) -> usize {
        let done = "the offset is clamped to the buffer and to whole characters";
        let strong = bidi::first_strong(text);
        let Some(side) = side.filter(|_| !text.contains('\n')) else {
            let home = strong.filter(|_| !text.contains('\n')).and_then(|direction| {
                self.buffer
                    .isolates()
                    .into_iter()
                    .filter(|i| (i.range.start == at || i.range.end == at) && i.direction == direction)
                    .min_by_key(|i| i.range.len())
            });
            return match home {
                Some(isolate) => self.buffer.insert_in(at, text, isolate.id),
                None => self.buffer.insert(at, text),
            }
            .expect(done);
        };
        let other = match side {
            Affinity::Upstream => Affinity::Downstream,
            Affinity::Downstream => Affinity::Upstream,
        };
        let (here, there) = (self.home(at, side), self.home(at, other));
        let apart = here.as_ref().map(|i| i.id) != there.as_ref().map(|i| i.id);
        let leaves = strong.is_some_and(|d| {
            !self.held && !self.stays_put && apart && self.runs(&here, at) != d && self.runs(&there, at) == d
        });
        if !leaves {
            return match here {
                Some(isolate) => self.buffer.insert_in(at, text, isolate.id),
                None => self.buffer.insert(at, text),
            }
            .expect(done);
        }
        // Out of `here`, and the spaces at its end with it: taken out,
        // and put back on the far side with the text.
        let mut from = at;
        if let (Affinity::Upstream, Some(left)) = (side, &here) {
            if left.range.end == at {
                let inside = self
                    .buffer
                    .read(left.range.start, at - left.range.start)
                    .expect("an isolate is in the buffer")
                    .text
                    .to_string();
                let kept = inside.trim_end_matches(' ').len();
                // Not all of it: an isolate of nothing but spaces goes.
                if kept > 0 {
                    from = left.range.start + kept;
                }
            }
        }
        let spaces = " ".repeat(at - from);
        if from < at {
            self.buffer.replace(Range { start: from, end: at }, "").expect(done);
        }
        let all = format!("{spaces}{text}");
        // `there` may have gone with the spaces' isolate; look again.
        let there = if from < at { self.home(from, Affinity::Downstream) } else { there };
        match there.filter(|i| i.range.start <= from && from <= i.range.end) {
            Some(isolate) => self.buffer.insert_in(from, &all, isolate.id),
            None => self.buffer.insert(from, &all),
        }
        .expect(done);
        text.len()
    }

    /// Makes isolates of the text that has none and wants them: each
    /// paragraph's stretches that run against it, as Unicode's algorithm
    /// finds them (see [`bidi::propose`]). What's in an isolate already
    /// is left as it is.
    fn propose(&mut self) {
        let text = self
            .buffer
            .read(0, self.buffer.len())
            .expect("the whole buffer is in bounds")
            .text
            .to_string();
        // An isolate is one paragraph's. One with a line end in it (Return
        // pressed inside it) is made one for each line it's on.
        for isolate in self.buffer.isolates() {
            let inside = &text[isolate.range.clone()];
            if !inside.contains('\n') {
                continue;
            }
            let _ = self.buffer.remove_isolate(isolate.id);
            let mut from = isolate.range.start;
            for piece in inside.split('\n') {
                // Not where its line now runs its way anyhow.
                let line_from = text[..from].rfind('\n').map_or(0, |i| i + 1);
                let line_to = text[from..].find('\n').map_or(text.len(), |i| from + i);
                let runs = if bidi::base(&text[line_from..line_to]) % 2 == 1 {
                    Direction::RightToLeft
                } else {
                    Direction::LeftToRight
                };
                if !piece.trim().is_empty() && runs != isolate.direction {
                    let range = Range { start: from, end: from + piece.len() };
                    let _ = self.buffer.mark_isolate(range, isolate.direction);
                }
                from += piece.len() + 1;
            }
        }
        let mut start = 0;
        for paragraph in text.split('\n') {
            let end = start + paragraph.len();
            // Each step makes one isolate of more text than before, so
            // there are fewer steps than characters.
            for _ in 0..=paragraph.len() {
                let top: Vec<_> = self
                    .buffer
                    .isolates()
                    .into_iter()
                    .filter(|i| i.parent.is_none() && start <= i.range.start && i.range.end <= end)
                    .collect();
                let stretches: Vec<Stretch> = top
                    .iter()
                    .map(|i| Stretch {
                        range: i.range.start - start..i.range.end - start,
                        direction: i.direction,
                    })
                    .collect();
                let Some(proposal) = bidi::propose(paragraph, &stretches) else {
                    break;
                };
                for &i in &proposal.remove {
                    let _ = self.buffer.remove_isolate(top[i].id);
                }
                let range = Range {
                    start: start + proposal.mark.range.start,
                    end: start + proposal.mark.range.end,
                };
                if self.buffer.mark_isolate(range, proposal.mark.direction).is_err() {
                    break;
                }
            }
            start = end + 1;
        }
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
        self.propose();
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
        self.display.set(&self.flat, &self.buffer.isolates());
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

    fn hebrew(text: &str) -> Editor {
        let inter =
            Font::from_bytes(include_bytes!("../fonts/Inter-Regular.ttf").to_vec()).unwrap();
        let noto =
            Font::from_bytes(include_bytes!("../fonts/NotoSansHebrew-Regular.ttf").to_vec()).unwrap();
        Editor::new(text, Arc::new(inter), vec![Arc::new(noto)], 17 * PT, 24 * PT, 300 * PT)
    }

    /// Each isolate's text.
    fn isolated(e: &Editor) -> Vec<&str> {
        e.buffer.isolates().iter().map(|i| &e.flat[i.range.clone()]).collect()
    }

    /// Types `text` a character at a time, as a keyboard does.
    fn type_in(e: &mut Editor, text: &str) {
        for c in text.chars() {
            let at = e.selection();
            e.replace(at, c.encode_utf8(&mut [0; 4]));
        }
    }

    #[test]
    fn hebrew_in_english_text_is_isolated_as_it_comes() {
        // Loaded: the algorithm's stretches become isolates.
        let e = hebrew("abc אבג דהו def\nאבג abc\nplain");
        assert_eq!(isolated(&e), ["אבג דהו", "abc"]);
        // Typed: a phrase is one isolate from its first letter on.
        let mut e = hebrew("say ");
        e.set_selection(r(4, 4));
        type_in(&mut e, "אבג");
        assert_eq!(isolated(&e), ["אבג"]);
        let id = e.buffer.isolates()[0].id;
        // A space stays with the word it was typed after.
        type_in(&mut e, " ");
        assert_eq!(isolated(&e), ["אבג "]);
        type_in(&mut e, "דהו ");
        assert_eq!(isolated(&e), ["אבג דהו "]);
        // English leaves the phrase, and the space before it goes too.
        type_in(&mut e, "end");
        assert_eq!((e.text(), isolated(&e)), ("say אבג דהו end", vec!["אבג דהו"]));
        assert_eq!(e.buffer.isolates()[0].id, id);
    }

    #[test]
    fn typing_never_sends_the_cursor_across_the_line() {
        // An English paragraph. Hebrew is typed leftwards from where it
        // starts; a space after it carries on leftwards.
        let mut e = hebrew("say ");
        e.set_selection(r(4, 4));
        let x = |e: &Editor| e.caret_rect(e.caret()).x;
        let start = x(&e);
        type_in(&mut e, "א");
        // The first letter is drawn at the cursor, and the cursor is on
        // its left.
        let after_one = x(&e);
        assert_eq!(after_one, start);
        type_in(&mut e, "בג");
        assert_eq!(x(&e), start);
        // Its right edge moved out as it grew; the cursor stayed.
        assert!(e.caret_rect(Place::downstream(4)).x > start);
        let right = e.caret_rect(Place::downstream(4)).x;
        type_in(&mut e, " ");
        // A space is more of the phrase: it goes in at the cursor, on the
        // Hebrew's left, and the Hebrew moves over for it. The cursor
        // hasn't moved, and it isn't on top of anything.
        assert_eq!(x(&e), start);
        assert!(e.caret_rect(Place::downstream(4)).x > right);
        let side = e.caret_side();
        assert_eq!((side.direction, side.choice, side.held), (Direction::RightToLeft, true, false));
        // English: it shows on the phrase's right, where English goes
        // on, and the cursor is after it.
        type_in(&mut e, "e");
        assert_eq!(e.text(), "say אבג e");
        assert_eq!(e.caret_side().direction, Direction::LeftToRight);
        assert!(x(&e) > e.caret_rect(Place::downstream(4)).x);
    }

    #[test]
    fn the_writer_can_hold_a_side() {
        // At the end of a Hebrew phrase, English leaves it...
        let mut e = hebrew("say אבג");
        e.set_caret(Place::upstream(10));
        assert_eq!(e.caret_side().direction, Direction::RightToLeft);
        type_in(&mut e, " x");
        assert_eq!(isolated(&e), ["אבג"]);
        // ...unless the side's held: then it's part of the phrase, and
        // set in it. The cursor's at the phrase's end, on its left; a
        // flip takes it to the other side of that spot, the end of the
        // English before, and a second brings it back, held.
        let mut e = hebrew("say אבג");
        e.set_caret(Place::upstream(10));
        let x = e.caret_rect(e.caret()).x;
        assert!(e.flip_side());
        assert_eq!((e.caret(), e.caret_rect(e.caret()).x), (Place::upstream(4), x));
        assert_eq!(e.caret_side().direction, Direction::LeftToRight);
        assert!(e.flip_side());
        assert_eq!((e.caret(), e.caret_side().held), (Place::upstream(10), true));
        assert_eq!(e.kept_in().map(|i| i.range), Some(4..10));
        type_in(&mut e, " abc");
        assert_eq!((e.text(), isolated(&e)), ("say אבג abc", vec!["אבג abc"]));
        // Moving the cursor lets go.
        e.set_caret(Place::downstream(4));
        assert!(!e.caret_side().held);
        // Held on the English side of that spot, Hebrew typed there is
        // before the phrase, and not in it.
        let mut e = hebrew("say אבג");
        e.set_caret(Place::upstream(10));
        assert!(e.flip_side());
        type_in(&mut e, "ד");
        assert_eq!(e.text(), "say דאבג");
        assert_eq!(isolated(&e), ["ד", "אבג"]);
        // At the phrase's right, the flip is between "before its first
        // letter" and "after everything", and the cursor doesn't move.
        let mut e = hebrew("say אבג");
        e.set_caret(Place::downstream(10));
        let x = e.caret_rect(e.caret()).x;
        assert!(e.flip_side());
        assert_eq!((e.caret(), e.caret_rect(e.caret()).x), (Place::downstream(4), x));
        assert_eq!(e.caret_side().direction, Direction::RightToLeft);
        // Where there's one side, there's nothing to flip.
        e.set_caret(Place::downstream(1));
        assert!(!e.flip_side());
    }

    #[test]
    fn a_touch_takes_the_keyboards_side() {
        // At the right of Hebrew that ends an English line.
        let mut e = hebrew("say אבג");
        e.set_caret(Place::downstream(10));
        // English keyboard: it's after everything already.
        assert!(!e.settle(Direction::LeftToRight));
        // Hebrew keyboard: before the Hebrew's first letter, same spot.
        let x = e.caret_rect(e.caret()).x;
        assert!(e.settle(Direction::RightToLeft));
        assert_eq!((e.caret(), e.caret_rect(e.caret()).x), (Place::downstream(4), x));
        assert!(!e.caret_side().held);
        // And back with the English one.
        assert!(e.settle(Direction::LeftToRight));
        assert_eq!(e.caret(), Place::downstream(10));
        // A held side isn't taken away, and the middle of text has one.
        e.flip_side();
        assert!(!e.settle(Direction::LeftToRight));
        e.set_caret(Place::downstream(6));
        assert!(!e.settle(Direction::LeftToRight));
    }

    #[test]
    fn a_cursor_that_stays_put() {
        let mut e = hebrew("say ");
        e.set_stays_put(true);
        e.set_selection(r(4, 4));
        let x = |e: &Editor| e.caret_rect(e.caret()).x;
        let start = x(&e);
        type_in(&mut e, "אבג ");
        // A change of keyboard doesn't move it.
        assert!(!e.face(Direction::LeftToRight));
        // English goes in at the cursor: in the phrase, at its left end,
        // where the cursor was. It runs rightwards from there, pushing
        // the Hebrew along; the cursor is after it, still left of the
        // Hebrew.
        assert_eq!(x(&e), start);
        type_in(&mut e, "ab");
        assert_eq!((e.text(), isolated(&e)), ("say אבג ab", vec!["אבג ab"]));
        assert_eq!(e.caret_rect(Place::downstream(11)).x, start);
        assert!(x(&e) > start && x(&e) < e.caret_rect(Place::downstream(4)).x);
        // While it's in the phrase, the phrase is what's shown as holding
        // it.
        assert_eq!(e.kept_in().map(|i| &e.flat[i.range]), Some("אבג ab"));
        // The writer steps out, to the line's end, and there it's
        // English's side.
        e.set_caret(Place::downstream(13));
        assert_eq!(e.kept_in(), None);
        type_in(&mut e, " end");
        assert_eq!((e.text(), isolated(&e)), ("say אבג ab end", vec!["אבג ab"]));
        // Turned off, it's as before.
        e.set_stays_put(false);
        let mut e = hebrew("say אבג");
        e.set_caret(Place::upstream(10));
        type_in(&mut e, " x");
        assert_eq!(isolated(&e), ["אבג"]);
    }

    #[test]
    fn typing_over_text_goes_where_the_text_was() {
        // The last Hebrew word, selected and typed over. דהו is 11..17.
        let text = "say אבג דהו end";
        let over = |e: &mut Editor, with: &str| {
            e.set_selection(r(11, 17));
            let selection = e.selection();
            e.replace(selection, with);
        };
        // With a space and a digit: neither says a direction, and they
        // stay in the phrase the word was in.
        let mut e = hebrew(text);
        over(&mut e, " 7");
        assert_eq!((e.text(), isolated(&e)), ("say אבג  7 end", vec!["אבג  7"]));
        // With Hebrew: in the phrase.
        let mut e = hebrew(text);
        over(&mut e, "זחט");
        assert_eq!(isolated(&e), ["אבג זחט"]);
        // With English: it leaves, as English typed there would.
        let mut e = hebrew(text);
        over(&mut e, "x");
        assert_eq!((e.text(), isolated(&e)), ("say אבג x end", vec!["אבג"]));
        // Unless the cursor stays put: then it's in the phrase too.
        let mut e = hebrew(text);
        e.set_stays_put(true);
        over(&mut e, "x");
        assert_eq!((e.text(), isolated(&e)), ("say אבג x end", vec!["אבג x"]));
        // A whole phrase typed over is gone, and what replaces it is
        // wherever the phrase was: here, in the English.
        let mut e = hebrew(text);
        e.set_selection(r(4, 17));
        e.replace(r(4, 17), "7");
        assert_eq!((e.text(), isolated(&e)), ("say 7 end", vec![]));
        // Pasted at the cursor, text goes to the cursor's side like
        // anything typed.
        let mut e = hebrew(text);
        e.set_caret(Place::upstream(17));
        e.replace(r(17, 17), ", 12");
        assert_eq!(isolated(&e), ["אבג דהו, 12"]);
    }

    fn reopened(e: &mut Editor, text: &str) -> Editor {
        let inter =
            Font::from_bytes(include_bytes!("../fonts/Inter-Regular.ttf").to_vec()).unwrap();
        let noto =
            Font::from_bytes(include_bytes!("../fonts/NotoSansHebrew-Regular.ttf").to_vec()).unwrap();
        let history = e.history();
        Editor::open(&history, "me", text, Arc::new(inter), vec![Arc::new(noto)], 17 * PT, 24 * PT, 300 * PT)
    }

    #[test]
    fn a_history_keeps_what_the_characters_cant_say() {
        // English held in a Hebrew phrase: nothing in the characters
        // says it's in there.
        let mut e = hebrew("say אבג");
        e.set_stays_put(true);
        e.set_caret(Place::upstream(10));
        type_in(&mut e, " abc");
        assert_eq!(isolated(&e), ["אבג abc"]);
        let x = e.caret_rect(Place::downstream(11)).x;
        // Made again from the text alone, the English is outside.
        let plain = hebrew("say אבג abc");
        assert_eq!(isolated(&plain), ["אבג"]);
        assert_ne!(plain.caret_rect(Place::downstream(11)).x, x);
        // Made again from its history, it's as it was left.
        let text = e.text().to_string();
        let mut back = reopened(&mut e, &text);
        assert_eq!((back.text(), isolated(&back)), ("say אבג abc", vec!["אבג abc"]));
        assert_eq!(back.buffer.isolates(), e.buffer.isolates());
        assert_eq!(back.caret_rect(Place::downstream(11)).x, x);
        // And it can be typed in, and kept again.
        back.set_caret(Place::upstream(14));
        back.flip_side();
        back.flip_side();
        type_in(&mut back, "d");
        assert_eq!(isolated(&back), ["אבג abcd"]);
        let text = back.text().to_string();
        assert_eq!(isolated(&reopened(&mut back, &text)), ["אבג abcd"]);
    }

    #[test]
    fn a_history_follows_text_changed_behind_its_back() {
        let mut e = hebrew("say אבג");
        e.set_stays_put(true);
        e.set_caret(Place::upstream(10));
        type_in(&mut e, " abc");
        let id = e.buffer.isolates()[0].id;
        // The text was edited somewhere that didn't have the history:
        // a word in front, a letter in the phrase.
        let back = reopened(&mut e, "do say אבdג abc now");
        assert_eq!(back.text(), "do say אבdג abc now");
        assert_eq!(isolated(&back), ["אבdג abc"]);
        assert_eq!(back.buffer.isolates()[0].id, id);
        // With no history at all, it's a new editor for the text.
        let inter =
            Font::from_bytes(include_bytes!("../fonts/Inter-Regular.ttf").to_vec()).unwrap();
        let fresh = Editor::open(b"", "me", "say אבג", Arc::new(inter), vec![], 17 * PT, 24 * PT, 300 * PT);
        assert_eq!((fresh.text(), isolated(&fresh)), ("say אבג", vec!["אבג"]));
    }

    #[test]
    fn a_broken_line_is_no_choice_of_side() {
        // Two places, one at the end of a line and one at the start of
        // the next: nothing to do with direction.
        let mut e = editor("aaaa-bbbb-cccc-dddd", 60);
        let b = e.lines()[1].start;
        e.set_caret(Place::upstream(b));
        assert_ne!(e.caret_rect(Place::upstream(b)), e.caret_rect(Place::downstream(b)));
        assert!(!e.caret_side().choice);
        assert!(!e.flip_side());
    }

    #[test]
    fn changing_keyboard_turns_the_cursor() {
        let mut e = hebrew("say אבג");
        // After the Hebrew, with it. A Latin keyboard comes up: the
        // cursor goes to where Latin will show.
        e.set_caret(Place::upstream(10));
        let x = e.caret_rect(e.caret()).x;
        assert!(e.face(Direction::LeftToRight));
        assert_eq!(e.caret(), Place::downstream(10));
        assert!(e.caret_rect(e.caret()).x > x);
        // Back to Hebrew, and back it goes.
        assert!(e.face(Direction::RightToLeft));
        assert_eq!(e.caret(), Place::upstream(10));
        // It's already facing that way: nothing to do. Nor if it's held.
        assert!(!e.face(Direction::RightToLeft));
        e.flip_side();
        assert!(!e.face(Direction::RightToLeft));
    }

    #[test]
    fn return_in_an_isolate_makes_two() {
        let mut e = hebrew("say אבג דהו end");
        assert_eq!(isolated(&e), ["אבג דהו"]);
        // Between the two Hebrew words, after the space.
        e.set_selection(r(11, 11));
        type_in(&mut e, "\n");
        assert_eq!(e.text(), "say אבג \nדהו end");
        // The first line keeps its Hebrew as an isolate; the second
        // starts with Hebrew, so it's a Hebrew paragraph, with "end" in
        // it.
        assert_eq!(isolated(&e), ["אבג ", "end"]);
        assert!(e.buffer.isolates().iter().all(|i| !e.flat[i.range.clone()].contains('\n')));
    }

    #[test]
    fn a_letter_typed_into_a_hebrew_word_doesnt_move_it() {
        // אבג is bytes 4..10; the cursor goes between ב and ג.
        let mut e = hebrew("abc אבג def");
        let left = e.caret_rect(Place::upstream(10)).x;
        let width = e.caret_rect(Place::downstream(4)).x - left;
        e.set_selection(r(8, 8));
        e.replace(r(8, 8), "d");
        assert_eq!((e.text(), isolated(&e)), ("abc אבdג def", vec!["אבdג"]));
        // On the page, from the left: ג, d, ב, א. The word's left end is
        // where it was, and it's wider by the d.
        let x = |p: Place| e.caret_rect(p).x;
        let (before_d, after_d) = (x(Place::downstream(8)), x(Place::upstream(9)));
        assert_eq!(x(Place::upstream(11)), left);
        assert!(left < before_d && before_d < after_d && after_d < x(Place::downstream(4)));
        assert_eq!(x(Place::downstream(4)) - left, width + (after_d - before_d));
        // ג is on the d's left, ב on its right.
        assert_eq!(x(Place::downstream(9)), before_d);
        assert_eq!(x(Place::upstream(8)), after_d);
        // The cursor is with the d: on its right, where the next Latin
        // letter will show.
        assert_eq!(e.caret(), Place::upstream(9));
        assert_eq!(e.caret_rect(e.caret()).x, after_d);
    }

    #[test]
    fn text_from_elsewhere_keeps_its_isolates() {
        let mut e = hebrew("abc אבג def");
        e.replace(r(8, 8), "d");
        let id = e.buffer.isolates()[0].id;
        // The block's text comes back from where it's saved, with more
        // in front. The word is still the isolate it was, Latin letter
        // and all, where the algorithm would have made two of it.
        e.set_text("so abc אבdג def");
        assert_eq!(isolated(&e), ["אבdג"]);
        assert_eq!(e.buffer.isolates()[0].id, id);
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
