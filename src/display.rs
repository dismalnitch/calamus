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
//! - No affinity: the offset at a soft line break is always the start of
//!   the next line.
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
/// [`Display::set`]. Steps go in text order for now; when right to left
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

    /// Where `offset`'s stop is: on its line, the last stop at or before
    /// it.
    pub fn stop_index(&self, offset: usize) -> StopIndex {
        let line = self.line_of(offset);
        let stops = &self.lines[line].stops;
        let stop = stops.partition_point(|s| s.offset <= offset).saturating_sub(1);
        StopIndex::new(line, stop)
    }

    /// The stop at `at`, if it's in the display as set.
    pub fn stop(&self, at: StopIndex) -> Option<&Stop> {
        self.lines.get(at.line)?.stops.get(at.stop)
    }

    /// The place `lines` lines up (negative) or down from `offset`, nearest
    /// `goal_x` (or the cursor's own x). `None` past the first or last line.
    pub fn offset_by_lines(
        &self,
        offset: usize,
        lines: isize,
        goal_x: Option<i64>,
    ) -> Option<usize> {
        let i = self.line_of(offset) as isize + lines;
        if i < 0 || i >= self.lines.len() as isize {
            return None;
        }
        let x = goal_x.unwrap_or_else(|| self.x_of(offset));
        Some(nearest_stop(&self.lines[i as usize], x))
    }

    /// Where the cursor stands at `offset`: a rectangle of no width, the
    /// line's height. The platform decides how thick to draw it.
    pub fn caret_rect(&self, offset: usize) -> Rect {
        let line = &self.lines[self.line_of(offset)];
        Rect {
            x: self.x_of(offset),
            y: line.top,
            width: 0,
            height: line.height,
        }
    }

    /// The place under (`x`, `y`): the caret stop nearest it on the line at
    /// that height.
    pub fn closest_offset(&self, x: i64, y: i64) -> usize {
        let i = self
            .lines
            .partition_point(|l| l.top + l.height <= y)
            .min(self.lines.len() - 1);
        nearest_stop(&self.lines[i], x)
    }

    /// The rectangles covering `range`, one per line it touches. A line the
    /// selection runs on past reaches the display's right edge.
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let (a, b) = (range.start.min(range.end), range.start.max(range.end));
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
                let on_line: Vec<usize> = graphemes
                    .iter()
                    .copied()
                    .filter(|&b| {
                        b >= line.text.start && (b < line.text.end || (last && b == line.text.end))
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

    /// The line `offset` is on (a soft break's offset is on the next).
    fn line_of(&self, offset: usize) -> usize {
        self.lines
            .partition_point(|l| l.start <= offset)
            .saturating_sub(1)
    }

    /// The x of `offset` on its line: its stop's, or the nearest stop
    /// before it.
    fn x_of(&self, offset: usize) -> i64 {
        let line = &self.lines[self.line_of(offset)];
        let i = line.stops.partition_point(|s| s.offset <= offset);
        line.stops.get(i.saturating_sub(1)).map_or(0, |s| s.x)
    }
}

fn nearest_stop(line: &Line, x: i64) -> usize {
    line.stops
        .iter()
        .min_by_key(|s| (s.x - x).abs())
        .map_or(line.start, |s| s.offset)
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
        // Every boundary is a stop on exactly one line.
        let all: Vec<usize> = d
            .lines()
            .iter()
            .flat_map(|l| l.stops.iter().map(|s| s.offset))
            .collect();
        assert_eq!(all, boundaries);
    }

    #[test]
    fn hit_and_caret_round_trip() {
        let text = "Take up and read, take up and read.\nTolle lege.";
        let d = display(text, 120);
        for o in boundaries(text) {
            let c = d.caret_rect(o);
            assert_eq!(d.closest_offset(c.x, c.y + c.height / 2), o, "offset {o}");
        }
    }

    #[test]
    fn up_and_down_keep_the_column() {
        let d = display("aaaa aaaa aaaa aaaa aaaa aaaa aaaa", 60);
        let x = d.caret_rect(2).x;
        let down = d.offset_by_lines(2, 1, Some(x)).unwrap();
        assert_eq!(d.offset_by_lines(down, -1, Some(x)), Some(2));
        assert_eq!(d.offset_by_lines(2, -1, None), None);
    }

    #[test]
    fn an_empty_text_is_one_line_with_one_stop() {
        let d = display("", 300);
        assert_eq!(d.lines().len(), 1);
        assert_eq!(d.lines()[0].stops, [Stop { offset: 0, x: 0 }]);
        assert_eq!(
            d.caret_rect(0),
            Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 24 * PT
            }
        );
        assert_eq!(d.height(), 24 * PT);
    }

    #[test]
    fn a_space_at_the_end_moves_the_cursor() {
        // Typing a space after a word, and deleting it, has to show.
        let d = display("hello  ", 300);
        let x = |o| d.caret_rect(o).x;
        assert!(x(5) < x(6) && x(6) < x(7), "{} {} {}", x(5), x(6), x(7));
        // And each place can be touched.
        for o in [5, 6, 7] {
            assert_eq!(d.closest_offset(x(o), 0), o);
        }
        // The same at the end of any paragraph, not only the last.
        let d = display("one \ntwo", 300);
        assert!(d.caret_rect(3).x < d.caret_rect(4).x);
    }

    #[test]
    fn steps_through_every_stop_and_back() {
        let text = "Take up and read, take up and read.\n\nTolle lege.";
        let d = display(text, 120);
        assert!(d.lines().len() > 3);
        let mut at = d.stop_index(0);
        let mut forward = vec![d.stop(at).unwrap().offset];
        while let Some(next) = at.next(&d) {
            at = next;
            forward.push(d.stop(at).unwrap().offset);
        }
        assert_eq!(forward, boundaries(text));
        assert_eq!(at.next(&d), None);
        let mut back = vec![d.stop(at).unwrap().offset];
        while let Some(prev) = at.prev(&d) {
            at = prev;
            back.push(d.stop(at).unwrap().offset);
        }
        back.reverse();
        assert_eq!(back, forward);
        assert_eq!(at, StopIndex::new(0, 0));
    }

    #[test]
    fn every_offset_finds_its_stop() {
        let text = "naïve e\u{301} 😀 office\nsecond";
        let d = display(text, 80);
        for o in boundaries(text) {
            assert_eq!(d.stop(d.stop_index(o)).unwrap().offset, o, "offset {o}");
        }
    }
}
