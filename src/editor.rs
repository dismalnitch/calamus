//! editor
//!
//! A walking skeleton of the editor: the text, the selection, marked text
//! and the text set into lines, with the answers a platform's text input
//! asks for (where is the cursor, what's under this point, which rectangles
//! cover this selection, what's one line up). Ported from tolege's
//! editing-on-Idem proof of concept (tolege#216) so a host app can type in
//! it while the real pieces are built.
//!
//! Offsets are UTF-8 byte offsets into the whole text, as in
//! [`Buffer`]; lengths are scaled points, as in [`layout`](crate::layout).
//! [`Editor::to_utf16`] and [`Editor::from_utf16`] are for bridges to
//! platforms that count in UTF-16.
//!
//! What's rough, and where the real thing is meant to come from:
//! - Caret stops are one per grapheme boundary, at the x of the glyph whose
//!   cluster starts there; a boundary with no glyph of its own (inside a
//!   ligature) is shared out evenly between its neighbours. The real rules
//!   are in [`caret`](crate::caret)'s notes.
//! - No affinity: the offset at a soft line break is always the start of
//!   the next line.
//! - Left to right only, as the typesetter is: stops assume x grows with
//!   the offset.
//! - Every edit copies the buffer out to a `String` and sets the whole text
//!   again, paragraph by paragraph. No cache, no incremental layout.
//! - One font and its fallbacks, ragged, no styles, no undo.
//! - Line ends are `\n` only: `\r\n` and `\r` become `\n` on the way in.
//! - Spaces before a paragraph's end get a space's width each for the
//!   cursor's sake, though the typesetter gives them none. Spaces at a soft
//!   break still take no room: the cursor goes to the next line.

use std::range::Range;
use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation;

use crate::buffer::Buffer;
use crate::layout::{Align, Font, Glyph, Run, Style, TextStyle, set_paragraph};

/// A rectangle in scaled points, from the top-left of the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

/// A place the cursor can stand on a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stop {
    /// Byte offset into the whole text.
    pub offset: usize,
    /// From the line's start, in scaled points.
    pub x: i64,
}

