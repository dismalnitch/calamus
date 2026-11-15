//! Setting paragraphs in Inter: what a line holds, where it breaks, how it's
//! justified, and that the answer never changes.

use std::sync::LazyLock;
use std::time::Instant;

use calamus::layout::{Align, Font, PT, Paragraph, Run, Style, TextStyle, set_paragraph};
use unicode_linebreak::{BreakOpportunity, linebreaks};

static INTER: LazyLock<Font> =
    LazyLock::new(|| Font::from_bytes(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/Inter-Regular.ttf")).to_vec()).unwrap());

const LINE_HEIGHT: i64 = 16 * PT;

/// Wider than any line here.
const WIDE: i64 = 1 << 50;

/// Inter, 12 pt on 16 pt.
static TEXT: LazyLock<TextStyle<'static>> =
    LazyLock::new(|| TextStyle { font: &INTER, fallback: &[], language: None, size: 12 * PT, line_height: LINE_HEIGHT });

fn style(align: Align) -> Style<'static> {
    Style { strut: *TEXT, align }
}

/// All of `text` in one run of `text_style`.
fn whole<'a>(text: &str, text_style: TextStyle<'a>) -> [Run<'a>; 1] {
    [Run { len: text.len(), style: text_style }]
}

fn set(text: &str, align: Align, width: i64) -> Paragraph {
    set_paragraph(text, &whole(text, *TEXT), &style(align), &[width])
}

/// The width of `text` set on one line.
fn natural(text: &str) -> i64 {
    let p = set(text, Align::Ragged, WIDE);
    assert_eq!(p.lines.len(), 1, "{text:?} should fit on one line");
    p.lines[0].width
}

fn texts<'a>(text: &'a str, p: &Paragraph) -> Vec<&'a str> {
    p.lines.iter().map(|l| &text[l.text.clone()]).collect()
}

fn is_line_end(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}' | '\u{b}' | '\u{c}')
}

// ---------------------------------------------------------------------------
// The shape of an answer

#[test]
fn empty_text_is_one_empty_line() {
    for align in [Align::Ragged, Align::Justified] {
        let p = set("", align, 100 * PT);
        assert_eq!(p.lines.len(), 1);
        assert_eq!(p.lines[0].text, 0..0);
        assert!(p.lines[0].glyphs.is_empty());
        assert_eq!(p.lines[0].width, 0);
        assert_eq!(p.height, LINE_HEIGHT);
    }
}

#[test]
fn one_word_is_one_line() {
    let p = set("Tolle", Align::Ragged, 100 * PT);
    assert_eq!(p.lines.len(), 1);
    let line = &p.lines[0];
    assert_eq!(line.text, 0..5);
    assert_eq!(line.glyphs.iter().map(|g| g.cluster).collect::<Vec<_>>(), [0, 1, 2, 3, 4]);
    assert_eq!(line.glyphs[0].x, 0);
    assert!(line.glyphs.windows(2).all(|w| w[0].x < w[1].x));
    assert!(line.width > 0);
    assert_eq!(line.ratio, 0.0);
}

#[test]
#[should_panic(expected = "no line widths")]
fn no_line_widths_panics() {
    set_paragraph("Tolle", &whole("Tolle", *TEXT), &style(Align::Ragged), &[]);
}

#[test]
fn baselines_step_by_the_line_height() {
    let text = "Take up and read; take up and read; take up and read.";
    let p = set(text, Align::Ragged, 60 * PT);
    assert!(p.lines.len() >= 3);
    assert_eq!(p.height, LINE_HEIGHT * p.lines.len() as i64);
    let first = p.lines[0].baseline;
    assert!(0 < first && first < LINE_HEIGHT, "the first baseline is inside the first line: {first}");
    for (k, line) in p.lines.iter().enumerate() {
        assert_eq!(line.baseline, first + k as i64 * LINE_HEIGHT);
        assert_eq!((line.top, line.height), (k as i64 * LINE_HEIGHT, LINE_HEIGHT));
    }
}

