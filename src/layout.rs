//! Tolege's typesetter: text in, lines of positioned glyphs out, the same on
//! every platform.
//!
//! A paragraph goes through four steps:
//!
//! 1. **Shape** the text with [rustybuzz] (a HarfBuzz port), each [`Run`]
//!    of it in its own font and size: glyphs, advances, kerning, ligatures.
//! 2. **Itemize**: find where lines may break (the UAX #14 rules, from
//!    [unicode-linebreak]) and turn the text into Knuth–Plass items: each
//!    stretch of text between breaks is a box, the spaces after it are glue,
//!    a break with no space (after a hyphen, say) is a penalty, and a line
//!    end (`\n`) is a forced break.
//! 3. **Break** the items into lines with [demerits]: [`Align::Ragged`] fills
//!    each line at its natural width, first fit; [`Align::Justified`] picks
//!    the breaks with the fewest demerits over the whole paragraph
//!    (Knuth–Plass), and falls back to first fit when no set of breaks keeps
//!    every line within the tolerance.
//! 4. **Set** each line: place its glyphs, and for justified lines spread the
//!    slack over its spaces, in proportion to their stretch (or shrink).
//!    A line is as tall as the tallest text on it asks, as in CSS.
//!
//! # Units
//!
//! Every length is an integer number of **scaled points**, TeX's unit:
//! [`PT`] of them make a point. Font units are converted once, glyph by
//! glyph, rounding half away from zero; after that everything is integer
//! arithmetic, so a line's width is exactly the sum of what's on it. Only
//! demerits' adjustment ratio is a float, and it only decides which break a
//! line takes; how far each space stretches is integer arithmetic.
//!
//! # The same lines everywhere
//!
//! Nothing here asks the platform: no system fonts, no platform maths
//! library (`powf`, `exp` and friends can differ in the last bit between
//! platforms), no hash-map ordering. The same text, runs, font files,
//! sizes and widths give the same [`Paragraph`] on every machine.
//!
//! # Fonts and fallback
//!
//! Fonts are handed over as bytes, never asked of the system by name. Each
//! run's style has its font and an ordered list of fallback fonts
//! ([`TextStyle::fallback`]); Idem sets each grapheme cluster in the first
//! of them that has all its characters, and shapes each stretch in one font
//! as a whole. Every [`Glyph`] says which of its run's fonts it's from.
//!
//! When no font has all of a cluster, the first font with its first
//! character takes it, so a letter still shows if only its accent is
//! missing. When none has even that, the cluster is one glyph: the run's
//! font's missing glyph (`.notdef`, glyph id 0, usually a box), however many
//! characters it holds. To draw something better, end the fallback list with
//! a font that has every character, such as Unicode's Last Resort font.
//!
//! Private-use characters are set only in fonts allowed them
//! ([`Font::with_private_use`]), since each font gives them its own meaning.
//!
//! # Not yet
//!
//! Every run sits on the line's baseline: no superscripts or other
//! `vertical-align`. Text runs right to left only where it's told to
//! ([`set_paragraph_bidi`]): which text does is decided elsewhere
//! ([`bidi`](crate::bidi)). No hyphenation inside words: lines break at
//! spaces, after hyphens and wherever else UAX #14 allows. Each stretch of
//! text between breaks is shaped on its own, so nothing kerns across a space
//! or a break.
//!
//! [rustybuzz]: https://docs.rs/rustybuzz
//! [unicode-linebreak]: https://docs.rs/unicode-linebreak
//! [demerits]: https://github.com/Ganderworks/demerits

#![forbid(unsafe_code)]

use std::ops::Range;

use demerits::{INFINITY, Item, Stretch};
use rustybuzz::{Direction, Language, Script, ShapePlan, UnicodeBuffer};
use unicode_linebreak::{BreakClass, BreakOpportunity, break_property, linebreaks};
use unicode_segmentation::UnicodeSegmentation;

/// Scaled points in a point: every length in Idem is in scaled points.
pub const PT: i64 = 65_536;

/// A font file, checked and ready to shape with.
#[derive(Debug, Clone)]
pub struct Font {
    data: Vec<u8>,
    index: u32,
    units_per_em: i32,
    ascender: i32,
    descender: i32,
    private_use: bool,
}

/// Why a font couldn't be loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontError {
    /// The bytes aren't a font rustybuzz can read, or the collection has no
    /// face at that index.
    Unreadable,
}

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FontError::Unreadable => f.write_str("not a font Idem can read"),
        }
    }
}

impl std::error::Error for FontError {}

impl Font {
    /// Load a font file (TrueType or OpenType).
    pub fn from_bytes(data: Vec<u8>) -> Result<Font, FontError> {
        Font::from_collection(data, 0)
    }

