//! Language tags (tolege#218): the same character, drawn the way its language
//! writes it. Han unification gives Japanese, Chinese and Korean one code
//! point for what each writes a little differently, and a CJK font that
//! covers them all picks the forms by language (OpenType's `locl`).

use super::fetched;

use std::sync::LazyLock;

use calamus::layout::{Align, Font, PT, Run, Style, TextStyle, set_paragraph};

static INTER: LazyLock<Font> =
    LazyLock::new(|| Font::from_bytes(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/Inter-Regular.ttf")).to_vec()).unwrap());
static NOTO: LazyLock<Font> = LazyLock::new(|| fetched::font("NotoSans-Regular.ttf"));
static CJK: LazyLock<Font> = LazyLock::new(|| fetched::font("NotoSansCJKjp-Regular.otf"));
static CJK_FALLBACK: LazyLock<[&'static Font; 1]> = LazyLock::new(|| [&*CJK]);

fn style(font: &'static Font, language: Option<&'static str>) -> TextStyle<'static> {
    TextStyle { font, fallback: &[], language, size: 12 * PT, line_height: 16 * PT }
}

/// The glyph ids of `text` set in these runs, on one line.
fn ids_in(text: &str, runs: &[Run<'static>]) -> Vec<u16> {
    let p = set_paragraph(text, runs, &Style { strut: runs[0].style, align: Align::Ragged }, &[1 << 40]);
    assert_eq!(p.lines.len(), 1);
    p.lines[0].glyphs.iter().map(|g| g.id).collect()
}

fn ids(text: &str, s: TextStyle<'static>) -> Vec<u16> {
    ids_in(text, &[Run { len: text.len(), style: s }])
}

/// 遍 ("everywhere", "a time"): five languages, five forms in Noto Sans CJK.
const BIAN: &str = "遍";
const CJK_LANGUAGES: [&str; 5] = ["ja", "zh-Hans", "zh-Hant", "zh-HK", "ko"];

#[test]
fn han_takes_its_languages_form() {
    let forms: Vec<Vec<u16>> = CJK_LANGUAGES.iter().map(|&l| ids(BIAN, style(&CJK, Some(l)))).collect();
    for (i, a) in forms.iter().enumerate() {
        for (j, b) in forms.iter().enumerate().skip(i + 1) {
            assert_ne!(a, b, "{} and {} should differ", CJK_LANGUAGES[i], CJK_LANGUAGES[j]);
        }
    }
    // Shared forms stay shared: 直 is one form in Japanese and Korean, one
    // in Simplified and Traditional Chinese, and a third in Hong Kong's.
    let zhi = |l| ids("直", style(&CJK, Some(l)));
    assert_eq!(zhi("ja"), zhi("ko"));
    assert_eq!(zhi("zh-Hans"), zhi("zh-Hant"));
    assert_ne!(zhi("ja"), zhi("zh-Hans"));
    assert_ne!(zhi("zh-HK"), zhi("zh-Hant"));
}

#[test]
fn no_language_is_the_fonts_own_default() {
    // This build of Noto Sans CJK is the Japanese one.
    assert_eq!(ids(BIAN, style(&CJK, None)), ids(BIAN, style(&CJK, Some("ja"))));
    // A tag nobody's font knows changes nothing either.
    assert_eq!(ids(BIAN, style(&CJK, Some("xx"))), ids(BIAN, style(&CJK, None)));
}

#[test]
fn tags_ignore_case() {
    assert_eq!(ids(BIAN, style(&CJK, Some("ZH-hant"))), ids(BIAN, style(&CJK, Some("zh-Hant"))));
}

#[test]
fn fallback_fonts_get_the_language_too() {
    // Body text in Inter, with the CJK font as fallback: the Chinese form
    // still comes through.
    for l in CJK_LANGUAGES {
        let body = TextStyle { fallback: CJK_FALLBACK.as_slice(), ..style(&INTER, Some(l)) };
        assert_eq!(ids(BIAN, body), ids(BIAN, style(&CJK, Some(l))), "{l}");
    }
}

#[test]
fn each_run_has_its_own_language() {
    // A Japanese note quoting Korean: one font, two languages, one line.
    let text = "遍 遍";
    let runs = [Run { len: 4, style: style(&CJK, Some("ja")) }, Run { len: 3, style: style(&CJK, Some("ko")) }];
    let got = ids_in(text, &runs);
    let (ja, ko) = (ids(BIAN, style(&CJK, Some("ja"))), ids(BIAN, style(&CJK, Some("ko"))));
    assert_eq!((got[0], *got.last().unwrap()), (ja[0], ko[0]));
}

#[test]
fn not_only_han() {
    // Serbian writes б differently from Russian; Romanian puts a comma, not
    // a cedilla, under its s.
    assert_ne!(ids("б", style(&NOTO, Some("sr"))), ids("б", style(&NOTO, Some("ru"))));
    assert_ne!(ids("ş", style(&INTER, Some("ro"))), ids("ş", style(&INTER, Some("tr"))));
}