/// One line of the text as set.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// Down from the top of the text.
    pub top: i64,
    pub height: i64,
    /// The baseline, down from the top of the text.
    pub baseline: i64,
    /// How wide the line is as set.
    pub width: i64,
    /// The first byte offset on the line.
    pub start: usize,
    /// Every place the cursor can stand on the line, in text order. A
    /// soft-broken line's end isn't one: that offset is the next line's
    /// start.
    pub stops: Vec<Stop>,
    /// The glyphs to draw: `x` from the line's start, `y` down from the
    /// baseline, `cluster` a byte offset into the whole text.
    pub glyphs: Vec<Glyph>,
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
    font: Arc<Font>,
    fallback: Vec<Arc<Font>>,
    size: i64,
    line_height: i64,
    width: i64,
    selection: Range<usize>,
    marked: Option<Range<usize>>,
    lines: Vec<Line>,
    /// Every grapheme cluster boundary in the text, in order.
    boundaries: Vec<usize>,
    height: i64,
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
            font,
            fallback,
            size,
            line_height,
            width,
            selection: Range { start: 0, end: 0 },
            marked: None,
            lines: Vec::new(),
            boundaries: Vec::new(),
            height: 0,
        };
        editor.splice(Range { start: 0, end: 0 }, text);
        editor
    }

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
        self.splice(Range { start: 0, end: self.len() }, text);
        self.marked = None;
        self.set_selection(selection);
    }

    /// Replace `range` with `text`; the cursor goes after it, and any
    /// marked text is forgotten.
    pub fn replace(&mut self, range: Range<usize>, text: &str) {
        let (start, added) = self.splice(range, text);
        self.selection = Range { start: start + added, end: start + added };
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
            Range { start: self.boundary_before(sel.start), end: sel.start }
        };
        let (start, _) = self.splice(range, "");
        self.selection = Range { start, end: start };
        self.marked = None;
    }

    // Selection and marked text

    pub fn selection(&self) -> Range<usize> {
        self.selection
    }

    /// Select `range`, widened to whole grapheme clusters: a selection never
    /// starts or ends inside one.
    pub fn set_selection(&mut self, range: Range<usize>) {
        let len = self.len();
        let (a, b) = (range.start.min(range.end).min(len), range.start.max(range.end).min(len));
        let start = self.boundary_at_or_before(a);
        let end = if a == b { start } else { self.boundary_at_or_after(b) };
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
        self.marked = (added > 0).then_some(Range { start, end: start + added });
        let inside = |at: usize| {
            let mut at = start + at.min(added);
            while !self.flat.is_char_boundary(at) {
                at -= 1;
            }
            at
        };
        self.selection = Range { start: inside(selected.start), end: inside(selected.end) };
    }

    /// Keep the marked text as it is, unmarked.
    pub fn unmark_text(&mut self) {
        if let Some(m) = self.marked.take() {
            self.selection = Range { start: m.end, end: m.end };
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

    /// The place `lines` lines up (negative) or down from `offset`, nearest
    /// `goal_x` (or the cursor's own x). `None` past the first or last line.
    pub fn offset_by_lines(&self, offset: usize, lines: isize, goal_x: Option<i64>) -> Option<usize> {
        let i = self.line_of(offset) as isize + lines;
        if i < 0 || i >= self.lines.len() as isize {
            return None;
        }
        let x = goal_x.unwrap_or_else(|| self.x_of(offset));
        Some(nearest_stop(&self.lines[i as usize], x))
    }

    // Geometry

    /// Where the cursor stands at `offset`: a rectangle of no width, the
    /// line's height. The platform decides how thick to draw it.
    pub fn caret_rect(&self, offset: usize) -> Rect {
        let line = &self.lines[self.line_of(offset)];
        Rect { x: self.x_of(offset), y: line.top, width: 0, height: line.height }
    }

    /// The place under (`x`, `y`): the caret stop nearest it on the line at
    /// that height.
    pub fn closest_offset(&self, x: i64, y: i64) -> usize {
        let i = self.lines.partition_point(|l| l.top + l.height <= y).min(self.lines.len() - 1);
        nearest_stop(&self.lines[i], x)
    }

    /// The rectangles covering `range`, one per line it touches. A line the
    /// selection runs on past reaches the editor's right edge.
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let (a, b) = (range.start.min(range.end), range.start.max(range.end));
        let (first, last) = (self.line_of(a), self.line_of(b));
        (first..=last)
            .filter_map(|i| {
                let line = &self.lines[i];
                let x0 = if i == first { self.x_of(a) } else { 0 };
                let x1 = if i == last { self.x_of(b) } else { self.width };
                (x1 > x0 || a == b).then_some(Rect { x: x0, y: line.top, width: x1 - x0, height: line.height })
            })
            .collect()
    }

    // Drawing and layout

    /// The text as set. Never empty: an empty text is one empty line.
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// The lines' heights added up.
    pub fn height(&self) -> i64 {
        self.height
    }

    pub fn width(&self) -> i64 {
        self.width
    }

    /// Set the text again `width` scaled points wide.
    pub fn set_width(&mut self, width: i64) {
        if width != self.width {
            self.width = width;
            self.layout();
        }
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
        Range { start: a.min(b), end: a.max(b) }
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

    /// The line `offset` is on (a soft break's offset is on the next).
    fn line_of(&self, offset: usize) -> usize {
        self.lines.partition_point(|l| l.start <= offset).saturating_sub(1)
    }

    /// The x of `offset` on its line: its stop's, or the nearest stop
    /// before it.
    fn x_of(&self, offset: usize) -> i64 {
        let line = &self.lines[self.line_of(offset)];
        let i = line.stops.partition_point(|s| s.offset <= offset);
        line.stops.get(i.saturating_sub(1)).map_or(0, |s| s.x)
    }

    /// Copy the buffer out and set the whole text again, paragraph by
    /// paragraph.
    fn layout(&mut self) {
        self.flat = self
            .buffer
            .read(0, self.buffer.len())
            .expect("the whole buffer is in bounds")
            .text
            .to_string();
        self.index_utf16();

        let fallback: Vec<&Font> = self.fallback.iter().map(|f| f.as_ref()).collect();
        let style = TextStyle {
            font: &self.font,
            fallback: &fallback,
            language: None,
            size: self.size,
            line_height: self.line_height,
        };
        let paragraph_style = Style { strut: style, align: Align::Ragged };
        // How wide a space is, measured as a no-break space, which the
        // typesetter keeps.
        let space = {
            let probe = "\u{a0}";
            let runs = [Run { len: probe.len(), style }];
            set_paragraph(probe, &runs, &paragraph_style, &[i64::MAX / 4]).lines[0].width
        };
        let mut lines = Vec::new();
        let mut boundaries = Vec::new();
        let mut top = 0i64;
        let mut start = 0usize;
        for (k, text) in self.flat.split('\n').enumerate() {
            if k > 0 {
                start += 1; // the line end
            }
            let graphemes: Vec<usize> =
                text.grapheme_indices(true).map(|(i, _)| i).chain(std::iter::once(text.len())).collect();
            boundaries.extend(graphemes.iter().map(|&b| start + b));
            let runs = [Run { len: text.len(), style }];
            let set = set_paragraph(text, &runs, &paragraph_style, &[self.width]);
            let count = set.lines.len();
            for (i, line) in set.lines.iter().enumerate() {
                let last = i + 1 == count;
                let on_line: Vec<usize> = graphemes
                    .iter()
                    .copied()
                    .filter(|&b| b >= line.text.start && (b < line.text.end || (last && b == line.text.end)))
                    .collect();
                let mut xs = stop_xs(&on_line, &line.glyphs, line.width);
                if last {
                    // The typesetter gives the spaces before a paragraph's
                    // end no room, so every stop among them would sit at
                    // the line's end: a space typed there, or deleted,
                    // wouldn't move the cursor. Step past each one.
                    let trimmed = text.trim_end_matches(' ').len();
                    for (&b, x) in on_line.iter().zip(xs.iter_mut()) {
                        if b > trimmed {
                            *x = line.width + (b - trimmed) as i64 * space;
                        }
                    }
                }
                lines.push(Line {
                    top: top + line.top,
                    height: line.height,
                    baseline: top + line.baseline,
                    width: line.width,
                    start: start + line.text.start,
                    stops: on_line.iter().zip(xs).map(|(&b, x)| Stop { offset: start + b, x }).collect(),
                    glyphs: line.glyphs.iter().map(|g| Glyph { cluster: start + g.cluster, ..*g }).collect(),
                });
            }
            top += set.height;
            start += text.len();
        }
        boundaries.dedup();
        self.lines = lines;
        self.boundaries = boundaries;
        self.height = top;
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

fn nearest_stop(line: &Line, x: i64) -> usize {
    line.stops.iter().min_by_key(|s| (s.x - x).abs()).map_or(line.start, |s| s.offset)
}

/// The x of each boundary on a line (byte offsets into the paragraph, in
/// order): the leftmost glyph whose cluster starts there; boundaries with
/// none (inside a ligature) evenly between their neighbours; past the last
/// glyph, the line's width.
fn stop_xs(boundaries: &[usize], glyphs: &[Glyph], width: i64) -> Vec<i64> {
    let at = |b: usize| glyphs.iter().filter(|g| g.cluster == b).map(|g| g.x).min();
    let last_cluster = glyphs.iter().map(|g| g.cluster).max();
    let mut xs: Vec<Option<i64>> = boundaries
        .iter()
        .map(|&b| match last_cluster {
            Some(c) if b > c => Some(width),
            None => Some(0),
            _ => at(b),
        })
        .collect();
    // Fill the gaps between known stops evenly.
    let mut i = 0;
    while i < xs.len() {
        if xs[i].is_some() {
            i += 1;
            continue;
        }
        let before = if i > 0 { xs[i - 1].unwrap_or(0) } else { 0 };
        let j = (i..xs.len()).find(|&j| xs[j].is_some()).unwrap_or(xs.len());
        let after = if j < xs.len() { xs[j].unwrap_or(width) } else { width };
        let gaps = (j - i + 1) as i64;
        for (k, x) in xs[i..j].iter_mut().enumerate() {
            *x = Some(before + (after - before) * (k as i64 + 1) / gaps);
        }
        i = j;
    }
    xs.into_iter().map(|x| x.unwrap_or(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::PT;

    fn editor(text: &str, width: i64) -> Editor {
        let inter = Font::from_bytes(include_bytes!("../fonts/Inter-Regular.ttf").to_vec()).unwrap();
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
    fn stops_never_fall_inside_a_cluster() {
        let text = "naïve e\u{301} 😀 caf\u{e9} office ffi\nsecond paragraph";
        let e = editor(text, 80);
        assert!(e.lines.len() > 3);
        for line in &e.lines {
            for s in &line.stops {
                assert!(e.boundaries.binary_search(&s.offset).is_ok(), "stop {} inside a cluster", s.offset);
            }
            assert!(line.stops.windows(2).all(|w| w[0].offset < w[1].offset && w[0].x <= w[1].x));
        }
        // Every boundary is a stop on exactly one line.
        let all: Vec<usize> = e.lines.iter().flat_map(|l| l.stops.iter().map(|s| s.offset)).collect();
        assert_eq!(all, e.boundaries);
    }

    #[test]
    fn hit_and_caret_round_trip() {
        let e = editor("Take up and read, take up and read.\nTolle lege.", 120);
        for &o in &e.boundaries {
            let c = e.caret_rect(o);
            assert_eq!(e.closest_offset(c.x, c.y + c.height / 2), o, "offset {o}");
        }
    }

    #[test]
    fn up_and_down_keep_the_column() {
        let e = editor("aaaa aaaa aaaa aaaa aaaa aaaa aaaa", 60);
        let x = e.caret_rect(2).x;
        let down = e.offset_by_lines(2, 1, Some(x)).unwrap();
        assert_eq!(e.offset_by_lines(down, -1, Some(x)), Some(2));
        assert_eq!(e.offset_by_lines(2, -1, None), None);
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
    fn an_empty_text_is_one_line_with_one_stop() {
        let e = editor("", 300);
        assert_eq!(e.lines().len(), 1);
        assert_eq!(e.lines()[0].stops, [Stop { offset: 0, x: 0 }]);
        assert_eq!(e.caret_rect(0), Rect { x: 0, y: 0, width: 0, height: 24 * PT });
        assert_eq!(e.height(), 24 * PT);
    }

    #[test]
    fn a_space_at_the_end_moves_the_cursor() {
        // Typing a space after a word, and deleting it, has to show.
        let e = editor("hello  ", 300);
        let x = |o| e.caret_rect(o).x;
        assert!(x(5) < x(6) && x(6) < x(7), "{} {} {}", x(5), x(6), x(7));
        // And each place can be touched.
        for o in [5, 6, 7] {
            assert_eq!(e.closest_offset(x(o), 0), o);
        }
        // The same at the end of any paragraph, not only the last.
        let e = editor("one \ntwo", 300);
        assert!(e.caret_rect(3).x < e.caret_rect(4).x);
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
}