    /// Load face `index` of a font collection (`.ttc`).
    pub fn from_collection(data: Vec<u8>, index: u32) -> Result<Font, FontError> {
        let face = rustybuzz::Face::from_slice(&data, index).ok_or(FontError::Unreadable)?;
        let units_per_em = face.units_per_em();
        let ascender = i32::from(face.ascender());
        let descender = i32::from(face.descender());
        Ok(Font { data, index, units_per_em, ascender, descender, private_use: false })
    }

    /// Let this font set private-use characters (U+E000 to U+F8FF, and
    /// planes 15 and 16). Unicode gives those no meaning: each font that has
    /// them decides its own, so U+E000 is a Tengwar letter in one font and a
    /// logo in another. Without this, Idem never takes one from this font,
    /// wherever it is in the list, and a character no font may set is a
    /// missing glyph. In tolege, the fonts an account installed get this;
    /// the fonts tolege bundles don't.
    pub fn with_private_use(mut self) -> Font {
        self.private_use = true;
        self
    }

    fn face(&self) -> rustybuzz::Face<'_> {
        rustybuzz::Face::from_slice(&self.data, self.index).expect("checked when the font was loaded")
    }

    /// Whether the font has a glyph for `c`. It reads the font's tables on
    /// every call, so it's for asking now and then, not character by
    /// character through a long text.
    pub fn has_char(&self, c: char) -> bool {
        self.face().glyph_index(c).is_some()
    }

    /// `units` font units at `size`, in scaled points, rounded half away
    /// from zero.
    fn scale(&self, units: i32, size: i64) -> i64 {
        scale(units, size, self.units_per_em)
    }
}

fn scale(units: i32, size: i64, units_per_em: i32) -> i64 {
    let n = i128::from(units) * i128::from(size);
    let d = i128::from(units_per_em);
    let rounded = if n >= 0 { (2 * n + d) / (2 * d) } else { -((-2 * n + d) / (2 * d)) };
    rounded as i64
}

/// How a paragraph's lines are filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// Each line as full as it goes at its natural width; the right edge is
    /// ragged.
    Ragged,
    /// The breaks with the fewest demerits over the whole paragraph
    /// (Knuth–Plass, with demerits' default [`Params`](demerits::Params)),
    /// then each line's spaces stretched or shrunk to make it exactly the
    /// line's width. The last line and lines before a `\n` are set at their
    /// natural width.
    ///
    /// When no set of breaks keeps every line within the tolerance (a word
    /// longer than a narrow line, say), the paragraph is broken first fit
    /// instead: each line as full as its spaces can shrink to hold, and a line
    /// that still doesn't fit sticks out.
    Justified,
}

/// A font at a size, and how tall a line it asks for.
#[derive(Debug, Clone, Copy)]
pub struct TextStyle<'a> {
    pub font: &'a Font,
    /// Fonts for what `font` doesn't have, in the order to try them. Each
    /// grapheme cluster (a letter and its accents, a syllable, an emoji
    /// sequence: what the cursor steps over) is set in the first font that
    /// has every character in it, `font` first. A cluster is never split
    /// between fonts. Default-ignorable characters (joiners, variation
    /// selectors) don't count, since fonts often leave them out.
    ///
    /// Each fallback font is set at the same size and line height as `font`.
    pub fallback: &'a [&'a Font],
    /// The text's language, as a BCP 47 tag: `"ja"`, `"zh-Hans"`,
    /// `"zh-Hant"`, `"zh-HK"`, `"ko"`, `"sr"`. Fonts draw some characters
    /// differently by language (OpenType's `locl`), and the shaper picks
    /// those forms from it: the same Han character is written one way in
    /// Japanese, another in Simplified or Traditional Chinese, another in
    /// Korean, as are some Cyrillic letters in Serbian and Bulgarian. `None`
    /// takes each font's default forms, whatever language it was made for.
    pub language: Option<&'a str>,
    /// The font size, in scaled points.
    pub size: i64,
    /// How tall a line this text needs, in scaled points: CSS's
    /// `line-height`. The font's ascent plus descent sits in the middle of
    /// it, so a line height smaller than the font's lets lines overlap.
    pub line_height: i64,
}

impl<'a> TextStyle<'a> {
    /// Font `i` of this style: 0 is `font`, 1 the first fallback, and so on.
    fn font_at(&self, i: usize) -> &'a Font {
        if i == 0 { self.font } else { self.fallback[i - 1] }
    }

    /// `font`, then the fallback fonts.
    fn fonts(&self) -> impl Iterator<Item = &'a Font> + use<'a> {
        std::iter::once(self.font).chain(self.fallback.iter().copied())
    }

    /// How far the line reaches above the baseline, and below it, for this
    /// text alone: the font's ascent and descent with half the leading
    /// (`line_height` less ascent and descent) added to each, the odd
    /// scaled point below.
    fn extent(&self) -> (i64, i64) {
        self.extent_in(self.font)
    }

    /// The same, for the glyphs set in `font`, one of this style's fonts: in
    /// CSS too, a fallback font's ascent and descent are its own.
    fn extent_in(&self, font: &Font) -> (i64, i64) {
        let ascent = font.scale(font.ascender, self.size);
        let descent = -font.scale(font.descender, self.size);
        let above = (self.line_height - (ascent + descent)).div_euclid(2) + ascent;
        (above, self.line_height - above)
    }
}

