//! Paragraphs in more than one style: regular, bold, mono and big text on one
//! line. Which glyphs each run gets, where they go, how tall each line is,
//! and that breaking and justifying see every run at its own width.

use std::ops::Range;
use std::sync::LazyLock;

use calamus::layout::{Align, Font, Line, PT, Paragraph, Run, Style, TextStyle, set_paragraph};
use unicode_linebreak::linebreaks;

fn font(bytes: &[u8]) -> Font {
    Font::from_bytes(bytes.to_vec()).unwrap()
}

static INTER: LazyLock<Font> = LazyLock::new(|| font(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/Inter-Regular.ttf"))));
static INTER_BOLD: LazyLock<Font> = LazyLock::new(|| font(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/Inter-Bold.ttf"))));
static MONO: LazyLock<Font> = LazyLock::new(|| font(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/JetBrainsMono-Regular.ttf"))));

/// Body text: Inter, 12 pt on 16 pt.
fn regular() -> TextStyle<'static> {
    TextStyle { font: &INTER, fallback: &[], language: None, size: 12 * PT, line_height: 16 * PT }
}

fn bold() -> TextStyle<'static> {
    TextStyle { font: &INTER_BOLD, ..regular() }
}

/// Inline code: 0.9 em of a font with other metrics (1000 units to the em,
/// a taller ascent), on the same line height.
fn mono() -> TextStyle<'static> {
    TextStyle { font: &MONO, fallback: &[], language: None, size: 12 * PT * 9 / 10, line_height: 16 * PT }
}

/// Text that asks for a taller line: 24 pt on 30 pt, with an odd scaled
/// point of leading.
fn big() -> TextStyle<'static> {
    TextStyle { font: &INTER, fallback: &[], language: None, size: 24 * PT, line_height: 30 * PT + 1 }
}

/// Text that asks for a short line: smaller than its font.
fn tight() -> TextStyle<'static> {
    TextStyle { font: &INTER, fallback: &[], language: None, size: 12 * PT, line_height: 4 * PT }
}

const WIDE: i64 = 1 << 50;

fn style(align: Align) -> Style<'static> {
    Style { strut: regular(), align }
}

/// Runs from pieces of text and their styles; returns the text too.
fn runs<'a>(pieces: &[(&str, TextStyle<'a>)]) -> (String, Vec<Run<'a>>) {
    let text = pieces.iter().map(|(t, _)| *t).collect();
    (text, pieces.iter().map(|&(t, style)| Run { len: t.len(), style }).collect())
}