#[test]
fn the_same_text_sets_the_same_way_every_time() {
    let text = "I heard from a neighbouring house a voice, as of boy or girl, I know not, chanting.";
    let other = Font::from_bytes(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/Inter-Regular.ttf")).to_vec()).unwrap();
    for align in [Align::Ragged, Align::Justified] {
        let a = set(text, align, 90 * PT);
        let b = set(text, align, 90 * PT);
        let other = TextStyle { font: &other, ..*TEXT };
        let c = set_paragraph(text, &whole(text, other), &Style { strut: other, align }, &[90 * PT]);
        assert_eq!(a, b);
        assert_eq!(a, c);
    }
}

// ---------------------------------------------------------------------------
// Shaping

#[test]
fn kerning_is_applied() {
    assert!(natural("AV") < natural("A") + natural("V"));
    assert!(natural("To") < natural("T") + natural("o"));
}

#[test]
fn clusters_are_bytes_of_the_text() {
    let text = "naïve café — “quoted” ½";
    let p = set(text, Align::Ragged, 40 * PT);
    for line in &p.lines {
        for g in &line.glyphs {
            assert!(text.is_char_boundary(g.cluster));
            assert!(line.text.contains(&g.cluster));
        }
    }
    let drawn: usize = p.lines.iter().map(|l| l.glyphs.len()).sum();
    assert!(drawn >= text.chars().filter(|c| !c.is_whitespace()).count() - 2, "nearly every character is a glyph");
}

#[test]
fn accents_sit_over_their_letter() {
    // An o with a diaeresis, then a combining acute: Inter has ö, and
    // positions the acute over it with a mark anchor.
    let p = set("o\u{308}\u{301}", Align::Ragged, WIDE);
    let glyphs = &p.lines[0].glyphs;
    assert_eq!(glyphs.len(), 2);
    let (base, mark) = (glyphs[0], glyphs[1]);
    assert_eq!(base.cluster, mark.cluster, "one cluster");
    assert!(mark.y < 0, "the acute is raised: {mark:?}");
    assert!(base.x < mark.x && mark.x < natural("\u{f6}"), "the acute is over the ö: {mark:?}");
    assert_eq!(p.lines[0].width, natural("\u{f6}"), "the acute takes no room");
}

#[test]
fn a_bigger_size_is_wider() {
    let small = natural("Tolle lege");
    let big = TextStyle { size: 24 * PT, ..*TEXT };
    let big = set_paragraph("Tolle lege", &whole("Tolle lege", big), &style(Align::Ragged), &[WIDE]).lines[0].width;
    // Each glyph rounds on its own, so allow a scaled point per glyph.
    assert!((big - 2 * small).abs() <= 10, "{big} vs 2 × {small}");
}

// ---------------------------------------------------------------------------
// Where lines break

#[test]
fn a_line_takes_what_fits_exactly() {
    let text = "Take up and read";
    let p = set(text, Align::Ragged, natural("Take up"));
    assert_eq!(texts(text, &p)[0], "Take up ");
    let p = set(text, Align::Ragged, natural("Take up") - 1);
    assert_eq!(texts(text, &p)[0], "Take ");
}

#[test]
fn a_space_at_a_break_takes_no_room() {
    let text = "Take up and read";
    let p = set(text, Align::Ragged, natural("Take up"));
    assert_eq!(p.lines[0].width, natural("Take up"));
    // The space is still there, for a cursor, but past the line's width.
    assert_eq!(p.lines[0].glyphs.len(), "Take up".len());
}

#[test]
fn a_word_wider_than_the_line_gets_its_own() {
    let text = "Supercalifragilistic is long";
    let p = set(text, Align::Ragged, 20 * PT);
    assert_eq!(texts(text, &p), ["Supercalifragilistic ", "is ", "long"]);
    assert!(p.lines[0].width > 20 * PT);
}

#[test]
fn lines_break_after_hyphens() {
    let text = "well-known";
    let p = set(text, Align::Ragged, natural("well-") + PT);
    assert_eq!(texts(text, &p), ["well-", "known"]);
}

#[test]
fn a_no_break_space_holds_words_together() {
    let text = "Take\u{a0}up and read";
    let p = set(text, Align::Ragged, natural("Take"));
    assert_eq!(texts(text, &p), ["Take\u{a0}up ", "and ", "read"]);
}

#[test]
fn a_line_end_always_breaks() {
    let text = "one\ntwo";
    assert_eq!(texts(text, &set(text, Align::Ragged, WIDE)), ["one\n", "two"]);
    let text = "one\r\ntwo";
    assert_eq!(texts(text, &set(text, Align::Ragged, WIDE)), ["one\r\n", "two"]);
    let text = "one\u{2028}two";
    assert_eq!(texts(text, &set(text, Align::Ragged, WIDE)), ["one\u{2028}", "two"]);
}

#[test]
fn a_blank_line_is_an_empty_line() {
    let text = "one\n\ntwo";
    let p = set(text, Align::Ragged, WIDE);
    assert_eq!(texts(text, &p), ["one\n", "\n", "two"]);
    assert!(p.lines[1].glyphs.is_empty());
    assert_eq!(p.lines[1].width, 0);
}

#[test]
fn spaces_before_a_line_end_take_no_room() {
    let text = "one   \ntwo";
    let p = set(text, Align::Ragged, WIDE);
    assert_eq!(texts(text, &p), ["one   \n", "two"]);
    assert_eq!(p.lines[0].width, natural("one"));
    assert_eq!(p.lines[0].glyphs.len(), 3);
}

#[test]
fn each_line_gets_its_own_width() {
    let text = "Take up and read; take up and read; take up and read; take up and read.";
    let p = set_paragraph(text, &whole(text, *TEXT), &style(Align::Ragged), &[40 * PT, 120 * PT]);
    assert!(p.lines[0].width <= 40 * PT);
    assert!(p.lines[1..].iter().all(|l| l.width <= 120 * PT));
    assert!(p.lines[1].width > 40 * PT, "the second line is wider than the first could be");
}

// ---------------------------------------------------------------------------
// Justifying

#[test]
fn justified_lines_fill_the_width_exactly() {
    let text = "I heard from a neighbouring house a voice, as of boy or girl, I know not, chanting, \
                and oft repeating, Take up and read; Take up and read.";
    for width in [90 * PT, 150 * PT, 151 * PT + 12_345] {
        let p = set(text, Align::Justified, width);
        let (last, rest) = p.lines.split_last().unwrap();
        assert!(!rest.is_empty());
        for line in rest {
            let words = text[line.text.clone()].trim_end();
            if words.contains(' ') {
                assert!(line.ratio >= -1.0 && line.ratio.is_finite(), "{line:?}");
                assert_eq!(line.width, width, "{words:?}");
            } else {
                // One word: no space inside the line to stretch.
                assert_eq!(line.ratio, f64::INFINITY);
                assert_eq!(line.width, natural(words));
            }
        }
        // The last line keeps its natural width, unless it's too long: then
        // its spaces shrink, as far as they can.
        let n = natural(text[last.text.clone()].trim_end());
        if n <= width {
            assert_eq!(last.width, n);
        } else {
            assert!(width <= last.width && last.width < n, "{last:?}");
        }
    }
}

#[test]
fn justified_lines_fill_their_own_widths() {
    let text = "I heard from a neighbouring house a voice, as of boy or girl, I know not, chanting, \
                and oft repeating, Take up and read; Take up and read.";
    let widths = [100 * PT, 150 * PT + 777];
    let p = set_paragraph(text, &whole(text, *TEXT), &style(Align::Justified), &widths);
    assert!(p.lines.len() >= 3);
    for (k, line) in p.lines[..p.lines.len() - 1].iter().enumerate() {
        if text[line.text.clone()].trim_end().contains(' ') {
            assert_eq!(line.width, widths[k.min(1)], "line {k}");
        }
    }
}

#[test]
fn justified_spaces_stretch_and_words_do_not() {
    let text = "Take up and read; take up and read.";
    let p = set(text, Align::Justified, natural("Take up and read;") + 9 * PT);
    let line = &p.lines[0];
    assert_eq!(&text[line.text.clone()], "Take up and read; ");
    let ragged = &set(text, Align::Ragged, WIDE).lines[0];
    // Glyphs inside a word keep their distances; the slack goes into the
    // three spaces, each a third of 9 pt (they're the same width).
    let word_starts = [0, 5, 8, 12];
    for (g, r) in line.glyphs.iter().zip(&ragged.glyphs) {
        assert_eq!(g.id, r.id);
        let spaces_before = word_starts.iter().filter(|&&s| s != 0 && s <= g.cluster).count() as i64;
        assert!((g.x - r.x - spaces_before * 3 * PT).abs() <= 1, "{g:?} vs {r:?}");
    }
}

#[test]
fn justified_lines_shrink_to_fit() {
    let text = "Take up and read.";
    let width = natural(text) - PT;
    let p = set(text, Align::Justified, width);
    assert_eq!(p.lines.len(), 1, "the spaces shrink rather than the line breaking");
    assert_eq!(p.lines[0].width, width);
    assert!(p.lines[0].ratio < 0.0 && p.lines[0].ratio >= -1.0);
    assert_eq!(set(text, Align::Ragged, width).lines.len(), 2, "ragged lines don't shrink");
}

#[test]
fn a_line_before_a_line_end_is_not_stretched() {
    let text = "Take up and read\nagain";
    let p = set(text, Align::Justified, 200 * PT);
    assert_eq!(texts(text, &p), ["Take up and read\n", "again"]);
    assert_eq!(p.lines[0].width, natural("Take up and read"));
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

/// Lowercase words, mostly single spaces, with a few double spaces, hyphens
/// and line ends.
fn random_text(rng: &mut Rng) -> String {
    let mut text = String::new();
    for _ in 0..rng.int(0, 40) {
        for _ in 0..rng.int(1, 12) {
            text.push((b'a' + rng.int(0, 25) as u8) as char);
        }
        text.push_str(match rng.int(0, 19) {
            0 => "\n",
            1 => "  ",
            2 => "-",
            3 => "\n\n",
            _ => " ",
        });
    }
    if rng.int(0, 1) == 0 {
        text.truncate(text.trim_end().len());
    }
    text
}

/// Where the first stretch of `text` ends: up to and including the spaces
/// after it.
fn first_piece(text: &str) -> usize {
    linebreaks(text).next().map_or(0, |(end, _)| end)
}

/// Whether `text` has a break opportunity before its end.
fn breakable(text: &str) -> bool {
    linebreaks(text).any(|(end, _)| end < text.len())
}

fn check(text: &str, p: &Paragraph, align: Align, width: i64) {
    let lines = &p.lines;
    assert!(!lines.is_empty());

    // The lines cover the text, in order.
    assert_eq!(lines[0].text.start, 0);
    assert_eq!(lines.last().unwrap().text.end, text.len());
    for w in lines.windows(2) {
        assert_eq!(w[0].text.end, w[1].text.start);
    }

    // Every letter and hyphen is drawn once, on its own line; spaces are
    // drawn except at a break; line ends never are.
    let mut drawn = Vec::new();
    for line in lines {
        let line_text = &text[line.text.clone()];
        let kept = line.text.start + line_text.trim_end().len();
        for g in &line.glyphs {
            assert!(line.text.contains(&g.cluster) && g.cluster < kept, "{g:?} on {line_text:?}");
        }
        assert!(line.glyphs.windows(2).all(|w| w[0].cluster < w[1].cluster));
        drawn.extend(line.glyphs.iter().map(|g| g.cluster));
    }
    let expected: Vec<usize> = text
        .char_indices()
        .filter(|&(i, c)| {
            !is_line_end(c) && {
                // A space is drawn unless only spaces and a line end (or the
                // end of the line) follow it.
                let line = lines.iter().find(|l| l.text.contains(&i)).unwrap();
                c != ' ' || i < line.text.start + text[line.text.clone()].trim_end().len()
            }
        })
        .map(|(i, _)| i)
        .collect();
    assert_eq!(drawn, expected);

    let ends_in_line_end = |line: &calamus::layout::Line| text[line.text.clone()].ends_with(is_line_end);
    for (k, line) in lines.iter().enumerate() {
        let line_text = &text[line.text.clone()];
        let trimmed = line_text.trim_end();
        let last = k + 1 == lines.len();
        let one_piece = !breakable(trimmed);
        match align {
            Align::Ragged => {
                assert_eq!(line.ratio, 0.0);
                // A line is set at its natural width, which fits unless the
                // line couldn't break any sooner.
                assert_eq!(line.width, natural(trimmed), "{line_text:?}");
                assert!(line.width <= width || one_piece, "{line_text:?} is {} wide in {width}", line.width);
                // First fit: the next stretch of text wouldn't have fitted.
                if !last && !ends_in_line_end(line) {
                    let next = line.text.end + first_piece(&text[line.text.end..]);
                    let longer = text[line.text.start..next].trim_end();
                    assert!(natural(longer) > width, "{longer:?} would have fitted in {width}");
                }
            }
            Align::Justified => {
                let stretchable = trimmed.contains(' ');
                if last || ends_in_line_end(line) {
                    if line.width != natural(trimmed) {
                        // Too long: shrunk, as far as it'll go.
                        assert!(natural(trimmed) > width && line.width >= width, "{line_text:?}");
                    }
                } else if stretchable && line.ratio >= -1.0 {
                    assert_eq!(line.width, width, "{line_text:?}");
                } else if stretchable {
                    assert!(one_piece || line.width > width);
                } else {
                    assert_eq!(line.width, natural(trimmed), "{line_text:?}");
                }
            }
        }
    }
}

#[test]
fn random_paragraphs() {
    let mut rng = Rng(168);
    for case in 0..600 {
        let text = random_text(&mut rng);
        let width = rng.int(20, 300) * PT + rng.int(0, PT - 1);
        let ragged = set(&text, Align::Ragged, width);
        let justified = set(&text, Align::Justified, width);
        check(&text, &ragged, Align::Ragged, width);
        check(&text, &justified, Align::Justified, width);
        assert!(justified.lines.len() <= ragged.lines.len(), "case {case}: shrinking never needs more lines");
    }
}

// ---------------------------------------------------------------------------
// The answer doesn't change

/// Augustine, *Confessions* VIII.12, in E. B. Pusey's translation (1838).
const TOLLE_LEGE: &str = "I heard from a neighbouring house a voice, as of boy or girl, I know not, \
    chanting, and oft repeating, “Take up and read; Take up and read.” Instantly, my countenance \
    altered, I began to think most intently whether children were wont in any kind of play to sing \
    such words: nor could I remember ever to have heard the like.";

fn dump(p: &Paragraph) -> String {
    let mut out = String::new();
    for line in &p.lines {
        out += &format!(
            "{:?} width {} baseline {} ratio {:?}\n",
            &TOLLE_LEGE[line.text.clone()],
            line.width,
            line.baseline,
            line.ratio
        );
        let glyphs: Vec<String> = line
            .glyphs
            .iter()
            .map(|g| if g.y == 0 { format!("{}@{}", g.id, g.x) } else { format!("{}@{},{}", g.id, g.x, g.y) })
            .collect();
        out += &format!("  {}\n", glyphs.join(" "));
    }
    out
}

/// Every glyph's id and position, against the file in `tests/snapshots`.
/// `CALAMUS_RECORD_SNAPSHOTS=1 cargo test` records it again; a change to it is
/// a change to every page.
#[test]
fn tolle_lege_snapshot() {
    let mut out = String::new();
    for (name, align) in [("ragged", Align::Ragged), ("justified", Align::Justified)] {
        out += &format!("# {name}, 12 pt on 16 pt, 240 pt wide\n");
        out += &dump(&set(TOLLE_LEGE, align, 240 * PT));
    }
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots/tolle_lege.txt");
    if std::env::var_os("CALAMUS_RECORD_SNAPSHOTS").is_some() {
        std::fs::write(path, &out).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(path).expect("no snapshot: run with CALAMUS_RECORD_SNAPSHOTS=1");
    assert!(out == expected, "the snapshot changed; got:\n{out}");
}

// ---------------------------------------------------------------------------
// Speed

#[test]
fn a_long_paragraph_sets_quickly() {
    let mut rng = Rng(9);
    let mut text = String::new();
    for _ in 0..20_000 {
        for _ in 0..rng.int(1, 10) {
            text.push((b'a' + rng.int(0, 25) as u8) as char);
        }
        text.push(' ');
    }
    for align in [Align::Ragged, Align::Justified] {
        let start = Instant::now();
        let p = set(&text, align, 300 * PT);
        let took = start.elapsed();
        assert!(p.lines.len() > 1000);
        assert!(took.as_secs_f64() < 2.0, "{align:?} took {took:?}");
    }
}

#[test]
fn breaks_are_only_where_unicode_allows() {
    // Sanity for the oracle above: the first piece of a text ends at its
    // first break opportunity.
    assert_eq!(first_piece("take up"), 5);
    assert_eq!(first_piece("well-known"), 5);
    assert!(matches!(linebreaks("a").last(), Some((1, BreakOpportunity::Mandatory))));
}
