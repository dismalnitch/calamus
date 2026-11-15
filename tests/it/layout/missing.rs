//! What Idem draws when no font has a character (tolege#218): each grapheme
//! cluster no font can start is one missing glyph, its run's font's
//! `.notdef` (glyph 0), however many characters it holds. A caller that
//! wants something better adds a font that has everything to the end of the
//! fallback list: Unicode's Last Resort font draws each character as its
//! Unicode block's sign.

use super::fetched;

use std::sync::LazyLock;

use calamus::layout::{Align, Font, Line, PT, Run, Style, TextStyle, set_paragraph};

static INTER: LazyLock<Font> =
    LazyLock::new(|| Font::from_bytes(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/Inter-Regular.ttf")).to_vec()).unwrap());
static MONO: LazyLock<Font> =
    LazyLock::new(|| Font::from_bytes(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/JetBrainsMono-Regular.ttf")).to_vec()).unwrap());
static NOTO: LazyLock<Font> = LazyLock::new(|| fetched::font("NotoSans-Regular.ttf"));
static CJK: LazyLock<Font> = LazyLock::new(|| fetched::font("NotoSansCJKjp-Regular.otf"));
static RESORT: LazyLock<Font> = LazyLock::new(|| fetched::font("LastResortHE-Regular.ttf"));

static STACK: LazyLock<[&'static Font; 2]> = LazyLock::new(|| [&*NOTO, &*CJK]);
static WITH_RESORT: LazyLock<[&'static Font; 3]> = LazyLock::new(|| [&*NOTO, &*CJK, &*RESORT]);
const CJK_AT: usize = 2;
const RESORT_AT: usize = 3;

fn style(font: &'static Font, fallback: &'static [&'static Font]) -> TextStyle<'static> {
    TextStyle { font, fallback, language: None, size: 12 * PT, line_height: 16 * PT }
}

fn body() -> TextStyle<'static> {
    style(&INTER, STACK.as_slice())
}

fn line(text: &str, s: TextStyle<'static>) -> Line {
    let p = set_paragraph(text, &[Run { len: text.len(), style: s }], &Style { strut: s, align: Align::Ragged }, &[1 << 40]);
    assert_eq!(p.lines.len(), 1, "{text:?}");
    p.lines.into_iter().next().unwrap()
}

/// Nobody here has emoji, flags or runes.
const MISSING: [&str; 5] = ["😀", "🇺🇸", "👨\u{200D}👩\u{200D}👧", "ᚠ", "ᚠ\u{0301}"];

#[test]
fn a_cluster_no_font_has_is_one_missing_glyph() {
    let width = line("😀", body()).width;
    assert!(width > 0, "the missing glyph takes room");
    for text in MISSING {
        let l = line(text, body());
        assert_eq!(l.glyphs.len(), 1, "{text:?}: {:?}", l.glyphs);
        let g = l.glyphs[0];
        assert_eq!((g.id, g.x, g.y, g.cluster, g.run, g.font), (0, 0, 0, 0, 0, 0), "{text:?}");
        // As wide as any other: the run's own font's missing glyph.
        assert_eq!(l.width, width, "{text:?}");
    }
}

#[test]
fn the_missing_glyph_is_the_runs_own_fonts() {
    // JetBrains Mono's missing glyph is its own, at its own width.
    let mono = style(&MONO, STACK.as_slice());
    let inter = line("😀", body());
    let l = line("😀", mono);
    assert_eq!((l.glyphs[0].id, l.glyphs[0].font), (0, 0));
    assert_ne!(l.width, inter.width);
}

#[test]
fn a_missing_cluster_leaves_its_neighbours_alone() {
    let l = line("ab😀cd", body());
    let (ab, box_, cd) = (line("ab", body()), line("😀", body()), line("cd", body()));
    let ids: Vec<u16> = l.glyphs.iter().map(|g| g.id).collect();
    let own: Vec<u16> = ab.glyphs.iter().chain(&box_.glyphs).chain(&cd.glyphs).map(|g| g.id).collect();
    assert_eq!(ids, own);
    assert_eq!(l.glyphs[2].x, ab.width);
    assert_eq!(l.glyphs[3].x, ab.width + box_.width);
    assert_eq!(l.width, ab.width + box_.width + cd.width);
    assert_eq!(l.glyphs.iter().map(|g| g.cluster).collect::<Vec<_>>(), [0, 1, 2, 6, 7]);
}

#[test]
fn a_letter_shows_when_only_its_accent_is_missing() {
    // Nobody here has a combining macron below on 日 (CJK has the 日, Noto
    // the mark): the 日 is still drawn, in the CJK font, which has it.
    assert!(!CJK.has_char('\u{0331}') && NOTO.has_char('\u{0331}') && !NOTO.has_char('日'));
    let l = line("日\u{0331}", body());
    assert!(l.glyphs.iter().all(|g| g.font == CJK_AT), "{:?}", l.glyphs);
    assert_eq!(l.glyphs[0].id, line("日", body()).glyphs[0].id);

    // And with Inter alone, the g of g-macron-below is still a g.
    assert!(!INTER.has_char('\u{0331}'));
    let l = line("g\u{0331}", style(&INTER, &[]));
    assert_eq!(l.glyphs[0].id, line("g", style(&INTER, &[])).glyphs[0].id);
    assert_ne!(l.glyphs[0].id, 0);
}

#[test]
fn an_accent_without_its_letter_goes_with_it() {
    // Inter has the acute but nobody has the rune: one missing glyph for
    // both, not a box and a floating accent.
    assert!(INTER.has_char('\u{0301}'));
    let l = line("ᚠ\u{0301}", body());
    assert_eq!(l.glyphs.len(), 1);
    assert_eq!(l.glyphs[0].id, 0);
}

#[test]
fn last_resort_at_the_end_draws_everything() {
    let s = style(&INTER, WITH_RESORT.as_slice());
    for text in MISSING {
        let l = line(text, s);
        assert!(l.glyphs.iter().all(|g| g.font == RESORT_AT && g.id != 0), "{text:?}: {:?}", l.glyphs);
    }
    // Only what nobody else has: Latin is still Inter's, Japanese CJK's.
    let l = line("a日😀", s);
    assert_eq!(l.glyphs.iter().map(|g| g.font).collect::<Vec<_>>(), [0, CJK_AT, RESORT_AT]);
}
