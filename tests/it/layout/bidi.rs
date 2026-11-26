//! Text that runs both ways: each stretch shaped in its direction, and
//! each line's stretches in the order they're read on the page.

use std::sync::LazyLock;

use calamus::bidi::{self, Stretch};
use calamus::buffer::Direction;
use calamus::layout::{Align, Font, Line, PT, Paragraph, Run, Style, TextStyle, set_paragraph, set_paragraph_bidi};

static INTER: LazyLock<Font> =
    LazyLock::new(|| Font::from_bytes(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/Inter-Regular.ttf")).to_vec()).unwrap());
static HEBREW: LazyLock<Font> = LazyLock::new(|| {
    Font::from_bytes(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/NotoSansHebrew-Regular.ttf")).to_vec()).unwrap()
});
static STACK: LazyLock<Vec<&'static Font>> = LazyLock::new(|| vec![&*HEBREW]);

fn body() -> TextStyle<'static> {
    TextStyle { font: &INTER, fallback: STACK.as_slice(), language: None, size: 12 * PT, line_height: 16 * PT }
}

const WIDE: i64 = 1 << 40;

fn set(text: &str, isolates: &[Stretch], width: i64) -> Paragraph {
    let levels = bidi::levels(text, isolates);
    let runs = [Run { len: text.len(), style: body() }];
    let style = Style { strut: body(), align: Align::Ragged };
    set_paragraph_bidi(text, &runs, &style, &[width], bidi::base(text), &levels, false)
}

fn line(text: &str, isolates: &[Stretch]) -> Line {
    let p = set(text, isolates, WIDE);
    assert_eq!(p.lines.len(), 1);
    p.lines.into_iter().next().unwrap()
}

/// Where each glyph's text begins, in the order they're drawn.
fn drawn(line: &Line) -> Vec<usize> {
    line.glyphs.iter().map(|g| g.cluster).collect()
}

/// Glyphs are placed one after another from the left, none on top of the
/// one before.
fn in_a_row(line: &Line) {
    for pair in line.glyphs.windows(2) {
        assert!(pair[0].x + pair[0].advance <= pair[1].x + pair[1].advance.max(1) + PT, "{pair:?}");
        assert!(pair[0].x <= pair[1].x + PT, "{pair:?}");
    }
}

#[test]
fn hebrew_in_english_is_drawn_from_its_right() {
    // אבג is bytes 4, 6 and 8.
    let text = "abc אבג def";
    let l = line(text, &[]);
    assert_eq!(drawn(&l), [0, 1, 2, 3, 8, 6, 4, 10, 11, 12, 13]);
    in_a_row(&l);
    assert_eq!(l.glyphs.iter().map(|g| g.level).collect::<Vec<_>>(), [0, 0, 0, 0, 1, 1, 1, 0, 0, 0, 0]);
    // About as wide as the same letters set the wrong way round: only
    // the Hebrew's kerning differs.
    let runs = [Run { len: text.len(), style: body() }];
    let plain = set_paragraph(text, &runs, &Style { strut: body(), align: Align::Ragged }, &[WIDE]);
    assert!((l.width - plain.lines[0].width).abs() < PT / 2);
    assert_eq!(drawn(&plain.lines[0]), [0, 1, 2, 3, 4, 6, 8, 10, 11, 12, 13]);
    // And it starts at the left.
    assert_eq!(l.glyphs[0].x, 0);
}

#[test]
fn a_hebrew_paragraph_starts_at_the_right() {
    // אבג is 0, 2, 4; the space 6; abc 7, 8, 9.
    let text = "אבג abc";
    let p = set(text, &[], 300 * PT);
    let l = &p.lines[0];
    assert_eq!(drawn(l), [7, 8, 9, 6, 4, 2, 0]);
    in_a_row(l);
    let last = l.glyphs.last().unwrap();
    assert_eq!(last.x + last.advance, 300 * PT);
    assert_eq!(l.glyphs[0].x, 300 * PT - l.width);
}

#[test]
fn points_stay_on_their_letters() {
    let text = "בְּרֵאשִׁית";
    let l = line(text, &[]);
    // Last letter first, and each letter's points drawn with it: clusters
    // never go back up.
    let clusters = drawn(&l);
    assert!(clusters.windows(2).all(|w| w[0] >= w[1]), "{clusters:?}");
    assert_eq!(clusters.last(), Some(&0));
    // Marks take no room.
    assert!(l.glyphs.iter().any(|g| g.advance == 0));
    assert!(l.glyphs.iter().all(|g| g.id != 0), "the Hebrew font has every glyph");
}

#[test]
fn an_isolate_keeps_a_sentence_in_its_order() {
    // אבג 4, 6, 8; abc 11, 12, 13; דהו 15, 17, 19; end 22.
    let text = "say אבג abc דהו end";
    // The characters alone: three stretches side by side, so the Hebrew
    // reads second half first.
    assert_eq!(
        drawn(&line(text, &[])),
        [0, 1, 2, 3, 8, 6, 4, 10, 11, 12, 13, 14, 19, 17, 15, 21, 22, 23, 24]
    );
    // As one isolate, it runs from the right, with the English word in
    // its place.
    let sentence = Stretch { range: 4..21, direction: Direction::RightToLeft };
    let l = line(text, &[sentence]);
    assert_eq!(drawn(&l), [0, 1, 2, 3, 19, 17, 15, 14, 11, 12, 13, 10, 8, 6, 4, 21, 22, 23, 24]);
    in_a_row(&l);
}

#[test]
fn a_letter_typed_into_a_hebrew_word_stays_put() {
    // אב 4, 6; d 8; ג 9.
    let text = "abc אבdג def";
    // Left to the characters, ג jumps from the word's left end to its
    // right.
    assert_eq!(drawn(&line(text, &[])), [0, 1, 2, 3, 6, 4, 8, 9, 11, 12, 13, 14]);
    // In the word's isolate, the letter goes in where it was typed.
    let word = Stretch { range: 4..11, direction: Direction::RightToLeft };
    assert_eq!(drawn(&line(text, &[word])), [0, 1, 2, 3, 9, 8, 6, 4, 11, 12, 13, 14]);
}

#[test]
fn each_line_is_ordered_on_its_own() {
    // A Hebrew phrase too long for the line: its first words are on the
    // first line, and each line reads from its own right.
    let text = "aa אבג דהו זחט יכל bb";
    let p = set(text, &[], 60 * PT);
    assert!(p.lines.len() > 1, "{} lines", p.lines.len());
    let mut seen = Vec::new();
    for l in &p.lines {
        in_a_row(l);
        let mut clusters = drawn(l);
        assert!(clusters.iter().all(|c| l.text.contains(c)));
        clusters.sort_unstable();
        seen.extend(clusters);
    }
    // Every glyph on some line, once, lines in the text's order.
    assert!(seen.windows(2).all(|w| w[0] < w[1]), "{seen:?}");
    // The first line holds the phrase's first word, not its last.
    assert!(drawn(&p.lines[0]).contains(&3));
}
