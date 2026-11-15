//! Private-use characters (tolege#218): U+E000 to U+F8FF and planes 15 and 16
//! mean whatever each font says they mean, so Idem sets them only in fonts
//! allowed to ([`Font::with_private_use`]): in tolege, the fonts an account
//! installed, say for Tengwar or pIqaD, and never the bundled ones, whose
//! private-use glyphs would only be a guess.

use calamus::layout::{Align, Font, Line, PT, Run, Style, TextStyle, set_paragraph};

fn inter() -> Font {
    Font::from_bytes(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/Inter-Regular.ttf")).to_vec()).unwrap()
}

fn mono() -> Font {
    Font::from_bytes(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/JetBrainsMono-Regular.ttf")).to_vec()).unwrap()
}

fn line(text: &str, font: &Font, fallback: &[&Font]) -> Line {
    let s = TextStyle { font, fallback, language: None, size: 12 * PT, line_height: 16 * PT };
    let p = set_paragraph(text, &[Run { len: text.len(), style: s }], &Style { strut: s, align: Align::Ragged }, &[1 << 40]);
    p.lines.into_iter().next().unwrap()
}

fn glyphs(l: &Line) -> Vec<(u16, usize)> {
    l.glyphs.iter().map(|g| (g.id, g.font)).collect()
}

/// A powerline arrow: both test fonts have a glyph at this private-use code
/// point, Inter one of its own and JetBrains Mono a powerline arrow.
const ARROW: &str = "\u{E0A0}";

#[test]
fn a_font_not_allowed_private_use_never_sets_it() {
    let (inter, mono) = (inter(), mono());
    assert!(inter.has_char('\u{E0A0}') && mono.has_char('\u{E0A0}'));
    // Not even as the run's own font.
    assert_eq!(glyphs(&line(ARROW, &inter, &[])), [(0, 0)]);
    assert_eq!(glyphs(&line(ARROW, &inter, &[&mono])), [(0, 0)]);
}

#[test]
fn a_font_allowed_private_use_sets_it_wherever_it_is() {
    let (inter, mono) = (inter(), mono());
    let mono_id = line(ARROW, &mono.clone().with_private_use(), &[]).glyphs[0].id;
    assert_ne!(mono_id, 0);
    // The account's font, as a fallback, wins over the body font that has
    // the code point but wasn't allowed it.
    let account = mono.with_private_use();
    assert_eq!(glyphs(&line(ARROW, &inter, &[&account])), [(mono_id, 1)]);
    // As the run's own font too.
    let own = inter.clone().with_private_use();
    assert_eq!(glyphs(&line(ARROW, &own, &[&account])).len(), 1);
    assert_eq!(line(ARROW, &own, &[&account]).glyphs[0].font, 0);
}

#[test]
fn the_rest_of_the_text_doesnt_care() {
    let (inter, mono) = (inter(), mono());
    let allowed = inter.clone().with_private_use();
    let text = "Take up and read: naïve café";
    assert_eq!(glyphs(&line(text, &inter, &[&mono])), glyphs(&line(text, &allowed, &[&mono])));
}

#[test]
fn a_cluster_with_a_private_use_character_needs_an_allowed_font() {
    // A private-use letter with an accent on it: the accent doesn't make
    // the body font acceptable.
    let (inter, mono) = (inter(), mono());
    let text = "\u{E0A0}\u{0301}";
    assert_eq!(glyphs(&line(text, &inter, &[])), [(0, 0)]);
    let account = mono.with_private_use();
    let l = line(text, &inter, &[&account]);
    assert!(l.glyphs.iter().all(|g| g.font == 1), "{:?}", l.glyphs);
}
