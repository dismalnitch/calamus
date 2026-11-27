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
//! - Text runs both ways. A paragraph's stretches that run against it are
//!   the buffer's isolates, handed in with the text; inside them
//!   Unicode's bidirectional algorithm decides ([`bidi`]).
//!   Stops are kept in the text's order and, beside that, in the order
//!   they are on the page ([`Line::visual`]). Where the direction
//!   changes, an offset is two stops.
//! - The whole text is set again on every change, paragraph by paragraph.
//!   No cache, no incremental layout.
//! - One font and its fallbacks, ragged, no styles.
//! - Spaces before a paragraph's end are set like any text, so one just
//!   typed shows where its text puts it. Spaces at a soft break still
//!   take no room: the cursor goes to the next line.

use std::range::Range;
use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation;

pub use crate::caret::Affinity;
use crate::bidi::{self, Stretch};
use crate::buffer::{Isolate, Kind};
use crate::layout::{Align, Font, Glyph, Run, Style, TextStyle, is_space, set_paragraph_bidi};

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
    /// Which paragraph of the text it's a line of, counting from 0.
    pub paragraph: usize,
    /// The size its text is set at, in scaled points: a heading's is
    /// larger than the display's.
    pub size: i64,
    /// Every place the cursor can stand on the line, in text order. An
    /// offset with two places has two stops, the upstream one first: the
    /// end of a line broken with no space is that line's last stop and the
    /// next line's first. A line broken at spaces doesn't end in one: the
    /// spaces take no room, so the place after them is the next line's
    /// start only.
    pub stops: Vec<Stop>,
    /// The line's stops in the order they are on the page, from the left:
    /// indices into `stops`. Where text runs right to left, that's the
    /// reverse of the text's order. Two stops at one x are in the order
    /// of the text either side of them: the one that's the end of what's
    /// on the left first.
    pub visual: Vec<usize>,
    /// For each stop, which stretch of the line it's a stop of: text at
    /// one level, side by side on the page. Two stops at one x in
    /// different stretches are the two sides of a change of direction.
    pub stretch: Vec<usize>,
    /// Whether the line's paragraph runs right to left: it starts at the
    /// right edge, and its next line is reached off its left end.
    pub rtl: bool,
    /// The glyphs to draw, from the left: `x` from the line's start, `y`
    /// down from the baseline, `cluster` a byte offset into the whole
    /// text.
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
    /// What each paragraph is, in order; plain past the end.
    kinds: Vec<Kind>,
    /// Whether the text opens with a break that isn't shown: the one a
    /// document starts with so its first paragraph can have a kind.
    opening_hidden: bool,
}

