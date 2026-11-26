//! display
//!
//! The text set into lines, and the answers a platform's text input asks
//! about where things are: where is the cursor, what's under this point,
//! which rectangles cover this selection, what's one line up. Rust computes
//! the lines and their caret stops once per edit; the platform looks things
//! up in them (see [`caret`](crate::caret)'s notes).
//!
//! It doesn't hold the text: it's handed the text to set, and keeps what
//! came of it. Offsets are UTF-8 byte offsets into the whole text; lengths
//! are scaled points, as in [`layout`](crate::layout).
//!
//! What's rough, and where the real thing is meant to come from:
//! - Caret stops are one per grapheme boundary, at the x of the glyph whose
//!   cluster starts there; a boundary with no glyph of its own (inside a
//!   ligature) is shared out evenly between its neighbours. The real rules
//!   are in [`caret`](crate::caret)'s notes.
//! - An offset can have two places ([`Place`]): where a line breaks with
//!   no space (after a hyphen, between ideographs) the offset is the end
//!   of one line and the start of the next, and an [`Affinity`] says
//!   which. Only line breaks make them so far; the lookups already take
//!   two stops for one offset on the same line, which is what a change of
//!   direction will make.
//! - Left to right only, as the typesetter is: stops assume x grows with
//!   the offset.
//! - The whole text is set again on every change, paragraph by paragraph.
//!   No cache, no incremental layout.
//! - One font and its fallbacks, ragged, no styles.
//! - Spaces before a paragraph's end get a space's width each for the
//!   cursor's sake, though the typesetter gives them none. Spaces at a soft
//!   break still take no room: the cursor goes to the next line.

use std::range::Range;
use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation;

pub use crate::caret::Affinity;
use crate::layout::{Align, Font, Glyph, Run, Style, TextStyle, is_space, set_paragraph};

/// A rectangle in scaled points, from the top-left of the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

/// Where the cursor is in the text: an offset, and which of its places
/// where it has two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    /// Byte offset into the whole text.
    pub offset: usize,
    pub affinity: Affinity,
}

impl Place {
    /// `offset`'s place with the text before it.
    pub fn upstream(offset: usize) -> Self {
        Place { offset, affinity: Affinity::Upstream }
    }

    /// `offset`'s place with the text after it: its only place, where it
    /// has one.
    pub fn downstream(offset: usize) -> Self {
        Place { offset, affinity: Affinity::Downstream }
    }
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
    /// Every place the cursor can stand on the line, in text order. An
    /// offset with two places has two stops, the upstream one first: the
    /// end of a line broken with no space is that line's last stop and the
    /// next line's first. A line broken at spaces doesn't end in one: the
    /// spaces take no room, so the place after them is the next line's
    /// start only.
    pub stops: Vec<Stop>,
    /// The glyphs to draw: `x` from the line's start, `y` down from the
    /// baseline, `cluster` a byte offset into the whole text.
    pub glyphs: Vec<Glyph>,
}

pub struct Display {
    font: Arc<Font>,
    fallback: Vec<Arc<Font>>,
    size: i64,
    line_height: i64,
    width: i64,
    lines: Vec<Line>,
    height: i64,
}

/// Where a caret stop is: its line, and which of the line's stops. A place
/// to step from, stop by stop, and back.
///
/// Indices into the display as set, so good until the next
/// [`Display::set`]. Steps go stop by stop, so from an offset's upstream
/// place to its downstream one is a step that keeps the offset (see
/// [`Display::place`]). They go in text order for now; when right to left
/// comes, they're where "next on the page" will part from "next offset".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct StopIndex {
    pub line: usize,
    pub stop: usize,
}

impl StopIndex {
    pub fn new(line: usize, stop: usize) -> Self {
        Self { line, stop }
    }

    /// The stop after this one in `display`, onto the next line from a
    /// line's last; `None` from the very last.
    pub fn next(self, display: &Display) -> Option<Self> {
        if self.stop + 1 < display.lines.get(self.line)?.stops.len() {
            return Some(Self::new(self.line, self.stop + 1));
        }
        (self.line + 1..display.lines.len())
            .find(|&i| !display.lines[i].stops.is_empty())
            .map(|i| Self::new(i, 0))
    }

    /// The stop before this one in `display`, onto the previous line from
    /// a line's first; `None` from the very first.
    pub fn prev(self, display: &Display) -> Option<Self> {
        if self.stop > 0 {
            return Some(Self::new(self.line, self.stop - 1));
        }
        (0..self.line.min(display.lines.len()))
            .rev()
            .find(|&i| !display.lines[i].stops.is_empty())
            .map(|i| Self::new(i, display.lines[i].stops.len() - 1))
    }
}

