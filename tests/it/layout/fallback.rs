//! Font fallback (tolege#218): each grapheme cluster set in the first of its run's
//! fonts that has it, never split between two. Body text is Inter, which has
//! Latin, Greek and Cyrillic, falling back to Noto for Hebrew, Arabic,
//! Devanagari, Thai, Japanese and the Latin Inter lacks.

use super::fetched;

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{LazyLock, Mutex};

use calamus::layout::{Align, Font, Line, PT, Paragraph, Run, Style, TextStyle, set_paragraph};
use unicode_segmentation::UnicodeSegmentation;

static INTER: LazyLock<Font> =
    LazyLock::new(|| Font::from_bytes(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/Inter-Regular.ttf")).to_vec()).unwrap());
static INTER_BOLD: LazyLock<Font> =
    LazyLock::new(|| Font::from_bytes(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/Inter-Bold.ttf")).to_vec()).unwrap());
static NOTO: LazyLock<Font> = LazyLock::new(|| fetched::font("NotoSans-Regular.ttf"));
static HEBREW: LazyLock<Font> = LazyLock::new(|| fetched::font("NotoSansHebrew-Regular.ttf"));
static ARABIC: LazyLock<Font> = LazyLock::new(|| fetched::font("NotoSansArabic-Regular.ttf"));
static DEVANAGARI: LazyLock<Font> = LazyLock::new(|| fetched::font("NotoSansDevanagari-Regular.ttf"));
static THAI: LazyLock<Font> = LazyLock::new(|| fetched::font("NotoSansThai-Regular.ttf"));
static CJK: LazyLock<Font> = LazyLock::new(|| fetched::font("NotoSansCJKjp-Regular.otf"));

/// The fallback list, in order: Inter's fallback is index 1 onwards.
static STACK: LazyLock<Vec<&'static Font>> =
    LazyLock::new(|| vec![&*HEBREW, &*ARABIC, &*DEVANAGARI, &*THAI, &*CJK, &*NOTO]);
const HEBREW_AT: usize = 1;
const ARABIC_AT: usize = 2;
const DEVANAGARI_AT: usize = 3;
const THAI_AT: usize = 4;
const CJK_AT: usize = 5;
const NOTO_AT: usize = 6;

fn body() -> TextStyle<'static> {
    TextStyle { font: &INTER, fallback: STACK.as_slice(), language: None, size: 12 * PT, line_height: 16 * PT }
}

fn bold() -> TextStyle<'static> {
    TextStyle { font: &INTER_BOLD, ..body() }
}

/// `font` alone, no fallback, at body size.
fn only(font: &'static Font) -> TextStyle<'static> {
    TextStyle { font, fallback: &[], ..body() }
}

const WIDE: i64 = 1 << 50;

fn set_in(text: &str, s: TextStyle<'static>, align: Align, width: i64) -> Paragraph {
    set_paragraph(text, &[Run { len: text.len(), style: s }], &Style { strut: body(), align }, &[width])
}

/// `text` on one line, in one style.
fn line(text: &str, s: TextStyle<'static>) -> Line {
    let p = set_in(text, s, Align::Ragged, WIDE);
    assert_eq!(p.lines.len(), 1, "{text:?} should fit on one line");
    p.lines.into_iter().next().unwrap()
}

/// The fonts the glyphs of `text[range]` came from, on `line`.
fn fonts_of(line: &Line, range: Range<usize>) -> Vec<usize> {
    let mut fonts: Vec<usize> = line.glyphs.iter().filter(|g| range.contains(&g.cluster)).map(|g| g.font).collect();
    fonts.dedup();
    fonts
}

fn range_of(text: &str, word: &str) -> Range<usize> {
    let start = text.find(word).unwrap_or_else(|| panic!("{word:?} isn't in {text:?}"));
    start..start + word.len()
}

// ---------------------------------------------------------------------------
// Which font

#[test]
fn every_script_finds_its_font() {
    let text = "Take שָׁלוֹם مرحبا नमस्ते สวัสดี 日本語。 λόγος мир";
    let l = line(text, body());
    for (word, font) in [
        ("Take", 0),
        ("שָׁלוֹם", HEBREW_AT),
        ("مرحبا", ARABIC_AT),
        ("नमस्ते", DEVANAGARI_AT),
        ("สวัสดี", THAI_AT),
        ("日本語。", CJK_AT),
        ("λόγος", 0),
        ("мир", 0),
    ] {
        assert_eq!(fonts_of(&l, range_of(text, word)), [font], "{word}");
    }
    // The spaces between are Inter's: the first font that has a space.
    for g in &l.glyphs {
        if text[g.cluster..].starts_with(' ') {
            assert_eq!(g.font, 0);
        }
    }
    assert!(l.glyphs.iter().all(|g| g.id != 0), "nothing is missing");
}

#[test]
fn fonts_are_tried_in_order() {
    assert!(NOTO.has_char('A') && CJK.has_char('A') && !INTER.has_char('\u{A78B}'));
    let both = [&*NOTO, &*CJK];
    let other_way = [&*CJK, &*NOTO];
    let s = |fallback: &'static [&'static Font]| TextStyle { fallback, ..body() };
    let leak = |f: [&'static Font; 2]| -> &'static [&'static Font] { Box::leak(Box::new(f)) };
    let (both, other_way) = (leak(both), leak(other_way));
    // Inter has "A", so neither fallback is asked.
    assert_eq!(fonts_of(&line("A", s(both)), 0..1), [0]);
    // Inter lacks the saltillo (Ꞌ); Noto, first, has it.
    assert_eq!(fonts_of(&line("\u{A78B}", s(both)), 0..3), [1], "NOTO first");
    assert_eq!(fonts_of(&line("日", s(both)), 0..3), [2], "only CJK has it");
    assert_eq!(fonts_of(&line("日", s(other_way)), 0..3), [1], "CJK first");
}

#[test]
fn a_cluster_is_never_split_between_fonts() {
    // g with a macron below, as Semitic transliteration writes it: Inter has
    // the g but not the mark, so the whole cluster goes to Noto, which has
    // both, and the mark sits on its own g.
    assert!(INTER.has_char('g') && !INTER.has_char('\u{0331}') && NOTO.has_char('\u{0331}'));
    let text = "g\u{0331}";
    assert_eq!(text.graphemes(true).count(), 1);
    let l = line(text, body());
    assert_eq!(fonts_of(&l, 0..text.len()), [NOTO_AT]);
    let own = line(text, only(&NOTO));
    assert_eq!(ids(&l), ids(&own), "set as Noto sets it");

    // The other way round: a letter Inter lacks with an accent it has.
    assert!(!INTER.has_char('\u{2C65}') && INTER.has_char('\u{0301}'));
    let text = "\u{2C65}\u{0301}";
    assert_eq!(fonts_of(&line(text, body()), 0..text.len()), [NOTO_AT]);
}

fn ids(line: &Line) -> Vec<u16> {
    line.glyphs.iter().map(|g| g.id).collect()
}

#[test]
fn a_fallback_font_shapes_its_script_whole() {
    // Conjuncts, joining forms and stacked marks need the stretch shaped in
    // one go, in its own font: the same glyphs as that font alone gives.
    for (text, font, at) in [
        ("क्षत्रिय", &*DEVANAGARI, DEVANAGARI_AT),
        ("السلام", &*ARABIC, ARABIC_AT),
        ("בְּרֵאשִׁית", &*HEBREW, HEBREW_AT),
        ("กรุงเทพมหานคร", &*THAI, THAI_AT),
    ] {
        let l = line(text, body());
        let own = line(text, only(font));
        assert_eq!(ids(&l), ids(&own), "{text}");
        assert!(l.glyphs.iter().all(|g| g.font == at), "{text}");
        let xs = |l: &Line| l.glyphs.iter().map(|g| (g.x, g.y)).collect::<Vec<_>>();
        assert_eq!(xs(&l), xs(&own), "{text}");
    }
}

#[test]
fn ignorable_characters_dont_choose_a_font() {
    // A tag character (Inter has none) joins the cluster of the x before
    // it; the x stays in Inter rather than going looking for a font with
    // tags.
    assert!(!INTER.has_char('\u{E0061}'));
    let text = "x\u{E0061}y";
    assert_eq!(text.graphemes(true).count(), 2);
    let l = line(text, body());
    assert!(l.glyphs.iter().all(|g| g.font == 0), "{:?}", l.glyphs);

    // A zero-width joiner on its own is a cluster of nothing but an
    // ignorable: it stays in the font before it.
    assert!(!INTER.has_char('\u{200D}'));
    let text = "日\u{200D}";
    let l = line(text, body());
    assert!(l.glyphs.iter().all(|g| g.font == CJK_AT), "{:?}", l.glyphs);
}

#[test]
fn fallback_text_asks_for_its_own_room() {
    // Noto Sans CJK reaches higher and lower than Inter: a line with
    // Japanese on it is as tall as the CJK font asks at this line height.
    let cjk = line("日", only(&CJK));
    let latin = line("x", only(&INTER));
    assert!(cjk.baseline > latin.baseline, "the test needs a taller font");
    let mixed = line("x 日", body());
    let above = cjk.baseline.max(latin.baseline);
    let below = (cjk.height - cjk.baseline).max(latin.height - latin.baseline);
    assert_eq!((mixed.baseline, mixed.height), (above, above + below));
    // And a line of Latin alone isn't.
    assert_eq!(line("x", body()).height, latin.height);
}

#[test]
fn no_fallback_changes_nothing() {
    let text = "Take up and read; 日本語 שָׁלוֹם";
    let l = line(text, only(&INTER));
    assert!(l.glyphs.iter().all(|g| g.font == 0));
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

const WORDS: &[&str] = &[
    "take", "up", "and", "read", "café", "naïve", "g\u{0331}", "o\u{030B}", "\u{2C65}\u{0301}", "\u{A78B}",
    "λόγος", "мир", "שָׁלוֹם", "בְּרֵאשִׁית", "مرحبا", "السلام", "नमस्ते", "क्षत्रिय", "สวัสดี", "日本語", "東京。",
    "漢字", "x\u{E0061}", "1984", "—", "«a»", "日\u{200D}本", "😀", "🇺🇸", "👨\u{200D}👩\u{200D}👧", "ᚠ\u{0301}",
    "日\u{0331}", "\u{E000}",
];

fn random_text(rng: &mut Rng) -> String {
    let mut text = String::new();
    for _ in 0..rng.int(0, 25) {
        for _ in 0..rng.int(1, 3) {
            text += WORDS[rng.int(0, WORDS.len() as i64 - 1) as usize];
        }
        text += match rng.int(0, 15) {
            0 => "\n",
            1 => "-",
            _ => " ",
        };
    }
    text
}

/// Runs cut at spaces, each body or bold.
fn random_runs(rng: &mut Rng, text: &str) -> Vec<Run<'static>> {
    let mut runs = Vec::new();
    let mut start = 0;
    for (i, _) in text.match_indices(' ') {
        if rng.int(0, 2) == 0 {
            runs.push(Run { len: i + 1 - start, style: if rng.int(0, 1) == 0 { body() } else { bold() } });
            start = i + 1;
        }
    }
    runs.push(Run { len: text.len() - start, style: body() });
    runs
}

/// Whether `font` has `c`, remembered: `has_char` reads the font's tables.
fn has(font: &'static Font, c: char) -> bool {
    static SEEN: LazyLock<Mutex<HashMap<(usize, char), bool>>> = LazyLock::new(Default::default);
    let key = (font as *const Font as usize, c);
    *SEEN.lock().unwrap().entry(key).or_insert_with(|| font.has_char(c))
}

fn ignorable(c: char) -> bool {
    matches!(c, '\u{200D}' | '\u{E0000}'..='\u{E0FFF}')
}

/// What the rule makes of `cluster` in `style`.
enum Expected {
    /// Nothing but ignorables: whatever font came before.
    Any,
    /// Set in this font.
    Font(usize),
    /// No font has even its first character: one missing glyph.
    Missing,
}

fn expected_font(style: &TextStyle<'static>, cluster: &str) -> Expected {
    let chars: Vec<char> = cluster.chars().filter(|&c| !ignorable(c)).collect();
    if chars.is_empty() {
        return Expected::Any;
    }
    // None of these fonts may set private-use characters, though Inter has
    // some.
    if chars.iter().any(|&c| ('\u{E000}'..='\u{F8FF}').contains(&c)) {
        return Expected::Missing;
    }
    let fonts: Vec<&'static Font> = std::iter::once(style.font).chain(style.fallback.iter().copied()).collect();
    if let Some(i) = fonts.iter().position(|f| chars.iter().all(|&c| has(f, c))) {
        return Expected::Font(i);
    }
    match fonts.iter().position(|f| has(f, chars[0])) {
        Some(i) => Expected::Font(i),
        None => Expected::Missing,
    }
}

fn check(text: &str, runs: &[Run<'static>], p: &Paragraph) {
    let mut run_of = Vec::with_capacity(text.len());
    for (r, run) in runs.iter().enumerate() {
        run_of.extend(std::iter::repeat_n(r, run.len));
    }
    for line in &p.lines {
        // Every glyph is in the font the rule picks for its cluster, and so
        // every glyph of a cluster is in the same font.
        for (start, cluster) in text[line.text.clone()].grapheme_indices(true) {
            let at = line.text.start + start;
            let range = at..at + cluster.len();
            let fonts = fonts_of(line, range.clone());
            assert!(fonts.len() <= 1, "{cluster:?} split between fonts {fonts:?}");
            let style = runs[run_of[at]].style;
            let glyphs: Vec<_> = line.glyphs.iter().filter(|g| range.contains(&g.cluster)).collect();
            if glyphs.is_empty() {
                continue; // spaces and line ends at a break aren't drawn
            }
            match expected_font(&style, cluster) {
                Expected::Any => {}
                Expected::Font(want) => {
                    assert_eq!(fonts.first(), Some(&want), "{cluster:?} in {text:?}");
                    assert!(glyphs[0].id != 0, "{cluster:?}'s first character is in its font");
                }
                Expected::Missing => {
                    assert!(glyphs.len() == 1 && glyphs[0].id == 0 && glyphs[0].font == 0, "{cluster:?}: {glyphs:?}");
                }
            }
        }
        for g in &line.glyphs {
            assert!(g.font <= runs[g.run].style.fallback.len());
        }
    }
}

#[test]
fn random_paragraphs_in_many_scripts() {
    let mut rng = Rng(218);
    for case in 0..150 {
        let text = random_text(&mut rng);
        let runs = random_runs(&mut rng, &text);
        let width = rng.int(40, 300) * PT;
        for align in [Align::Ragged, Align::Justified] {
            let style = Style { strut: body(), align };
            let p = set_paragraph(&text, &runs, &style, &[width]);
            assert_eq!(p, set_paragraph(&text, &runs, &style, &[width]), "case {case}");
            check(&text, &runs, &p);
        }
    }
}