fn set(pieces: &[(&str, TextStyle<'static>)], align: Align, width: i64) -> (String, Paragraph) {
    let (text, runs) = runs(pieces);
    let p = set_paragraph(&text, &runs, &style(align), &[width]);
    (text, p)
}

/// `text` set on one line in one style, strut and all.
fn alone(text: &str, s: TextStyle<'static>) -> Line {
    let p = set_paragraph(text, &[Run { len: text.len(), style: s }], &Style { strut: s, align: Align::Ragged }, &[WIDE]);
    assert_eq!(p.lines.len(), 1, "{text:?} should fit on one line");
    p.lines.into_iter().next().unwrap()
}

fn natural(text: &str, s: TextStyle<'static>) -> i64 {
    alone(text, s).width
}

/// How far a line of this style alone reaches above its baseline, and below.
fn extent(s: TextStyle<'static>) -> (i64, i64) {
    let line = alone("x", s);
    assert_eq!((line.top, line.height), (0, s.line_height));
    (line.baseline, line.height - line.baseline)
}

/// Where each run starts and ends.
fn ranges(runs: &[Run]) -> Vec<Range<usize>> {
    let mut start = 0;
    runs.iter()
        .map(|r| {
            start += r.len;
            start - r.len..start
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Which run each glyph is in

#[test]
fn every_glyph_says_its_run() {
    let pieces = [("Take ", regular()), ("up", bold()), (" and ", regular()), ("read", mono())];
    let (text, p) = set(&pieces, Align::Ragged, WIDE);
    let (_, runs) = runs(&pieces);
    let ranges = ranges(&runs);
    let glyphs = &p.lines[0].glyphs;
    assert_eq!(glyphs.len(), text.len());
    for g in glyphs {
        assert!(ranges[g.run].contains(&g.cluster), "{g:?} isn't in run {}", g.run);
    }
    assert_eq!(glyphs.iter().map(|g| g.run).collect::<Vec<_>>(), [0, 0, 0, 0, 0, 1, 1, 2, 2, 2, 2, 2, 3, 3, 3, 3]);
}

#[test]
fn empty_runs_are_counted_but_hold_nothing() {
    let pieces = [("", big()), ("Tolle", regular()), ("", big()), ("", mono()), (" lege", bold()), ("", big())];
    let (_, p) = set(&pieces, Align::Ragged, WIDE);
    let line = &p.lines[0];
    assert!(line.glyphs[..5].iter().all(|g| g.run == 1));
    assert!(line.glyphs[5..].iter().all(|g| g.run == 4));
    assert_eq!(line.height, 16 * PT, "an empty run asks for no room: it has nothing on the line");
}

#[test]
fn each_run_is_set_in_its_own_font() {
    let (_, p) = set(&[("Tol", regular()), ("le", bold())], Align::Ragged, WIDE);
    let line = &p.lines[0];
    let bold_alone = alone("le", bold());
    let split = natural("Tol", regular());
    assert_eq!(line.glyphs[3].x, split, "the bold starts where the regular ends");
    for (g, b) in line.glyphs[3..].iter().zip(&bold_alone.glyphs) {
        assert_eq!((g.id, g.x - split, g.y), (b.id, b.x, b.y));
    }
    assert_eq!(line.width, split + bold_alone.width);
    assert!(natural("le", bold()) > natural("le", regular()), "bold is wider, or this test proves nothing");
}

#[test]
fn a_bigger_run_is_bigger() {
    let (_, p) = set(&[("a ", regular()), ("big", big()), (" word", regular())], Align::Ragged, WIDE);
    let line = &p.lines[0];
    let big_glyphs: Vec<_> = line.glyphs.iter().filter(|g| g.run == 1).collect();
    let advance = line.glyphs[5].x - line.glyphs[2].x;
    assert_eq!(big_glyphs.len(), 3);
    assert_eq!(advance, natural("big", big()), "the big run takes its own room");
    assert!(advance > 2 * natural("big", regular()) - 10);
}

#[test]
fn spaces_are_as_wide_as_their_own_run() {
    let mono_space = natural("a b", mono()) - natural("ab", mono());
    let regular_space = natural("a b", regular()) - natural("ab", regular());
    assert_ne!(mono_space, regular_space, "or this test proves nothing");
    let (_, p) = set(&[("one", regular()), (" ", mono()), ("two", regular())], Align::Ragged, WIDE);
    assert_eq!(p.lines[0].width, natural("one", regular()) + mono_space + natural("two", regular()));
}

#[test]
fn runs_split_inside_words_lose_their_kerning() {
    // A change of style is a new piece of shaping: A and V can't kern.
    let one = alone("AV", regular());
    let (_, split) = set(&[("A", regular()), ("V", regular())], Align::Ragged, WIDE);
    assert!(one.width < split.lines[0].width);
    assert_eq!(split.lines[0].width, natural("A", regular()) + natural("V", regular()));
}

#[test]
fn runs_split_at_spaces_change_nothing_but_run_numbers() {
    let text = "I heard from a neighbouring house a voice, as of boy or girl, I know not, chanting.";
    let mut pieces = Vec::new();
    let mut start = 0;
    for (end, _) in linebreaks(text) {
        let word_end = start + text[start..end].trim_end().len();
        pieces.push((&text[start..word_end], regular()));
        pieces.push((&text[word_end..end], regular()));
        start = end;
    }
    for align in [Align::Ragged, Align::Justified] {
        let whole = set_paragraph(text, &[Run { len: text.len(), style: regular() }], &style(align), &[90 * PT]);
        let (_, split) = set(&pieces, align, 90 * PT);
        assert_eq!(whole.lines.len(), split.lines.len());
        for (w, s) in whole.lines.iter().zip(&split.lines) {
            assert_eq!((&w.text, w.top, w.height, w.baseline, w.width), (&s.text, s.top, s.height, s.baseline, s.width));
            assert_eq!(w.ratio.to_bits(), s.ratio.to_bits());
            for (a, b) in w.glyphs.iter().zip(&s.glyphs) {
                assert_eq!((a.id, a.x, a.y, a.cluster), (b.id, b.x, b.y, b.cluster));
            }
            assert_eq!(w.glyphs.len(), s.glyphs.len());
        }
    }
}

// ---------------------------------------------------------------------------
// How tall a line is

#[test]
fn the_styles_here_differ_where_it_matters() {
    // The tests below would pass for the wrong reasons if these didn't hold.
    let (r, m, b) = (extent(regular()), extent(mono()), extent(big()));
    assert_ne!(r, m, "mono sits differently in the same line height");
    assert!(b.0 > r.0 && b.1 > r.1, "big asks for more room above and below");
    assert!(m.0 > r.0 || m.1 > r.1, "mono asks for more room somewhere: {m:?} vs {r:?}");
}

#[test]
fn a_line_is_as_tall_as_its_tallest_text_asks() {
    let (r, m, b) = (extent(regular()), extent(mono()), extent(big()));
    type Pieces<'a> = &'a [(&'a str, TextStyle<'static>)];
    let cases: [(Pieces, &[(i64, i64)]); 4] = [
        (&[("plain", regular())], &[r]),
        (&[("with ", regular()), ("code", mono())], &[r, m]),
        (&[("with ", regular()), ("big", big())], &[r, b]),
        (&[("all ", regular()), ("big", big()), (" and ", regular()), ("code", mono())], &[r, b, m]),
    ];
    for (pieces, asks) in cases {
        let (text, p) = set(pieces, Align::Ragged, WIDE);
        let above = asks.iter().map(|e| e.0).max().unwrap();
        let below = asks.iter().map(|e| e.1).max().unwrap();
        let line = &p.lines[0];
        assert_eq!((line.top, line.baseline, line.height), (0, above, above + below), "{text:?}");
        assert_eq!(p.height, above + below);
    }
}

#[test]
fn the_strut_is_the_least_a_line_can_be() {
    // Text that asks for a short line still gets the paragraph's.
    let (_, p) = set(&[("tight", tight())], Align::Ragged, WIDE);
    let r = extent(regular());
    assert_eq!((p.lines[0].baseline, p.lines[0].height), (r.0, r.0 + r.1));
    // And a paragraph whose strut is tight, of regular text, gets the text's.
    let s = Style { strut: tight(), align: Align::Ragged };
    let p = set_paragraph("x", &[Run { len: 1, style: regular() }], &s, &[WIDE]);
    assert_eq!((p.lines[0].baseline, p.lines[0].height), (r.0, r.0 + r.1));
    // An empty paragraph is the strut alone, whatever its runs.
    for runs in [vec![], vec![Run { len: 0, style: big() }]] {
        let p = set_paragraph("", &runs, &s, &[WIDE]);
        let t = extent(tight());
        assert_eq!((p.lines[0].baseline, p.lines[0].height, p.height), (t.0, t.0 + t.1, t.0 + t.1));
    }
}

#[test]
fn a_tall_line_pushes_down_only_the_lines_after_it() {
    let pieces = [("one ", regular()), ("two", big()), (" three four five six seven", regular())];
    // Room for "one two" and no more.
    let width = set(&pieces[..2], Align::Ragged, WIDE).1.lines[0].width + PT;
    let (text, p) = set(&pieces, Align::Ragged, width);
    assert!(p.lines.len() >= 3, "{:?}", texts(&text, &p));
    let (r, b) = (extent(regular()), extent(big()));
    let heights: Vec<i64> = p.lines.iter().map(|l| l.height).collect();
    assert_eq!(heights[0], b.0 + b.1);
    assert!(heights[1..].iter().all(|&h| h == r.0 + r.1), "{heights:?}");
    let mut top = 0;
    for line in &p.lines {
        assert_eq!(line.top, top);
        top += line.height;
    }
    assert_eq!(p.height, top);
    assert_eq!(p.lines[1].baseline, b.0 + b.1 + r.0);
}

#[test]
fn a_space_left_at_a_break_asks_for_no_room() {
    // The big space is where the line breaks: it's not drawn, so it doesn't
    // make the line tall.
    let pieces = [("first", regular()), (" ", big()), ("second", regular())];
    let (text, p) = set(&pieces, Align::Ragged, natural("first", regular()) + PT);
    assert_eq!(texts(&text, &p), ["first ", "second"]);
    assert_eq!(p.lines[0].height, 16 * PT);
    // Drawn inside a line, it does.
    let (_, p) = set(&pieces, Align::Ragged, WIDE);
    let b = extent(big());
    assert_eq!(p.lines[0].height, b.0 + extent(regular()).1.max(b.1));
}

#[test]
fn a_blank_line_is_the_strut() {
    let pieces = [("big\n", big()), ("\n", big()), ("after", regular())];
    let (text, p) = set(&pieces, Align::Ragged, WIDE);
    assert_eq!(texts(&text, &p), ["big\n", "\n", "after"]);
    let (r, b) = (extent(regular()), extent(big()));
    assert_eq!(p.lines.iter().map(|l| l.height).collect::<Vec<_>>(), [b.0 + b.1, r.0 + r.1, r.0 + r.1]);
}

// ---------------------------------------------------------------------------
// Breaking and justifying see each run at its width

fn texts<'a>(text: &'a str, p: &Paragraph) -> Vec<&'a str> {
    p.lines.iter().map(|l| &text[l.text.clone()]).collect()
}

#[test]
fn bold_takes_more_room_and_breaks_sooner() {
    let width = natural("Take up and", regular());
    let (text, p) = set(&[("Take up and", regular()), (" read", regular())], Align::Ragged, width);
    assert_eq!(texts(&text, &p)[0], "Take up and ");
    let (text, p) = set(&[("Take ", regular()), ("up", bold()), (" and read", regular())], Align::Ragged, width);
    assert_eq!(texts(&text, &p)[0], "Take up ");
}

#[test]
fn justified_mixed_lines_fill_the_width_exactly() {
    let pieces = [
        ("I heard from a ", regular()),
        ("neighbouring house", bold()),
        (" a voice, as of boy or girl, I know not, ", regular()),
        ("chanting", mono()),
        (", and oft repeating, ", regular()),
        ("Take up and read", big()),
        ("; Take up and read.", regular()),
    ];
    for width in [120 * PT, 200 * PT + 4321] {
        let (text, p) = set(&pieces, Align::Justified, width);
        for line in &p.lines[..p.lines.len() - 1] {
            if text[line.text.clone()].trim_end().contains(' ') && line.ratio >= -1.0 {
                assert_eq!(line.width, width, "{:?}", &text[line.text.clone()]);
            }
        }
    }
}

#[test]
fn justified_slack_goes_to_each_space_by_its_own_stretch() {
    // A mono space and a regular one: each stretches by half its own width,
    // so the slack splits between them in proportion to their widths.
    let pieces = [("one", regular()), (" ", mono()), ("two", regular()), (" ", regular()), ("three", regular())];
    let (_, natural_p) = set(&pieces, Align::Ragged, WIDE);
    let slack = 6 * PT;
    let (_, p) = set(&[pieces.as_slice(), &[(" four", regular())]].concat(), Align::Justified, natural_p.lines[0].width + slack);
    let line = &p.lines[0];
    let ragged = &natural_p.lines[0];
    let mono_space = natural("a b", mono()) - natural("ab", mono());
    let regular_space = natural("a b", regular()) - natural("ab", regular());
    let first = mono_space / 2;
    let second = regular_space / 2;
    // A space's own glyph stays put; what comes after it moves.
    assert_eq!(line.glyphs[3].x, ragged.glyphs[3].x);
    let moved_first = line.glyphs[4].x - ragged.glyphs[4].x;
    let moved_second = line.glyphs[8].x - ragged.glyphs[8].x;
    let expected_first = (slack as i128 * first as i128 / (first + second) as i128) as i64;
    assert_eq!(moved_first, expected_first);
    assert_eq!(moved_second, slack);
}

// ---------------------------------------------------------------------------
// Runs that don't fit the text

fn panics(text: &str, lens: &[usize]) -> String {
    let runs: Vec<Run> = lens.iter().map(|&len| Run { len, style: regular() }).collect();
    let text = text.to_string();
    let err = std::panic::catch_unwind(move || set_paragraph(&text, &runs, &style(Align::Ragged), &[WIDE]))
        .expect_err("should panic");
    err.downcast_ref::<String>().cloned().or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string())).unwrap()
}

#[test]
fn runs_must_cover_the_text_exactly() {
    assert!(panics("Tolle", &[2, 2]).contains("hold 4 of the text's 5 bytes"));
    assert!(panics("Tolle", &[]).contains("hold 0 of the text's 5 bytes"));
    assert!(panics("Tolle", &[3, 3]).contains("more than the text's 5 bytes"));
    assert!(panics("Tolle", &[usize::MAX, 2]).contains("more than the text's 5 bytes"));
    assert!(panics("Tolle", &[2, usize::MAX]).contains("more than the text's 5 bytes"));
    assert!(panics("", &[1]).contains("more than the text's 0 bytes"));
}

#[test]
fn runs_must_end_between_characters() {
    // é is two bytes.
    assert!(panics("café", &[4, 1]).contains("byte 4, inside a character"));
    assert!(panics("😀", &[1, 3]).contains("byte 1, inside a character"));
}

// ---------------------------------------------------------------------------
// Random paragraphs

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn int(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() % (hi - lo + 1) as u64) as i64
    }
}