impl Display {
    /// A display setting text in `font` (and `fallback` for what it lacks)
    /// at `size` and `line_height`, `width` wide, all in scaled points.
    /// Nothing is set until [`Display::set`].
    pub fn new(
        font: Arc<Font>,
        fallback: Vec<Arc<Font>>,
        size: i64,
        line_height: i64,
        width: i64,
    ) -> Self {
        Display {
            font,
            fallback,
            size,
            line_height,
            width,
            lines: Vec::new(),
            height: 0,
        }
    }

    /// The text as set. Never empty once set: an empty text is one empty
    /// line.
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

    /// Set `text` again `width` scaled points wide.
    pub fn set_width(&mut self, width: i64, text: &str) {
        if width != self.width {
            self.width = width;
            self.set(text);
        }
    }

    /// Where `at`'s stop is: on its line, its own stop, or the last stop
    /// before it if it's inside a cluster.
    pub fn stop_index(&self, at: Place) -> StopIndex {
        let line = self.line_of(at);
        let stops = &self.lines[line].stops;
        let before = stops.partition_point(|s| s.offset < at.offset);
        let through = stops.partition_point(|s| s.offset <= at.offset);
        // Its stops are `before..through`: the first is upstream, the last
        // downstream. None: the stop before.
        let stop = match at.affinity {
            Affinity::Upstream if before < through => before,
            _ => through.saturating_sub(1),
        };
        StopIndex::new(line, stop)
    }

    /// The stop at `at`, if it's in the display as set.
    pub fn stop(&self, at: StopIndex) -> Option<&Stop> {
        self.lines.get(at.line)?.stops.get(at.stop)
    }

    /// The place the stop at `at` stands for: upstream if the stop after
    /// it, on its line or the next, is the same offset's other place.
    pub fn place(&self, at: StopIndex) -> Option<Place> {
        let offset = self.stop(at)?.offset;
        let twin = at.next(self).and_then(|next| self.stop(next));
        Some(if twin.is_some_and(|t| t.offset == offset) {
            Place::upstream(offset)
        } else {
            Place::downstream(offset)
        })
    }

    /// The place `lines` lines up (negative) or down from `at`, nearest
    /// `goal_x` (or the cursor's own x). `None` past the first or last line.
    pub fn place_by_lines(&self, at: Place, lines: isize, goal_x: Option<i64>) -> Option<Place> {
        let i = self.line_of(at) as isize + lines;
        if i < 0 || i >= self.lines.len() as isize {
            return None;
        }
        let x = goal_x.unwrap_or_else(|| self.x_of(at));
        Some(self.nearest_place(i as usize, x))
    }

    /// Where the cursor stands at `at`: a rectangle of no width, the
    /// line's height. The platform decides how thick to draw it.
    pub fn caret_rect(&self, at: Place) -> Rect {
        let line = &self.lines[self.line_of(at)];
        Rect {
            x: self.x_of(at),
            y: line.top,
            width: 0,
            height: line.height,
        }
    }

    /// The place under (`x`, `y`): the caret stop nearest it on the line at
    /// that height.
    pub fn closest_place(&self, x: i64, y: i64) -> Place {
        let i = self
            .lines
            .partition_point(|l| l.top + l.height <= y)
            .min(self.lines.len() - 1);
        self.nearest_place(i, x)
    }