/// A stretch of a paragraph's text set in one style. A paragraph's runs
/// follow each other and cover its text.
#[derive(Debug, Clone, Copy)]
pub struct Run<'a> {
    /// How many bytes of the text the run holds. It may be zero.
    pub len: usize,
    pub style: TextStyle<'a>,
}

/// How to set a paragraph.
#[derive(Debug, Clone, Copy)]
pub struct Style<'a> {
    /// The paragraph's own style, which every line is at least as tall as:
    /// CSS's strut. An empty line is exactly its height.
    pub strut: TextStyle<'a>,
    pub align: Align,
}

/// A paragraph set into lines.
#[derive(Debug, Clone, PartialEq)]
pub struct Paragraph {
    pub lines: Vec<Line>,
    /// The lines' heights added up: where the last line ends.
    pub height: i64,
}

/// One line of a [`Paragraph`].
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// The bytes of the paragraph's text this line holds, including the
    /// spaces and line end it broke at, which take no room. The lines' ranges
    /// follow each other and cover the whole text.
    pub text: Range<usize>,
    /// Where the line starts, down from the top of the paragraph: where the
    /// line before it ends.
    pub top: i64,
    /// How tall the line is. Each run with a glyph on the line asks for
    /// room above and below the baseline (see [`TextStyle::line_height`]),
    /// and so does the strut; the line is as tall as the most room asked for
    /// above plus the most below, as in CSS.
    pub height: i64,
    /// The baseline, down from the top of the paragraph.
    pub baseline: i64,
    /// How wide the line is as set: the line's width for a justified line
    /// (unless it's too long even with its spaces shrunk all they can),
    /// its natural width otherwise.
    pub width: i64,
    /// demerits' adjustment ratio for the line: how far its spaces stretch
    /// (above 0) or shrink (below 0). Always `0.0` when ragged.
    pub ratio: f64,
    /// The glyphs to draw, from left to right: where text runs right to
    /// left, that's the reverse of its order in the text.
    pub glyphs: Vec<Glyph>,
}

/// A glyph, placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyph {
    /// The glyph's id in the font. 0 is the font's missing glyph: no font
    /// had this cluster's characters.
    pub id: u16,
    /// From the start of the line, in scaled points.
    pub x: i64,
    /// Down from the line's baseline, in scaled points (marks set above a
    /// letter have a negative `y`).
    pub y: i64,
    /// The byte in the paragraph's text where this glyph's characters begin.
    /// A ligature covers the characters up to the next glyph's cluster.
    pub cluster: usize,
    /// The index of the run it's set in, which says its size.
    pub run: usize,
    /// Which of its run's fonts it's from: 0 is the run's own
    /// [`font`](TextStyle::font), 1 the first of its
    /// [`fallback`](TextStyle::fallback), and so on.
    pub font: usize,
    /// How far the pen moves on past it: its width on the line. A mark set
    /// on a letter has none.
    pub advance: i64,
    /// Its text's level: even runs left to right, odd right to left. Always
    /// 0 from [`set_paragraph`].
    pub level: u8,
}

/// Set `text` as one paragraph, each of its `runs` in its own style. Line
/// `k` is `line_widths[k]` wide (in scaled points); lines past the end of
/// `line_widths` use its last entry.
///
/// Every text gives at least one line: an empty one gives one empty line,
/// the strut's height, which is where a cursor goes.
///
/// Each run is shaped on its own, so nothing kerns or ligates across a
/// change of style, and a run that starts inside a character sequence that
/// shapes as one (a letter and its accent) splits it.
///
/// # Panics
///
/// If `line_widths` is empty, if the runs don't add up to the text, or if
/// one ends inside a character.
pub fn set_paragraph(text: &str, runs: &[Run], style: &Style, line_widths: &[i64]) -> Paragraph {
    set_paragraph_bidi(text, runs, style, line_widths, 0, &[], false, false)
}