fn random_text(rng: &mut Rng) -> String {
    let mut text = String::new();
    for _ in 0..rng.int(0, 30) {
        for _ in 0..rng.int(1, 10) {
            text.push(match rng.int(0, 30) {
                0 => 'é',
                1 => 'A',
                2 => 'V',
                n => (b'a' + (n % 26) as u8) as char,
            });
        }
        text.push_str(match rng.int(0, 19) {
            0 => "\n",
            1 => "  ",
            2 => "-",
            3 => "\n\n",
            _ => " ",
        });
    }
    text
}

fn random_runs(rng: &mut Rng, text: &str) -> Vec<Run<'static>> {
    let styles = [regular(), bold(), mono(), big(), tight()];
    let bounds: Vec<usize> = text.char_indices().map(|(i, _)| i).chain([text.len()]).collect();
    let mut runs = Vec::new();
    let mut start = 0;
    while start < text.len() || rng.int(0, 3) == 0 {
        // Mostly whole words or more; now and then a cut inside a word, or
        // an empty run.
        let end = match rng.int(0, 9) {
            0 => start,
            1..=3 => bounds[bounds.partition_point(|&b| b <= start).min(bounds.len() - 1)],
            _ => bounds[rng.int(0, bounds.len() as i64 - 1) as usize].max(start),
        };
        runs.push(Run { len: end - start, style: styles[rng.int(0, 4) as usize] });
        start = end;
        if runs.len() > 200 {
            break;
        }
    }
    if start < text.len() {
        runs.push(Run { len: text.len() - start, style: regular() });
    }
    runs
}