    /// The rectangles covering `range`, one per line it touches. A line the
    /// selection runs on past reaches the display's right edge. The range
    /// starts at its first offset's downstream place and ends at its last
    /// one's upstream place: a selection up to a line's end stays on that
    /// line.
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let (a, b) = (range.start.min(range.end), range.start.max(range.end));
        let a = Place::downstream(a);
        let b = if a.offset == b { a } else { Place::upstream(b) };
        let (first, last) = (self.line_of(a), self.line_of(b));
        (first..=last)
            .filter_map(|i| {
                let line = &self.lines[i];
                let x0 = if i == first { self.x_of(a) } else { 0 };
                let x1 = if i == last { self.x_of(b) } else { self.width };
                (x1 > x0 || a == b).then_some(Rect {
                    x: x0,
                    y: line.top,
                    width: x1 - x0,
                    height: line.height,
                })
            })
            .collect()
    }

    /// Set the whole of `text` (line ends `\n` only), paragraph by
    /// paragraph.
    pub fn set(&mut self, text: &str) {
        let fallback: Vec<&Font> = self.fallback.iter().map(|f| f.as_ref()).collect();
        let style = TextStyle {
            font: &self.font,
            fallback: &fallback,
            language: None,
            size: self.size,
            line_height: self.line_height,
        };
        let paragraph_style = Style {
            strut: style,
            align: Align::Ragged,
        };
        // How wide a space is, measured as a no-break space, which the
        // typesetter keeps.
        let space = {
            let probe = "\u{a0}";
            let runs = [Run {
                len: probe.len(),
                style,
            }];
            set_paragraph(probe, &runs, &paragraph_style, &[i64::MAX / 4]).lines[0].width
        };
        let mut lines = Vec::new();
        let mut top = 0i64;
        let mut start = 0usize;
        for (k, text) in text.split('\n').enumerate() {
            if k > 0 {
                start += 1; // the line end
            }
            let graphemes: Vec<usize> = text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .chain(std::iter::once(text.len()))
                .collect();
            let runs = [Run {
                len: text.len(),
                style,
            }];
            let set = set_paragraph(text, &runs, &paragraph_style, &[self.width]);
            let count = set.lines.len();
            for (i, line) in set.lines.iter().enumerate() {
                let last = i + 1 == count;
                // The line's end is a place on it, unless the break took
                // spaces: they have no room, so that place would be the
                // one before them over again.
                let ends_here = last || !text[..line.text.end].ends_with(is_space);
                let on_line: Vec<usize> = graphemes
                    .iter()
                    .copied()
                    .filter(|&b| {
                        b >= line.text.start
                            && (b < line.text.end || (ends_here && b == line.text.end))
                    })
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
                    stops: on_line
                        .iter()
                        .zip(xs)
                        .map(|(&b, x)| Stop {
                            offset: start + b,
                            x,
                        })
                        .collect(),
                    glyphs: line
                        .glyphs
                        .iter()
                        .map(|g| Glyph {
                            cluster: start + g.cluster,
                            ..*g
                        })
                        .collect(),
                });
            }
            top += set.height;
            start += text.len();
        }
        self.lines = lines;
        self.height = top;
    }

    /// The line `at` is on: the last starting at or before its offset, or
    /// for the upstream place of a line's start, the line before, if that
    /// line ends in the same offset.
    fn line_of(&self, at: Place) -> usize {
        let i = self
            .lines
            .partition_point(|l| l.start <= at.offset)
            .saturating_sub(1);
        let ends = |l: &Line| l.stops.last().is_some_and(|s| s.offset == at.offset);
        if at.affinity == Affinity::Upstream
            && i > 0
            && self.lines[i].start == at.offset
            && ends(&self.lines[i - 1])
        {
            i - 1
        } else {
            i
        }
    }

    /// The x of `at` on its line.
    fn x_of(&self, at: Place) -> i64 {
        self.stop(self.stop_index(at)).map_or(0, |s| s.x)
    }

    /// The place of the stop nearest `x` on line `i`; of two as near, the
    /// earlier in the text.
    fn nearest_place(&self, i: usize, x: i64) -> Place {
        let nearest = self.lines[i]
            .stops
            .iter()
            .enumerate()
            .min_by_key(|(_, s)| (s.x - x).abs())
            .and_then(|(stop, _)| self.place(StopIndex::new(i, stop)));
        nearest.unwrap_or(Place::downstream(self.lines[i].start))
    }
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
        let after = if j < xs.len() {
            xs[j].unwrap_or(width)
        } else {
            width
        };
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

    fn display(text: &str, width: i64) -> Display {
        let inter =
            Font::from_bytes(include_bytes!("../fonts/Inter-Regular.ttf").to_vec()).unwrap();
        let mut d = Display::new(Arc::new(inter), vec![], 17 * PT, 24 * PT, width * PT);
        d.set(text);
        d
    }

    /// Every grapheme boundary in `text`, the end among them.
    fn boundaries(text: &str) -> Vec<usize> {
        text.grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .collect()
    }

    /// Every stop's place, in order.
    fn places(d: &Display) -> Vec<Place> {
        let mut at = d.stop_index(Place::downstream(0));
        let mut all = vec![d.place(at).unwrap()];
        while let Some(next) = at.next(d) {
            at = next;
            all.push(d.place(at).unwrap());
        }
        all
    }

    fn down(offset: usize) -> Place {
        Place::downstream(offset)
    }

    fn up(offset: usize) -> Place {
        Place::upstream(offset)
    }

    #[test]
    fn stops_never_fall_inside_a_cluster() {
        let text = "naïve e\u{301} 😀 caf\u{e9} office ffi\nsecond paragraph";
        let d = display(text, 80);
        let boundaries = boundaries(text);
        assert!(d.lines.len() > 3);
        for line in &d.lines {
            for s in &line.stops {
                assert!(
                    boundaries.binary_search(&s.offset).is_ok(),
                    "stop {} inside a cluster",
                    s.offset
                );
            }
            assert!(
                line.stops
                    .windows(2)
                    .all(|w| w[0].offset < w[1].offset && w[0].x <= w[1].x)
            );
        }
        // Every boundary is a stop, and only at a line's end twice.
        let mut all: Vec<usize> = d
            .lines()
            .iter()
            .flat_map(|l| l.stops.iter().map(|s| s.offset))
            .collect();
        all.dedup();
        assert_eq!(all, boundaries);
    }

    #[test]
    fn hit_and_caret_round_trip() {
        // Breaks at spaces and after hyphens both.
        let text = "Take up and read, take-up-and-read.\nTolle lege.";
        let d = display(text, 120);
        for p in places(&d) {
            let c = d.caret_rect(p);
            assert_eq!(d.closest_place(c.x, c.y + c.height / 2), p, "{p:?}");
        }
    }

    #[test]
    fn up_and_down_keep_the_column() {
        let d = display("aaaa aaaa aaaa aaaa aaaa aaaa aaaa", 60);
        let x = d.caret_rect(down(2)).x;
        let below = d.place_by_lines(down(2), 1, Some(x)).unwrap();
        assert_eq!(d.place_by_lines(below, -1, Some(x)), Some(down(2)));
        assert_eq!(d.place_by_lines(down(2), -1, None), None);
    }

    #[test]
    fn an_empty_text_is_one_line_with_one_stop() {
        let d = display("", 300);
        assert_eq!(d.lines().len(), 1);
        assert_eq!(d.lines()[0].stops, [Stop { offset: 0, x: 0 }]);
        let rect = Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 24 * PT,
        };
        assert_eq!(d.caret_rect(down(0)), rect);
        assert_eq!(d.caret_rect(up(0)), rect);
        assert_eq!(d.height(), 24 * PT);
    }

    #[test]
    fn a_space_at_the_end_moves_the_cursor() {
        // Typing a space after a word, and deleting it, has to show.
        let d = display("hello  ", 300);
        let x = |o| d.caret_rect(down(o)).x;
        assert!(x(5) < x(6) && x(6) < x(7), "{} {} {}", x(5), x(6), x(7));
        // And each place can be touched.
        for o in [5, 6, 7] {
            assert_eq!(d.closest_place(x(o), 0), down(o));
        }
        // The same at the end of any paragraph, not only the last.
        let d = display("one \ntwo", 300);
        assert!(d.caret_rect(down(3)).x < d.caret_rect(down(4)).x);
    }

    #[test]
    fn a_break_with_no_space_has_two_places() {
        // Nowhere to break but after the hyphens.
        let text = "aaaa-bbbb-cccc-dddd";
        let d = display(text, 60);
        assert!(d.lines().len() > 1);
        for (i, pair) in d.lines().windows(2).enumerate() {
            let (line, next) = (&pair[0], &pair[1]);
            let b = next.start;
            assert!(text[..b].ends_with('-'));
            // The break is the last stop of one line and the first of the
            // next.
            assert_eq!(line.stops.last().unwrap().offset, b);
            assert_eq!(next.stops[0].offset, b);
            // Upstream is after the hyphen, at the line's end; downstream
            // is the next line's start.
            let (before, after) = (d.caret_rect(up(b)), d.caret_rect(down(b)));
            assert_eq!((before.x, before.y), (line.width, line.top));
            assert_eq!((after.x, after.y), (0, next.top));
            assert!(before.x > d.caret_rect(down(b - 1)).x);
            // A touch past the line's end is the place at its end.
            assert_eq!(d.closest_place(d.width(), line.top), up(b));
            assert_eq!(d.closest_place(0, next.top), down(b));
            // From one to the other is a step that keeps the offset.
            let at = d.stop_index(up(b));
            assert_eq!(at, StopIndex::new(i, line.stops.len() - 1));
            assert_eq!(at.next(&d), Some(d.stop_index(down(b))));
            // A selection up to the break stays on its line.
            let rects = d.selection_rects(Range { start: line.start, end: b });
            assert_eq!(rects.len(), 1);
            assert_eq!((rects[0].y, rects[0].width), (line.top, line.width));
            // Down from the line's end, and back up to it.
            let below = d.place_by_lines(up(b), 1, None).unwrap();
            assert_eq!(d.caret_rect(below).y, next.top);
            assert_eq!(d.place_by_lines(below, -1, Some(line.width)), Some(up(b)));
        }
        // Everywhere else has one place, whichever is asked for.
        let breaks: Vec<usize> = d.lines()[1..].iter().map(|l| l.start).collect();
        for o in boundaries(text) {
            if !breaks.contains(&o) {
                assert_eq!(d.caret_rect(up(o)), d.caret_rect(down(o)), "offset {o}");
                assert_eq!(d.place(d.stop_index(up(o))), Some(down(o)));
            }
        }
    }

    #[test]
    fn a_break_at_a_space_has_one_place() {
        let d = display("aaaa bbbb", 60);
        assert_eq!(d.lines().len(), 2);
        // The space takes no room, so the place after it is the next
        // line's start and nowhere else.
        assert_eq!(d.lines()[0].stops.last().unwrap().offset, 4);
        assert_eq!(d.lines()[1].stops[0].offset, 5);
        assert_eq!(d.caret_rect(up(5)), d.caret_rect(down(5)));
        assert_eq!(d.closest_place(d.width(), 0), down(4));
        assert_eq!(d.selection_rects(Range { start: 0, end: 5 }).len(), 1);
    }

    #[test]
    fn two_stops_on_one_line_are_two_places() {
        // What a change of direction will make: offset 2 is the end of
        // what runs right and, further along, the start of what runs
        // left. Nothing sets a line like this yet.
        let mut d = display("", 300);
        let stop = |offset, x| Stop { offset, x };
        d.lines[0].stops = vec![
            stop(0, 0),
            stop(1, 10),
            stop(2, 20),
            stop(2, 50),
            stop(3, 40),
            stop(4, 30),
        ];
        assert_eq!(d.caret_rect(up(2)).x, 20);
        assert_eq!(d.caret_rect(down(2)).x, 50);
        assert_eq!(d.closest_place(22, 0), up(2));
        assert_eq!(d.closest_place(48, 0), down(2));
        assert_eq!(d.closest_place(33, 0), down(4));
        assert_eq!(d.place(StopIndex::new(0, 2)), Some(up(2)));
        assert_eq!(d.place(StopIndex::new(0, 3)), Some(down(2)));
        let all: Vec<Place> = places(&d);
        assert_eq!(all, [down(0), down(1), up(2), down(2), down(3), down(4)]);
    }

    #[test]
    fn steps_through_every_stop_and_back() {
        let text = "Take up and read, take-up-and-read.\n\nTolle lege.";
        let d = display(text, 120);
        assert!(d.lines().len() > 3);
        let mut at = d.stop_index(down(0));
        let mut forward = vec![d.stop(at).unwrap().offset];
        while let Some(next) = at.next(&d) {
            at = next;
            forward.push(d.stop(at).unwrap().offset);
        }
        assert_eq!(at.next(&d), None);
        let mut back = vec![d.stop(at).unwrap().offset];
        while let Some(prev) = at.prev(&d) {
            at = prev;
            back.push(d.stop(at).unwrap().offset);
        }
        back.reverse();
        assert_eq!(back, forward);
        assert_eq!(at, StopIndex::new(0, 0));
        // Every boundary once, but for a break with no space, twice.
        assert!(forward.windows(2).any(|w| w[0] == w[1]));
        forward.dedup();
        assert_eq!(forward, boundaries(text));
    }

    #[test]
    fn every_offset_finds_its_stop() {
        let text = "naïve e\u{301} 😀 office\nsecond well-met";
        let d = display(text, 80);
        for o in boundaries(text) {
            for p in [up(o), down(o)] {
                assert_eq!(d.stop(d.stop_index(p)).unwrap().offset, o, "{p:?}");
            }
        }
        // Inside a cluster (the accent at 8, on the e at 7) is the stop
        // before it.
        assert_eq!(d.stop(d.stop_index(up(8))).unwrap().offset, 7);
        assert_eq!(d.stop(d.stop_index(down(8))).unwrap().offset, 7);
    }
}