/// How much larger than the text a kind is set, as a fraction.
fn scale(kind: Kind) -> (i64, i64) {
    match kind {
        Kind::Heading(1) => (3, 2),
        Kind::Heading(2) => (5, 4),
        Kind::Heading(_) => (9, 8),
        _ => (1, 1),
    }
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
            kinds: Vec::new(),
            opening_hidden: false,
        }
    }

    /// What each paragraph is, for the next [`Display::set`]: headings
    /// are set larger, with room above. With `opening_hidden`, a text
    /// that opens with a break doesn't show the empty paragraph before
    /// it, and offset 0 has no place.
    pub fn set_kinds(&mut self, kinds: Vec<Kind>, opening_hidden: bool) {
        self.kinds = kinds;
        self.opening_hidden = opening_hidden;
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
    pub fn set_width(&mut self, width: i64, text: &str, isolates: &[Isolate]) {
        if width != self.width {
            self.width = width;
            self.set(text, isolates);
        }
    }

    /// The other place at the very spot `at` is on the page, where there
    /// is one: at a change of direction, the end of the text on one side
    /// and the edge of the text on the other share an x. It's a different
    /// offset. `None` anywhere else.
    pub fn twin(&self, at: Place) -> Option<Place> {
        let here = self.stop_index(at);
        let line = self.lines.get(here.line)?;
        let (x, stretch) = (line.stops.get(here.stop)?.x, *line.stretch.get(here.stop)?);
        let other = (0..line.stops.len())
            .find(|&i| i != here.stop && line.stops[i].x == x && line.stretch.get(i) != Some(&stretch))?;
        self.place(StopIndex::new(here.line, other))
    }

    /// The place next to `at` on the page, to its right or its left:
    /// what an arrow key moves to. Off a line's end, it's the line after
    /// or before, as the paragraph reads. `None` past the text's ends.
    ///
    /// Where text changes direction, two places share an x: the end of
    /// what's on one side and the edge of what's on the other. A step
    /// from one to the other moves nothing on the page, and changes
    /// which text the cursor is with.
    pub fn place_beside(&self, at: Place, right: bool) -> Option<Place> {
        let here = self.stop_index(at);
        let line = &self.lines[here.line];
        let v = line.visual.iter().position(|&s| s == here.stop)?;
        let beside = if right { v + 1 } else { v.wrapping_sub(1) };
        if let Some(&stop) = line.visual.get(beside) {
            return self.place(StopIndex::new(here.line, stop));
        }
        // Off the end: on to the next line if that's the way the
        // paragraph reads, else back to the one before; and onto the end
        // of it that's met first.
        let onward = right != line.rtl;
        let to = if onward { here.line + 1 } else { here.line.checked_sub(1)? };
        let next = self.lines.get(to)?;
        let first = onward != next.rtl;
        let stop = if first { next.visual.first() } else { next.visual.last() };
        self.place(StopIndex::new(to, *stop?))
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

    /// The rectangles covering `range`: one per line it touches where the
    /// text runs one way, more where a line's selected text isn't side by
    /// side on the page. A line the selection runs on past reaches the
    /// display's edge. The range starts at its first offset's downstream
    /// place and ends at its last one's upstream place: a selection up to
    /// a line's end stays on that line.
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let (a, b) = (range.start.min(range.end), range.start.max(range.end));
        if a == b {
            return vec![self.caret_rect(Place::downstream(a))];
        }
        let first = self.line_of(Place::downstream(a));
        let last = self.line_of(Place::upstream(b));
        let mut rects = Vec::new();
        for i in first..=last {
            let line = &self.lines[i];
            // Between one stop and the next in the text is a cluster's
            // room on the page; between two stops at one offset is
            // nothing.
            let mut spans: Vec<(i64, i64)> = line
                .stops
                .windows(2)
                .filter(|w| w[0].offset < w[1].offset && a <= w[0].offset && w[1].offset <= b)
                .map(|w| (w[0].x.min(w[1].x), w[0].x.max(w[1].x)))
                .collect();
            if i < last {
                // It runs on: out to the edge the paragraph reads toward.
                let (lo, hi) = extent(line);
                spans.push(if line.rtl { (0, lo) } else { (hi, self.width.max(hi)) });
            }
            spans.sort_unstable();
            let mut joined: Vec<(i64, i64)> = Vec::new();
            for (lo, hi) in spans {
                match joined.last_mut() {
                    Some(last) if lo <= last.1 => last.1 = last.1.max(hi),
                    _ => joined.push((lo, hi)),
                }
            }
            rects.extend(joined.into_iter().filter(|(lo, hi)| hi > lo).map(|(lo, hi)| Rect {
                x: lo,
                y: line.top,
                width: hi - lo,
                height: line.height,
            }));
        }
        rects
    }

    /// Set the whole of `text` (line ends `\n` only), paragraph by
    /// paragraph, with its `isolates`: the stretches the buffer keeps as
    /// set in a direction of their own.
    pub fn set(&mut self, text: &str, isolates: &[Isolate]) {
        let fallback: Vec<&Font> = self.fallback.iter().map(|f| f.as_ref()).collect();
        let style = TextStyle {
            font: &self.font,
            fallback: &fallback,
            language: None,
            size: self.size,
            line_height: self.line_height,
        };
        let mut lines: Vec<Line> = Vec::new();
        let mut top = 0i64;
        let mut start = 0usize;
        for (k, text) in text.split('\n').enumerate() {
            if k > 0 {
                start += 1; // the line end
            }
            if k == 0 && self.opening_hidden && text.is_empty() && self.kinds.len() > 1 {
                continue;
            }
            let kind = self.kinds.get(k).copied().unwrap_or(Kind::Text);
            let (up, down) = scale(kind);
            let style = TextStyle {
                size: style.size * up / down,
                line_height: style.line_height * up / down,
                ..style
            };
            let paragraph_style = Style {
                strut: style,
                align: Align::Ragged,
            };
            // Room above a heading, unless it opens the text.
            if kind.is_heading() && !lines.is_empty() {
                top += self.line_height / 2;
            }
            let graphemes: Vec<usize> = text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .chain(std::iter::once(text.len()))
                .collect();
            // The paragraph's isolates, in its own bytes.
            let within: Vec<Stretch> = isolates
                .iter()
                .filter(|i| start <= i.range.start && i.range.end <= start + text.len())
                .map(|i| Stretch {
                    range: i.range.start - start..i.range.end - start,
                    direction: i.direction,
                })
                .collect();
            let base = bidi::base(text);
            let levels = bidi::levels(text, &within);
            let runs = [Run {
                len: text.len(),
                style,
            }];
            let set = set_paragraph_bidi(text, &runs, &paragraph_style, &[self.width], base, &levels, true, true);
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
                // Where what's drawn ends. A line broken at spaces doesn't
                // draw them. A paragraph's last line draws everything: the
                // spaces at its end are kept, so one just typed shows.
                let drawn = if last {
                    line.text.end
                } else {
                    line.text.start + text[line.text.start..line.text.end].trim_end_matches(is_space).len()
                };
                let (stops, visual, stretch) =
                    line_stops(&on_line, drawn, &line.glyphs, &levels, base, self.width);
                lines.push(Line {
                    top: top + line.top,
                    height: line.height,
                    baseline: top + line.baseline,
                    width: line.width,
                    start: start + line.text.start,
                    paragraph: k,
                    size: style.size,
                    stops: stops
                        .into_iter()
                        .map(|s| Stop {
                            offset: start + s.offset,
                            x: s.x,
                        })
                        .collect(),
                    visual,
                    stretch,
                    rtl: base % 2 == 1,
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

/// How far left and right a line's stops reach.
fn extent(line: &Line) -> (i64, i64) {
    let xs = || line.stops.iter().map(|s| s.x);
    (xs().min().unwrap_or(0), xs().max().unwrap_or(0))
}

/// A line's caret stops, in the text's order, and the order they're in on
/// the page.
///
/// `boundaries` are the grapheme boundaries on the line (byte offsets into
/// the paragraph, in order), `drawn` where its glyphs' text ends, and
/// `levels` each byte's level. Offsets in the stops are the paragraph's.
///
/// The line is taken a stretch at a time: the text at one level, which is
/// side by side on the page. Running left to right, a boundary is at its
/// cluster's left edge and the stretch ends at its right; running right to
/// left, at the cluster's right edge, ending at the stretch's left. A
/// boundary with no glyph of its own (inside a ligature) is shared out
/// evenly between its neighbours. Where one stretch ends and the next
/// begins, that offset has both places: two stops, unless they're at the
/// same x.
///
/// Past `drawn` there's nothing drawn (the spaces a line broke at): each
/// boundary there is where the text before it ends. And the paragraph's
/// own end is a place at its own level: after Hebrew that ends an English
/// paragraph, the end is both the Hebrew's left edge and, on its right,
/// where English goes on.
fn line_stops(
    boundaries: &[usize],
    drawn: usize,
    glyphs: &[Glyph],
    levels: &[u8],
    base: u8,
    width: i64,
) -> (Vec<Stop>, Vec<usize>, Vec<usize>) {
    let level_at = |b: usize| levels.get(b).copied().unwrap_or(base);
    let mut stops: Vec<Stop> = Vec::new();
    // Which stretch each stop is of, counted as they come.
    let mut stretch: Vec<usize> = Vec::new();
    let mut stretches = 0;
    // For each stop, where its stretch is on the page and which way it
    // runs: what orders two stops at one x.
    let mut keys: Vec<(i64, bool)> = Vec::new();
    let content: Vec<usize> = boundaries.iter().copied().filter(|&b| b <= drawn).collect();
    let (mut left, mut right) = (i64::MAX, i64::MIN);
    // The level of the last stretch drawn.
    let mut last_level = None;
    let mut i = 0;
    while i + 1 < content.len() {
        let level = level_at(content[i]);
        let mut j = i + 1;
        while j + 1 < content.len() && level_at(content[j]) == level {
            j += 1;
        }
        let (from, to) = (content[i], content[j]);
        let rtl = level % 2 == 1;
        let own = || glyphs.iter().filter(move |g| from <= g.cluster && g.cluster < to);
        let x0 = own().map(|g| g.x).min();
        let x1 = own().map(|g| g.x + g.advance).max();
        let (Some(x0), Some(x1)) = (x0, x1) else {
            i = j;
            continue;
        };
        (left, right) = (left.min(x0), right.max(x1));
        // Each boundary's x where a glyph says: its cluster's near edge.
        let mut xs: Vec<Option<i64>> = content[i..=j]
            .iter()
            .map(|&b| {
                let at = glyphs.iter().filter(|g| g.cluster == b);
                if rtl { at.map(|g| g.x + g.advance).max() } else { at.map(|g| g.x).min() }
            })
            .collect();
        xs[0] = Some(if rtl { x1 } else { x0 });
        *xs.last_mut().expect("two at least") = Some(if rtl { x0 } else { x1 });
        // The rest evenly between the ones either side.
        let mut k = 0;
        while k < xs.len() {
            if xs[k].is_some() {
                k += 1;
                continue;
            }
            let before = xs[k - 1].expect("filled");
            let next = (k..xs.len()).find(|&n| xs[n].is_some()).expect("the last is filled");
            let after = xs[next].expect("found");
            let gaps = (next - k + 1) as i64;
            for (n, x) in xs[k..next].iter_mut().enumerate() {
                *x = Some(before + (after - before) * (n as i64 + 1) / gaps);
            }
            k = next;
        }
        for (&offset, x) in content[i..=j].iter().zip(xs) {
            let stop = Stop { offset, x: x.expect("filled") };
            // The end of the stretch before, at the same place: one stop.
            if stops.last() != Some(&stop) {
                stops.push(stop);
                keys.push((x0, rtl));
                stretch.push(stretches);
            }
        }
        stretches += 1;
        last_level = Some(level);
        i = j;
    }
    // What isn't drawn: on from where the text before it ends if it runs
    // the same way, else from the line's edge that way.
    let tail_level = level_at(drawn.min(levels.len().saturating_sub(1)));
    let tail_level = if drawn < levels.len() { tail_level } else { base };
    let rtl = tail_level % 2 == 1;
    if left > right {
        // Nothing drawn at all: the line's start.
        let x = if base % 2 == 1 { width } else { 0 };
        (left, right) = (x, x);
    }
    // Only at the same level: English in a Hebrew phrase that ends an
    // English paragraph runs the paragraph's way, and still isn't where
    // the paragraph goes on.
    let follows = last_level == Some(tail_level) && stops.last().is_some_and(|s| s.offset == drawn);
    let from = match stops.last() {
        Some(s) if follows => s.x,
        _ => if rtl { left } else { right },
    };
    for &offset in boundaries {
        if offset < drawn || (offset == drawn && follows) {
            continue;
        }
        let stop = Stop { offset, x: from };
        if stops.last() != Some(&stop) {
            stops.push(stop);
            keys.push((from, rtl));
            stretch.push(stretches);
        }
    }
    // Nothing the cursor can stand at is off the page: text too long to
    // break (spaces typed on and on, a word wider than the line) runs
    // out past the edge, and the cursor waits at it.
    for stop in &mut stops {
        stop.x = stop.x.clamp(0, width.max(0));
    }
    // On the page: by x; at one x, the stretch further left first; in one
    // stretch (clusters with no width), the way it runs.
    let mut visual: Vec<usize> = (0..stops.len()).collect();
    visual.sort_by_key(|&i| {
        let (at, rtl) = keys[i];
        (stops[i].x, at, if rtl { -(i as i64) } else { i as i64 })
    });
    (stops, visual, stretch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::PT;

    fn display(text: &str, width: i64) -> Display {
        let inter =
            Font::from_bytes(include_bytes!("../fonts/Inter-Regular.ttf").to_vec()).unwrap();
        let mut d = Display::new(Arc::new(inter), vec![], 17 * PT, 24 * PT, width * PT);
        d.set(text, &[]);
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

    fn hebrew(text: &str, isolates: &[(std::ops::Range<usize>, crate::buffer::Direction)], width: i64) -> Display {
        let inter =
            Font::from_bytes(include_bytes!("../fonts/Inter-Regular.ttf").to_vec()).unwrap();
        let noto =
            Font::from_bytes(include_bytes!("../fonts/NotoSansHebrew-Regular.ttf").to_vec()).unwrap();
        let mut buffer = crate::buffer::Buffer::new();
        buffer.insert(0, text).unwrap();
        for (range, direction) in isolates {
            buffer.mark_isolate(Range { start: range.start, end: range.end }, *direction).unwrap();
        }
        let mut d = Display::new(Arc::new(inter), vec![Arc::new(noto)], 17 * PT, 24 * PT, width * PT);
        d.set(text, &buffer.isolates());
        d
    }

    /// The places on the first line, from the left.
    fn across(d: &Display) -> Vec<Place> {
        let line = &d.lines()[0];
        line.visual.iter().map(|&s| d.place(StopIndex::new(0, s)).unwrap()).collect()
    }

    #[test]
    fn hebrew_in_english_has_two_places_at_each_edge() {
        // אבג is bytes 4..10: its letters start at 4, 6 and 8.
        let d = hebrew("abc אבג def", &[], 300);
        let x = |p: Place| d.caret_rect(p).x;
        // Before the Hebrew in the text: after the space, on its left; or
        // before its first letter, on its right.
        assert!(x(up(4)) < x(down(4)));
        // After it: after its last letter, on its left; or before the
        // space that follows, on its right.
        assert!(x(up(10)) < x(down(10)));
        assert_eq!((x(up(4)), x(down(4))), (x(up(10)), x(down(10))));
        // Its letters run from the right.
        assert!(x(down(4)) > x(down(6)) && x(down(6)) > x(down(8)) && x(down(8)) > x(up(10)));
        // Everywhere else has one place.
        for o in [0, 1, 2, 3, 6, 8, 11, 12, 13, 14] {
            assert_eq!(x(up(o)), x(down(o)), "offset {o}");
        }
        // Across the page: the English, into the Hebrew at its left edge,
        // out at its right, the English after.
        assert_eq!(
            across(&d),
            [
                down(0), down(1), down(2), down(3), up(4), up(10), down(8), down(6), down(4),
                down(10), down(11), down(12), down(13), down(14)
            ]
        );
    }

    #[test]
    fn two_places_share_a_spot_where_direction_changes() {
        // Hebrew at the end of an English paragraph: אבג is 4..10.
        let d = hebrew("abc אבג", &[], 300);
        // Its left edge: the end of the English before it, and its own
        // end.
        assert_eq!(d.twin(up(4)), Some(up(10)));
        assert_eq!(d.twin(up(10)), Some(up(4)));
        // Its right edge: before its first letter, and the paragraph's
        // end, where English goes on.
        assert_eq!(d.twin(down(4)), Some(down(10)));
        assert_eq!(d.twin(down(10)), Some(down(4)));
        for (a, b) in [(up(4), up(10)), (down(4), down(10))] {
            assert_eq!(d.caret_rect(a), d.caret_rect(b));
        }
        // Nowhere else.
        for o in [0, 1, 2, 3, 6, 8] {
            assert_eq!(d.twin(down(o)), None, "offset {o}");
        }
        assert_eq!(display("plain text", 300).twin(down(5)), None);
    }

    #[test]
    fn a_paragraphs_end_is_its_own_place() {
        // English held in a Hebrew phrase, at the end of an English
        // paragraph: אבג 4..10, a space, abc 11..14, all in the phrase.
        // The English is drawn at the phrase's left, so its end is
        // there; the paragraph's end is at the line's right.
        let d = hebrew("say אבג abc", &[(4..14, crate::buffer::Direction::RightToLeft)], 300);
        let (word_end, paragraph_end) = (d.caret_rect(up(14)).x, d.caret_rect(down(14)).x);
        assert!(word_end < d.caret_rect(up(10)).x);
        assert!(paragraph_end > d.caret_rect(down(4)).x - 1);
        assert_eq!(paragraph_end, d.caret_rect(down(4)).x);
        // The word's end shares its spot with the end of the Hebrew and
        // its space; the paragraph's end with the Hebrew's start.
        assert_eq!(d.twin(up(14)), Some(up(11)));
        assert_eq!(d.twin(down(14)), Some(down(4)));
    }

    #[test]
    fn arrows_go_the_way_they_point() {
        let d = hebrew("abc אבג def", &[], 300);
        let all = across(&d);
        // Rightwards visits every place from the left, and leftwards
        // comes back.
        let mut at = down(0);
        let mut seen = vec![at];
        while let Some(next) = d.place_beside(at, true) {
            at = next;
            seen.push(at);
        }
        assert_eq!(seen, all);
        let mut back = vec![at];
        while let Some(prev) = d.place_beside(at, false) {
            at = prev;
            back.push(at);
        }
        back.reverse();
        assert_eq!(back, all);
        // At the Hebrew's edge a step keeps the cursor's x and changes
        // which text it's with.
        assert_eq!(d.place_beside(up(4), true), Some(up(10)));
        assert_eq!(d.caret_rect(up(4)).x, d.caret_rect(up(10)).x);
        // Inside it, right is back through the text.
        assert_eq!(d.place_beside(down(6), true), Some(down(4)));
        assert_eq!(d.place_beside(down(6), false), Some(down(8)));
    }

    #[test]
    fn a_touch_finds_the_place_it_shows() {
        let d = hebrew("abc אבג def\nאבג abc", &[], 300);
        for line in 0..d.lines().len() {
            for stop in 0..d.lines()[line].stops.len() {
                let p = d.place(StopIndex::new(line, stop)).unwrap();
                let c = d.caret_rect(p);
                let found = d.closest_place(c.x, c.y + c.height / 2);
                assert_eq!(d.caret_rect(found), c, "{p:?} found as {found:?}");
            }
        }
    }

    #[test]
    fn a_hebrew_paragraph_is_set_from_the_right() {
        // אבג 0..6, the space 6, abc 7..10.
        let d = hebrew("אבג abc", &[], 300);
        let line = &d.lines()[0];
        assert!(line.rtl);
        let x = |p: Place| d.caret_rect(p).x;
        // It starts at the right edge and its end is on the left.
        assert_eq!(x(down(0)), 300 * PT);
        assert_eq!(x(down(0)) - x(down(10)), line.width);
        // The English word in it runs left to right, in its place.
        assert!(x(down(7)) < x(down(8)) && x(down(8)) < x(down(9)) && x(down(9)) < x(up(10)));
        assert!(x(up(10)) < x(down(4)));
        // So the text's end has two places: after the word's last letter,
        // and where the paragraph goes on, at its left.
        assert!(x(down(10)) < x(up(10)));
        // Past the right end is the line before; past the left, the next.
        let d = hebrew("אבג\nדהו", &[], 300);
        assert_eq!(d.place_beside(down(0), true), None);
        assert_eq!(d.place_beside(down(6), false), Some(down(7)));
        assert_eq!(d.place_beside(down(7), true), Some(down(6)));
    }

    #[test]
    fn a_selection_over_a_change_of_direction() {
        let d = hebrew("abc אבג def", &[], 300);
        let x = |p: Place| d.caret_rect(p).x;
        let r = |a, b| d.selection_rects(Range { start: a, end: b });
        // Side by side on the page: one rectangle.
        assert_eq!(r(0, 14).len(), 1);
        assert_eq!(r(4, 10).len(), 1);
        assert_eq!((r(4, 10)[0].x, r(4, 10)[0].width), (x(up(10)), x(down(4)) - x(up(10))));
        // "c " and the first Hebrew letter: next to each other in the
        // text, apart on the page.
        let two = r(2, 6);
        assert_eq!(two.len(), 2);
        assert_eq!((two[0].x, two[0].x + two[0].width), (x(down(2)), x(up(4))));
        assert_eq!((two[1].x, two[1].x + two[1].width), (x(down(6)), x(down(4))));
        // Nothing selected is the cursor.
        assert_eq!(r(6, 6), [d.caret_rect(down(6))]);
    }

    #[test]
    fn an_isolate_holds_its_place_on_the_page() {
        // A Latin letter in a Hebrew word: אב 4..8, d 8, ג 9..11.
        let text = "abc אבdג def";
        let x = |d: &Display, o| d.caret_rect(down(o)).x;
        // The characters alone put ג on the far side of the d.
        let loose = hebrew(text, &[], 300);
        assert!(x(&loose, 9) > x(&loose, 8));
        // Its isolate keeps the word in order: ג is left of d, d left of ב.
        let kept = hebrew(text, &[(4..11, crate::buffer::Direction::RightToLeft)], 300);
        assert!(x(&kept, 9) < x(&kept, 6));
        assert!(kept.caret_rect(up(9)).x <= x(&kept, 6));
    }

    #[test]
    fn spaces_at_the_end_sit_where_their_text_puts_them() {
        // English typed after Hebrew in a Hebrew paragraph, then spaces.
        // אבג is 0..6; abc 6..9; the spaces from 9.
        let d = hebrew("אבגabc", &[(6..9, crate::buffer::Direction::LeftToRight)], 300);
        // The Hebrew's left edge is its end; the English runs up to it.
        let hebrew_left = d.caret_rect(up(6)).x;
        let english_left = d.caret_rect(down(6)).x;
        assert!(english_left < hebrew_left);
        assert_eq!(d.caret_rect(up(9)).x, hebrew_left, "the English ends against the Hebrew");
        // With two spaces in the English's isolate: they're between the
        // word and the Hebrew, the word has moved left for them, and the
        // cursor after them is still against the Hebrew, not in it.
        let spaced = hebrew("אבגabc  ", &[(6..11, crate::buffer::Direction::LeftToRight)], 300);
        let x = |o| spaced.caret_rect(up(o)).x;
        assert_eq!(x(11), hebrew_left);
        assert!(x(9) < x(10) && x(10) < x(11));
        assert!(spaced.caret_rect(down(6)).x < english_left);
        // The Hebrew hasn't moved.
        assert_eq!(spaced.caret_rect(down(0)).x, d.caret_rect(down(0)).x);
    }

    #[test]
    fn spaces_at_a_lines_end_never_rewrap_it() {
        // Two Hebrew words and then spaces, on and on. The words stay on
        // their line, where they were; the spaces hang off its end.
        let words = "אבג דהו";
        let before = hebrew(words, &[], 120);
        let after = hebrew(&format!("{words}{}", " ".repeat(200)), &[], 120);
        assert_eq!((before.lines().len(), after.lines().len()), (1, 1));
        for o in [0, 2, 6, 7, 11, 13] {
            assert_eq!(after.caret_rect(down(o)).x, before.caret_rect(down(o)).x, "offset {o}");
        }
        // The cursor after them is at the paragraph's far edge: the left.
        assert_eq!(after.caret_rect(down(13 + 200)).x, 0);
        // The same in English: the word stays, the cursor's at the right.
        let before = display("word", 120);
        let after = display(&format!("word{}", " ".repeat(200)), 120);
        assert_eq!(after.lines().len(), 1);
        assert_eq!(after.caret_rect(down(2)).x, before.caret_rect(down(2)).x);
        assert_eq!(after.caret_rect(down(204)).x, 120 * PT);
    }

    #[test]
    fn the_cursor_never_leaves_the_page() {
        // Spaces typed on and on: they can't break from the word, so the
        // line runs out past the edge. The cursor stops at it.
        let text = format!("word{}", " ".repeat(400));
        let d = display(&text, 120);
        for line in d.lines() {
            for s in &line.stops {
                assert!((0..=120 * PT).contains(&s.x), "{}", s.x);
            }
        }
        assert_eq!(d.caret_rect(down(text.len())).x, 120 * PT);
        // And in a Hebrew paragraph, at the left edge.
        let text = format!("אבג{}", " ".repeat(400));
        let d = hebrew(&text, &[], 120);
        for line in d.lines() {
            for s in &line.stops {
                assert!((0..=120 * PT).contains(&s.x), "{}", s.x);
            }
        }
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