/// The runs over `range` of the text, and for each, the index it had.
fn restrict<'a>(runs: &[Run<'a>], range: Range<usize>) -> (Vec<Run<'a>>, Vec<usize>) {
    let mut out = Vec::new();
    let mut index = Vec::new();
    for (i, (run, r)) in runs.iter().zip(ranges(runs)).enumerate() {
        let (s, e) = (r.start.max(range.start), r.end.min(range.end));
        if s < e {
            out.push(Run { len: e - s, style: run.style });
            index.push(i);
        }
    }
    (out, index)
}

/// `text[range]` set alone on one line, in its runs.
fn line_alone(text: &str, runs: &[Run<'static>], range: Range<usize>) -> (Line, Vec<usize>) {
    let (sub, index) = restrict(runs, range.clone());
    let p = set_paragraph(&text[range], &sub, &style(Align::Ragged), &[WIDE]);
    assert_eq!(p.lines.len(), 1);
    (p.lines.into_iter().next().unwrap(), index)
}

fn is_line_end(c: char) -> bool {
    c == '\n'
}

fn check(text: &str, runs: &[Run<'static>], p: &Paragraph, align: Align, width: i64) {
    let lines = &p.lines;
    let spans = ranges(runs);
    let strut = extent(regular());
    let extents: Vec<(i64, i64)> = runs.iter().map(|r| extent(r.style)).collect();

    assert_eq!(lines[0].text.start, 0);
    assert_eq!(lines.last().unwrap().text.end, text.len());
    let mut top = 0;
    for (k, line) in lines.iter().enumerate() {
        let line_text = &text[line.text.clone()];
        let trimmed = line_text.trim_end();
        let kept = line.text.start..line.text.start + trimmed.len();
        if k > 0 {
            assert_eq!(lines[k - 1].text.end, line.text.start);
        }

        // Each glyph is on its line, in its run.
        for g in &line.glyphs {
            assert!(kept.contains(&g.cluster), "{g:?} on {line_text:?}");
            assert!(spans[g.run].contains(&g.cluster), "{g:?} outside run {}: {:?}", g.run, spans[g.run]);
        }
        assert!(line.glyphs.windows(2).all(|w| w[0].cluster < w[1].cluster && w[0].run <= w[1].run));

        // Its height: the most room asked for above and below.
        let (mut above, mut below) = strut;
        for g in &line.glyphs {
            above = above.max(extents[g.run].0);
            below = below.max(extents[g.run].1);
        }
        assert_eq!((line.top, line.baseline, line.height), (top, top + above, above + below), "{line_text:?}");
        top += line.height;

        // Its glyphs are what the line's text gives set alone, in its runs:
        // the same ids, and the same places unless justified.
        let (own, index) = line_alone(text, runs, kept.clone());
        assert_eq!(line.glyphs.len(), own.glyphs.len(), "{line_text:?}");
        for (g, o) in line.glyphs.iter().zip(&own.glyphs) {
            assert_eq!((g.id, g.y, g.cluster, g.run), (o.id, o.y, o.cluster + kept.start, index[o.run]), "{line_text:?}");
            if align == Align::Ragged {
                assert_eq!(g.x, o.x, "{line_text:?}");
            }
        }

        let last = k + 1 == lines.len();
        let forced = line_text.ends_with(is_line_end);
        let one_piece = linebreaks(trimmed).all(|(end, _)| end == trimmed.len());
        match align {
            Align::Ragged => {
                assert_eq!(line.width, own.width);
                assert!(line.width <= width || one_piece, "{line_text:?} is {} wide in {width}", line.width);
                if !last && !forced {
                    let rest = &text[line.text.end..];
                    let next = line.text.end + linebreaks(rest).next().map_or(0, |(end, _)| end);
                    let longer = line.text.start..line.text.start + text[line.text.start..next].trim_end().len();
                    assert!(line_alone(text, runs, longer).0.width > width, "{line_text:?} could take more in {width}");
                }
            }
            Align::Justified => {
                let stretchable = trimmed.contains(' ');
                if last || forced {
                    assert!(line.width == own.width || (own.width > width && line.width >= width), "{line_text:?}");
                } else if stretchable && line.ratio >= -1.0 {
                    assert_eq!(line.width, width, "{line_text:?}");
                } else if !stretchable {
                    assert_eq!(line.width, own.width, "{line_text:?}");
                }
            }
        }
    }
    assert_eq!(p.height, top);
}

#[test]
fn random_mixed_paragraphs() {
    let mut rng = Rng(1386);
    for case in 0..400 {
        let text = random_text(&mut rng);
        let runs = random_runs(&mut rng, &text);
        let width = rng.int(30, 300) * PT + rng.int(0, PT - 1);
        for align in [Align::Ragged, Align::Justified] {
            let p = set_paragraph(&text, &runs, &style(align), &[width]);
            let again = set_paragraph(&text, &runs, &style(align), &[width]);
            assert_eq!(p, again, "case {case}");
            check(&text, &runs, &p, align, width);
        }
    }
}

// ---------------------------------------------------------------------------
// The answer doesn't change

fn dump(text: &str, p: &Paragraph) -> String {
    let mut out = String::new();
    for line in &p.lines {
        out += &format!(
            "{:?} top {} height {} baseline {} width {} ratio {:?}\n",
            &text[line.text.clone()],
            line.top,
            line.height,
            line.baseline,
            line.width,
            line.ratio
        );
        let glyphs: Vec<String> = line
            .glyphs
            .iter()
            .map(|g| {
                let y = if g.y == 0 { String::new() } else { format!(",{}", g.y) };
                format!("{}:{}@{}{y}", g.run, g.id, g.x)
            })
            .collect();
        out += &format!("  {}\n", glyphs.join(" "));
    }
    out
}

/// A note's paragraph, with bold, code and a big word, against
/// `tests/snapshots/runs.txt`. `CALAMUS_RECORD_SNAPSHOTS=1 cargo test`
/// records it again.
#[test]
fn mixed_snapshot() {
    let pieces = [
        ("Tolle lege: ", bold()),
        ("I heard from a neighbouring house a voice, chanting ", regular()),
        ("take_up(and: read)", mono()),
        (", and oft repeating, ", regular()),
        ("Take up and read.", big()),
        ("\nInstantly, my countenance altered.", regular()),
    ];
    let mut out = String::new();
    for (name, align) in [("ragged", Align::Ragged), ("justified", Align::Justified)] {
        out += &format!("# {name}, 180 pt wide; glyphs are run:id@x[,y]\n");
        let (text, p) = set(&pieces, align, 180 * PT);
        out += &dump(&text, &p);
    }
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots/runs.txt");
    if std::env::var_os("CALAMUS_RECORD_SNAPSHOTS").is_some() {
        std::fs::write(path, &out).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(path).expect("no snapshot: run with CALAMUS_RECORD_SNAPSHOTS=1");
    assert!(out == expected, "the snapshot changed; got:\n{out}");
}