/// [`set_paragraph`], for text that doesn't all run left to right.
/// `levels` is each byte's level, as Unicode's bidirectional algorithm
/// has them (see [`bidi::levels`](crate::bidi::levels)): even runs left to
/// right, odd right to left. `base` is the paragraph's own. Empty `levels`
/// is every byte at `base`.
///
/// Each stretch at one level is shaped in its direction, and each line's
/// stretches are put in the order they're read on the page (the
/// algorithm's rule L2). A paragraph that runs right to left and isn't
/// justified is set against the right edge.
///
/// `keep_spaces` gives the spaces before the paragraph's end the room
/// they'd have anywhere else, where a finished page gives them none. An
/// editor wants them: a space just typed has to show, and to sit where
/// its text puts it (after an English word in a Hebrew paragraph, between
/// the word and the Hebrew).
///
/// # Panics
///
/// As [`set_paragraph`], and if `levels` is neither empty nor as long as
/// the text.
pub fn set_paragraph_bidi(
    text: &str,
    runs: &[Run],
    style: &Style,
    line_widths: &[i64],
    base: u8,
    levels: &[u8],
    keep_spaces: bool,
    break_words: bool,
) -> Paragraph {
    assert!(levels.is_empty() || levels.len() == text.len(), "a level for every byte, or none");
    assert!(!line_widths.is_empty(), "no line widths: every line needs one");
    let mut ends = Vec::with_capacity(runs.len());
    let mut end = 0usize;
    for run in runs {
        end = match end.checked_add(run.len) {
            Some(e) if e <= text.len() => e,
            _ => panic!("the runs hold more than the text's {} bytes", text.len()),
        };
        assert!(text.is_char_boundary(end), "a run ends at byte {end}, inside a character");
        ends.push(end);
    }
    assert!(end == text.len(), "the runs hold {end} of the text's {} bytes", text.len());

    // A word wider than the narrowest line can't fit on one whole. Set
    // as a page is, it has a line to itself and runs past the edge; an
    // editor asks for it broken (`break_words`), so nothing typed is ever
    // out of sight.
    let limit = if break_words { line_widths.iter().copied().min().unwrap_or(0) } else { 0 };
    let (items, spans, glyphs, parts) = itemize(text, runs, &ends, base, levels, keep_spaces, limit);
    let breaks = match style.align {
        Align::Ragged => demerits::ragged(&items, line_widths),
        Align::Justified => match demerits::break_lines(&items, line_widths, &demerits::Params::default()) {
            Err(demerits::Error::Infeasible) => demerits::greedy(&items, line_widths),
            found => found,
        },
    }
    .expect("itemize always ends the paragraph with a forced break");
    let lines = set_lines(&items, &spans, &glyphs, &parts, &breaks, runs, style, line_widths, base);
    let height = lines.last().map_or(0, |l| l.top + l.height);
    Paragraph { lines, height }
}

/// A glyph as shaped, before it's placed on a line.
#[derive(Debug, Clone, Copy)]
struct Shaped {
    id: u16,
    cluster: usize,
    run: usize,
    font: usize,
    advance: i64,
    x_offset: i64,
    y_offset: i64,
    level: u8,
}

/// Shapes a paragraph one run at a time: each stretch of text between two
/// breaks, and each run of spaces, on its own. So a word's glyphs never kern
/// against the next line's (a hyphen at a line end against the letter after
/// it), and a word is the same however it's broken: the editor can keep its
/// shaping from one keystroke to the next. The text on either side is still
/// given as context, for the substitutions that look at it (Arabic joining,
/// contextual alternates). Where a stretch crosses from one run to the next,
/// each run's part is shaped on its own, in its own font.
struct Shaper<'a> {
    runs: &'a [Run<'a>],
    /// Where each run ends, in bytes.
    ends: &'a [usize],
    faces: Vec<Faced<'a>>,
    buffer: Option<UnicodeBuffer>,
    glyphs: Vec<Shaped>,
    /// The paragraph's level, and each byte's (none: all the paragraph's).
    base: u8,
    levels: &'a [u8],
    /// Where each stretch shaped as one ends among the glyphs. Inside one,
    /// the glyphs are in the order they're drawn, left to right; a line
    /// puts whole stretches in order.
    parts: Vec<usize>,
}

/// A font in use, parsed once, with its plans. Building a plan is slow, so
/// there's one per font, script and language, reused.
struct Faced<'a> {
    font: &'a Font,
    face: rustybuzz::Face<'a>,
    plans: Vec<(Script, Option<Language>, Direction, ShapePlan)>,
}

impl<'a> Shaper<'a> {
    fn new(runs: &'a [Run<'a>], ends: &'a [usize], base: u8, levels: &'a [u8]) -> Self {
        Shaper { runs, ends, faces: Vec::new(), buffer: None, glyphs: Vec::new(), base, levels, parts: Vec::new() }
    }

    fn level(&self, at: usize) -> u8 {
        self.levels.get(at).copied().unwrap_or(self.base)
    }

    /// Shape `text[range]`, run by run, and return the range of its glyphs.
    fn shape(&mut self, text: &str, range: Range<usize>) -> Range<usize> {
        let first = self.glyphs.len();
        // The first run that ends after the range starts.
        let mut r = self.ends.partition_point(|&end| end <= range.start);
        while range.end > range.start && r < self.runs.len() {
            let run_start = if r == 0 { 0 } else { self.ends[r - 1] };
            if run_start >= range.end {
                break;
            }
            let piece = run_start.max(range.start)..self.ends[r].min(range.end);
            // Each stretch at one level on its own, in its direction.
            let mut start = piece.start;
            while start < piece.end {
                let level = self.level(start);
                let end = (start..piece.end).find(|&at| self.level(at) != level).unwrap_or(piece.end);
                self.shape_piece(text, start..end, r);
                start = end;
            }
            r += 1;
        }
        first..self.glyphs.len()
    }

    /// Where `font` is in `faces`, parsing it the first time it's used.
    fn faced(&mut self, font: &'a Font) -> usize {
        match self.faces.iter().position(|faced| std::ptr::eq(faced.font, font)) {
            Some(f) => f,
            None => {
                self.faces.push(Faced { font, face: font.face(), plans: Vec::new() });
                self.faces.len() - 1
            }
        }
    }

    /// Shape `text[piece]`, all of it in run `r`: each grapheme cluster in
    /// the first of the run's fonts that has it, and each stretch in one font
    /// shaped as a whole, so a font's own ligatures and joining still work
    /// inside it. A cluster no font can draw is one missing glyph.
    fn shape_piece(&mut self, text: &str, piece: Range<usize>, r: usize) {
        let mut start = piece.start;
        let mut current = None;
        for (i, cluster) in text[piece.clone()].grapheme_indices(true) {
            let at = piece.start + i;
            let font = self.choose(cluster, r, current);
            if font != current {
                if let Some(c) = current {
                    self.shape_in(text, start..at, r, c);
                }
                (start, current) = (at, font);
            }
            if font.is_none() {
                self.missing(at, r);
            }
        }
        if let Some(c) = current {
            self.shape_in(text, start..piece.end, r, c);
        }
    }

    /// Which of run `r`'s fonts to set `cluster` in: the first that has all
    /// its characters, ignoring the default-ignorable ones, and only fonts
    /// allowed private use for a cluster with a private-use character in it
    /// ([`Font::with_private_use`]); failing that, the
    /// first that has its first character, so a letter still shows when only
    /// its accent is missing. `None` when no font has even that. A cluster
    /// of nothing but ignorables stays in the font before it (`previous`).
    fn choose(&mut self, cluster: &str, r: usize, previous: Option<usize>) -> Option<usize> {
        let style = self.runs[r].style;
        let Some(first) = cluster.chars().find(|&c| !is_default_ignorable(c)) else {
            return Some(previous.unwrap_or(0));
        };
        let private = cluster.chars().any(is_private_use);
        let mut has_first = None;
        for (i, font) in style.fonts().enumerate() {
            if private && !font.private_use {
                continue;
            }
            let f = self.faced(font);
            let face = &self.faces[f].face;
            if face.glyph_index(first).is_none() {
                continue;
            }
            if cluster.chars().filter(|&c| !is_default_ignorable(c)).all(|c| face.glyph_index(c).is_some()) {
                return Some(i);
            }
            has_first.get_or_insert(i);
        }
        has_first
    }

    /// A cluster at `at` that none of run `r`'s fonts has: one glyph, its
    /// font's missing glyph (`.notdef`, glyph 0, usually a box), however
    /// many characters the cluster holds, so a flag or a family emoji is one
    /// box, not several.
    fn missing(&mut self, at: usize, r: usize) {
        let TextStyle { font, size, .. } = self.runs[r].style;
        let f = self.faced(font);
        let advance = self.faces[f].face.glyph_hor_advance(rustybuzz::ttf_parser::GlyphId(0)).unwrap_or(0);
        self.glyphs.push(Shaped {
            id: 0,
            cluster: at,
            run: r,
            font: 0,
            advance: font.scale(i32::from(advance), size),
            x_offset: 0,
            y_offset: 0,
            level: self.level(at),
        });
        self.parts.push(self.glyphs.len());
    }

    /// Shape `text[piece]` in run `r`'s font `font_index`.
    fn shape_in(&mut self, text: &str, piece: Range<usize>, r: usize, font_index: usize) {
        let TextStyle { size, language, .. } = self.runs[r].style;
        let language = language.and_then(|tag| tag.parse::<Language>().ok());
        let font = self.runs[r].style.font_at(font_index);
        let f = self.faced(font);
        let mut buffer = self.buffer.take().unwrap_or_default();
        buffer.push_str(&text[piece.clone()]);
        buffer.set_pre_context(&text[..piece.start]);
        buffer.set_post_context(&text[piece.end..]);
        // Set right to left, the glyphs come out as they're drawn, from
        // the left: last cluster first.
        let level = self.level(piece.start);
        let direction = if level % 2 == 1 { Direction::RightToLeft } else { Direction::LeftToRight };
        buffer.set_direction(direction);
        if let Some(language) = &language {
            buffer.set_language(language.clone());
        }
        buffer.guess_segment_properties();
        let script = buffer.script();
        let Faced { face, plans, .. } = &mut self.faces[f];
        let plan = match plans.iter().position(|(s, l, d, _)| *s == script && *l == language && *d == direction) {
            Some(i) => &plans[i].3,
            None => {
                let plan = ShapePlan::new(face, direction, Some(script), language.as_ref(), &[]);
                plans.push((script, language, direction, plan));
                &plans.last().expect("just pushed").3
            }
        };
        let shaped = rustybuzz::shape_with_plan(face, plan, buffer);
        self.glyphs.extend(shaped.glyph_infos().iter().zip(shaped.glyph_positions()).map(|(info, pos)| {
            Shaped {
                id: u16::try_from(info.glyph_id).expect("glyph ids fit in 16 bits"),
                cluster: piece.start + info.cluster as usize,
                run: r,
                font: font_index,
                advance: font.scale(pos.x_advance, size),
                x_offset: font.scale(pos.x_offset, size),
                y_offset: font.scale(pos.y_offset, size),
                level,
            }
        }));
        self.parts.push(self.glyphs.len());
        self.buffer = Some(shaped.clear());
    }
}

/// What an item stands for: the glyphs it places, and where in the text the
/// line ends if it breaks there.
#[derive(Debug, Clone)]
struct Span {
    glyphs: Range<usize>,
    end: usize,
}

/// Unicode's Default_Ignorable_Code_Point: joiners, variation selectors,
/// bidi controls, tags and the like, which draw nothing and which fonts
/// often leave out of their character maps.
fn is_default_ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// The Private Use Areas: the BMP's, and planes 15 and 16.
fn is_private_use(c: char) -> bool {
    matches!(c, '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..='\u{FFFFD}' | '\u{100000}'..='\u{10FFFD}')
}

/// Spaces that a line can break after, stretch and lose at a break.
pub(crate) fn is_space(c: char) -> bool {
    break_property(u32::from(c)) == BreakClass::Space
}

/// Line ends: a forced break, taking no room.
fn is_line_end(c: char) -> bool {
    matches!(
        break_property(u32::from(c)),
        BreakClass::Mandatory | BreakClass::CarriageReturn | BreakClass::LineFeed | BreakClass::NextLine
    )
}

/// Turn the text into items, shaping it as it goes.
fn itemize(
    text: &str,
    runs: &[Run],
    ends: &[usize],
    base: u8,
    levels: &[u8],
    keep_spaces: bool,
    limit: i64,
) -> (Vec<Item>, Vec<Span>, Vec<Shaped>, Vec<usize>) {
    let mut shaper = Shaper::new(runs, ends, base, levels);
    let mut items = Vec::new();
    let mut spans = Vec::new();
    let mut push = |item: Item, glyphs: Range<usize>, end: usize| {
        items.push(item);
        spans.push(Span { glyphs, end });
    };
    let end_of_paragraph = |push: &mut dyn FnMut(Item, Range<usize>, usize), at: usize, end: usize| {
        // The line is set at its natural width: TeX's \par.
        push(Item::Penalty { width: 0, cost: INFINITY, flagged: false }, at..at, end);
        push(Item::Glue { width: 0, stretch: Stretch::Infinite, shrink: 0 }, at..at, end);
        push(Item::Penalty { width: 0, cost: -INFINITY, flagged: false }, at..at, end);
    };
    if text.is_empty() {
        // An empty paragraph is one empty line.
        end_of_paragraph(&mut push, 0, 0);
        return (items, spans, shaper.glyphs, shaper.parts);
    }
    let mut start = 0;
    for (end, opportunity) in linebreaks(text) {
        let segment = &text[start..end];
        let space_end = start + segment.trim_end_matches(is_line_end).len();
        let content_end = start + text[start..space_end].trim_end_matches(is_space).len();
        let (glyphs_before, parts_before) = (shaper.glyphs.len(), shaper.parts.len());
        let content = shaper.shape(text, start..content_end);
        let width = |glyphs: &[Shaped], range: &Range<usize>| glyphs[range.clone()].iter().map(|g| g.advance).sum();

        if content_end > start {
            let whole: i64 = width(&shaper.glyphs, &content);
            if limit > 0 && whole > limit {
                // Wider than a line: it may break between any two of its
                // characters (CSS's `overflow-wrap: anywhere`), at a cost,
                // so it's only done to what can't fit. Each is shaped on
                // its own, as anything a line can break between is.
                shaper.glyphs.truncate(glyphs_before);
                shaper.parts.truncate(parts_before);
                let mut at = start;
                for cluster in text[start..content_end].graphemes(true) {
                    if at > start {
                        let g = shaper.glyphs.len();
                        push(Item::Penalty { width: 0, cost: 1000, flagged: false }, g..g, at);
                    }
                    let end = at + cluster.len();
                    let shaped = shaper.shape(text, at..end);
                    push(Item::Box { width: width(&shaper.glyphs, &shaped) }, shaped, end);
                    at = end;
                }
            } else {
                push(Item::Box { width: whole }, content, content_end);
            }
        }
        match opportunity {
            BreakOpportunity::Mandatory => {
                // Spaces before a line end take no room, and the line end
                // itself is never drawn. Kept, the spaces are part of the
                // line like any text: they can't stretch, or be broken
                // from what they follow.
                if keep_spaces && space_end > content_end {
                    // At the paragraph's own level they're at the line's
                    // end, and hang: they're drawn (out past the edge if
                    // need be) but count nothing toward where lines
                    // break, so typing spaces never sends the word before
                    // them to the next line. Deeper (in an isolate) they
                    // sit inside the line, and take their room.
                    let hang = (content_end..space_end).all(|at| shaper.level(at) == base);
                    let spaces = shaper.shape(text, content_end..space_end);
                    let room = if hang { 0 } else { width(&shaper.glyphs, &spaces) };
                    push(Item::Box { width: room }, spaces, space_end);
                }
                end_of_paragraph(&mut push, shaper.glyphs.len(), end);
            }
            BreakOpportunity::Allowed if space_end > content_end => {
                // TeX's interword glue: stretch a half, shrink a third.
                let spaces = shaper.shape(text, content_end..space_end);
                let w = width(&shaper.glyphs, &spaces);
                push(Item::Glue { width: w, stretch: Stretch::Finite(w / 2), shrink: w / 3 }, spaces, end);
            }
            BreakOpportunity::Allowed => {
                // A break with no space: after a hyphen, a slash, between
                // ideographs. After a hyphen it's flagged, as Knuth and Plass
                // flag an explicit hyphen.
                let hyphen = text[start..content_end].ends_with(['-', '\u{2010}']);
                let cost = if hyphen { 50 } else { 0 };
                let at = shaper.glyphs.len();
                push(Item::Penalty { width: 0, cost, flagged: hyphen }, at..at, end);
            }
        }
        start = end;
    }
    (items, spans, shaper.glyphs, shaper.parts)
}

/// A stretch of a line's glyphs that stays together, in the order they're
/// drawn: what was shaped as one, or part of it.
struct Part {
    glyphs: Range<usize>,
    level: u8,
    /// Room after it that's no glyph's: what a space was stretched by, or
    /// shrunk (then it's less than nothing).
    pad: i64,
}

/// The order to draw `parts` in, from the left: Unicode's rule L2. From
/// the highest level down to the lowest odd one, every run of parts at
/// that level or higher is turned round.
fn drawn_order(parts: &[Part], base: u8) -> Vec<usize> {
    let mut order: Vec<usize> = (0..parts.len()).collect();
    let highest = parts.iter().map(|p| p.level).max().unwrap_or(base).max(base);
    let lowest_odd = parts.iter().map(|p| p.level).chain([base]).filter(|l| l % 2 == 1).min();
    let Some(lowest_odd) = lowest_odd else {
        return order;
    };
    for level in (lowest_odd..=highest).rev() {
        let mut i = 0;
        while i < order.len() {
            if parts[order[i]].level < level {
                i += 1;
                continue;
            }
            let start = i;
            while i < order.len() && parts[order[i]].level >= level {
                i += 1;
            }
            order[start..i].reverse();
        }
    }
    order
}

#[allow(clippy::too_many_arguments)]
fn set_lines(
    items: &[Item],
    spans: &[Span],
    glyphs: &[Shaped],
    shaped_parts: &[usize],
    breaks: &[demerits::Break],
    runs: &[Run],
    style: &Style,
    line_widths: &[i64],
    base: u8,
) -> Vec<Line> {
    let strut = style.strut.extent();
    // The room each run asks for, in each of its fonts.
    let extents: Vec<Vec<(i64, i64)>> =
        runs.iter().map(|run| run.style.fonts().map(|font| run.style.extent_in(font)).collect()).collect();

    let mut lines = Vec::with_capacity(breaks.len());
    let mut s = 0;
    let mut text_start = 0;
    let mut top = 0;
    for (k, b) in breaks.iter().enumerate() {
        let width = line_widths[k.min(line_widths.len() - 1)];
        let at = b.position;

        // Measure the line, as demerits does.
        let (mut natural, mut stretch, mut infinite, mut shrink) = (0, 0, false, 0);
        for item in &items[s..at] {
            match *item {
                Item::Box { width } => natural += width,
                Item::Glue { width, stretch: y, shrink: z } => {
                    natural += width;
                    match y {
                        Stretch::Finite(y) => stretch += y,
                        Stretch::Infinite => infinite = true,
                    }
                    shrink += z;
                }
                Item::Penalty { .. } => {}
            }
        }
        if let Item::Penalty { width, .. } = items[at] {
            natural += width;
        }

        // How much the spaces give or take in all, and out of how much.
        // Infinite glue (the end of a paragraph, or a line end) takes up any
        // slack itself, but doesn't help a line that's too long.
        let (adjust, total) = if style.align == Align::Justified {
            if natural < width && stretch > 0 && !infinite {
                (width - natural, stretch)
            } else if natural > width && shrink > 0 {
                (-(natural - width).min(shrink), shrink)
            } else {
                (0, 0)
            }
        } else {
            (0, 0)
        };

        // The line's parts, in the text's order: each stretch shaped as
        // one, with what its space gives or takes after it.
        let mut parts: Vec<Part> = Vec::new();
        let split = |parts: &mut Vec<Part>, range: &Range<usize>| {
            let mut from = range.start;
            // The first shaped stretch that ends after `from`.
            let mut k = shaped_parts.partition_point(|&end| end <= from);
            while from < range.end {
                let to = shaped_parts.get(k).copied().unwrap_or(range.end).min(range.end);
                parts.push(Part { glyphs: from..to, level: glyphs[from].level, pad: 0 });
                from = to;
                k += 1;
            }
        };
        // The running share of each glue in `adjust`: glue j ends up
        // `floor(adjust * (its share and every share before it) / total)`
        // from where it would be, so the shares add up to `adjust` exactly.
        let (mut shares, mut given) = (0i64, 0i64);
        for (item, span) in items[s..at].iter().zip(&spans[s..at]) {
            match *item {
                Item::Box { .. } => split(&mut parts, &span.glyphs),
                Item::Glue { width: natural_width, stretch: y, shrink: z } => {
                    let before = parts.len();
                    split(&mut parts, &span.glyphs);
                    let share = match (adjust > 0, y) {
                        (true, Stretch::Finite(y)) => y,
                        (true, Stretch::Infinite) => 0,
                        (false, _) => z,
                    };
                    let mut moved = 0;
                    if total > 0 {
                        shares += share;
                        let target = (i128::from(adjust) * i128::from(shares)).div_euclid(i128::from(total)) as i64;
                        moved = target - given;
                        given = target;
                    }
                    // The glue is as wide as its natural width and its
                    // share, whatever its glyphs came to.
                    let drawn: i64 = glyphs[span.glyphs.clone()].iter().map(|g| g.advance).sum();
                    let pad = natural_width + moved - drawn;
                    if parts.len() > before {
                        parts.last_mut().expect("just pushed").pad = pad;
                    } else if let Some(last) = parts.last_mut() {
                        last.pad += pad;
                    }
                }
                Item::Penalty { .. } => {}
            }
        }

        let mut placed = Vec::new();
        let mut pen = 0;
        let (mut above, mut below) = strut;
        for i in drawn_order(&parts, base) {
            let part = &parts[i];
            for g in &glyphs[part.glyphs.clone()] {
                placed.push(Glyph {
                    id: g.id,
                    x: pen + g.x_offset,
                    y: -g.y_offset,
                    cluster: g.cluster,
                    run: g.run,
                    font: g.font,
                    advance: g.advance,
                    level: g.level,
                });
                pen += g.advance;
                let (a, b) = extents[g.run][g.font];
                above = above.max(a);
                below = below.max(b);
            }
            pen += part.pad;
        }
        if let Item::Penalty { width, .. } = items[at] {
            pen += width;
        }
        // A paragraph that runs right to left starts at the right edge:
        // what doesn't fit (spaces hanging at its end, a word too long
        // to break) runs out past the left one.
        if base % 2 == 1 && style.align == Align::Ragged {
            for g in &mut placed {
                g.x += width - pen;
            }
        }

        let end = spans[at].end;
        lines.push(Line {
            text: text_start..end,
            top,
            height: above + below,
            baseline: top + above,
            width: pen,
            ratio: if style.align == Align::Ragged { 0.0 } else { b.ratio },
            glyphs: placed,
        });
        text_start = end;
        top += above + below;

        // The next line starts at the next box or forced break.
        s = at + 1;
        while s < items.len() && !starts_line(&items[s]) {
            s += 1;
        }
    }
    lines
}

fn starts_line(item: &Item) -> bool {
    matches!(*item, Item::Box { .. }) || matches!(*item, Item::Penalty { cost, .. } if cost <= -INFINITY)
}

#[cfg(test)]
mod tests {
    use super::{is_private_use, scale};

    #[test]
    fn private_use_areas() {
        for c in ['\u{E000}', '\u{F8FF}', '\u{F0000}', '\u{FFFFD}', '\u{100000}', '\u{10FFFD}'] {
            assert!(is_private_use(c), "U+{:04X}", c as u32);
        }
        for c in ['\u{D7FF}', '\u{F900}', '\u{EFFFF}', '\u{FFFFE}', '\u{FFFFF}', '\u{10FFFE}', 'a', '日'] {
            assert!(!is_private_use(c), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn scaling_rounds_half_away_from_zero() {
        // 1 unit at 1 sp in a 2-unit em is half a scaled point.
        assert_eq!(scale(1, 1, 2), 1);
        assert_eq!(scale(-1, 1, 2), -1);
        assert_eq!(scale(1, 1, 3), 0);
        assert_eq!(scale(2, 1, 3), 1);
        assert_eq!(scale(-2, 1, 3), -1);
        // 1000 units of a 2048-unit em at 12 pt.
        assert_eq!(scale(1000, 12 * super::PT, 2048), 384_000);
        assert_eq!(scale(0, 12 * super::PT, 2048), 0);
    }
}
