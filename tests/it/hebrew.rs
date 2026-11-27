//! Hebrew, the hard parts: points and accents, words, direction, the
//! cursor, selection, line breaking, typing, and keeping it all.
//!
//! Next version: every test here is ignored, so none of it turns the build
//! red while the editor's foundations are still moving. Run them with
//!
//!     cargo test --test it hebrew:: -- --ignored
//!
//! and see docs/hebrew-tests.md for what each one found, which ones say a
//! choice still to be made ("Decision:"), and which edge cases need an API
//! the editor doesn't have yet. When the suite is taken on, a test that
//! passes loses its `#[ignore]`; one that fails is fixed, or its
//! expectation is changed by a decision, and then loses it.
//!
//! The text is pointed Biblical Hebrew where that's the hard case (Genesis
//! 1:1 in the Westminster Leningrad Codex's spelling), and modern Hebrew
//! where that is. Offsets are UTF-8 bytes, as everywhere in calamus.

use std::sync::Arc;

use calamus::Editor;
use calamus::buffer::{Edge, Unit};
use calamus::editor::{Direction, Place};
use calamus::layout::{Font, PT};
use unicode_segmentation::UnicodeSegmentation;

/// Genesis 1:1, pointed and accented, as the WLC spells it.
const GEN_1_1: &str = "בְּרֵאשִׁ֖ית בָּרָ֣א אֱלֹהִ֑ים אֵ֥ת הַשָּׁמַ֖יִם וְאֵ֥ת הָאָֽרֶץ׃";

fn fonts() -> (Arc<Font>, Vec<Arc<Font>>) {
    let inter = Font::from_bytes(include_bytes!("../../fonts/Inter-Regular.ttf").to_vec()).unwrap();
    let noto = Font::from_bytes(include_bytes!("../../fonts/NotoSansHebrew-Regular.ttf").to_vec()).unwrap();
    (Arc::new(inter), vec![Arc::new(noto)])
}

/// An editor wide enough that nothing here wraps: tests that walk a line
/// need it all on one. Tests of wrapping say their width.
fn he(text: &str) -> Editor {
    wide(text, 2000)
}

fn wide(text: &str, width: i64) -> Editor {
    let (font, fallback) = fonts();
    Editor::new(text, font, fallback, 17 * PT, 24 * PT, width * PT)
}

/// Every grapheme cluster boundary in `s`, the ends included.
fn boundaries(s: &str) -> Vec<usize> {
    let mut out: Vec<usize> = s.grapheme_indices(true).map(|(i, _)| i).collect();
    out.push(s.len());
    out
}

fn x(e: &Editor, offset: usize) -> i64 {
    e.caret_rect(Place::downstream(offset)).x
}

/// The byte offset of the first `needle` in the editor's text.
fn at(e: &Editor, needle: &str) -> usize {
    e.text().find(needle).unwrap_or_else(|| panic!("{needle:?} isn't in {:?}", e.text()))
}

/// Types `text` a character at a time, as a keyboard does.
fn type_in(e: &mut Editor, text: &str) {
    for c in text.chars() {
        let sel = e.selection();
        e.replace(sel, c.encode_utf8(&mut [0; 4]));
    }
}

/// The words a double click finds, left to right in the text.
fn words(e: &Editor) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < e.len() {
        match e.unit_range_at(i, Unit::MamreWord) {
            Some(r) if r.start >= i || out.is_empty() => {
                out.push(e.text_in(r).to_string());
                i = r.end.max(i + 1);
            }
            _ => i = e.offset_by_clusters(i, 1).max(i + 1),
        }
    }
    out.dedup();
    out
}

/// Walks the cursor with the arrow keys from the line's left edge to its
/// right, collecting the offsets it stands at.
fn walk_right(e: &Editor) -> Vec<usize> {
    let line = &e.lines()[0];
    let first = line.stops[line.visual[0]];
    let mut at = Place::downstream(first.offset);
    let mut out = vec![at.offset];
    for _ in 0..(e.len() * 4 + 8) {
        match e.place_beside(at, true) {
            Some(next) if next != at => {
                at = next;
                out.push(at.offset);
            }
            _ => break,
        }
    }
    out
}

// MARK: Points and accents

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_pointed_letter_is_one_cluster() {
    // Bet, dagesh, sheva: one letter to the cursor.
    let e = he("בְּ");
    assert_eq!(e.offset_by_clusters(0, 1), e.len());
    // Shin, shin dot, qamats, an accent (munah), and meteg.
    let e = he("שָֽׁ֣");
    assert_eq!(e.offset_by_clusters(0, 1), e.len());
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn every_mark_hebrew_has_stays_on_its_letter() {
    // Every Hebrew point, accent and mark (U+0591–U+05C7, the
    // non-spacing ones) after a letter makes one cluster with it.
    for mark in ('\u{0591}'..='\u{05C7}').filter(|c| !matches!(c, '\u{05BE}' | '\u{05C0}' | '\u{05C3}' | '\u{05C6}')) {
        let text = format!("ב{mark}");
        let e = he(&text);
        assert_eq!(e.offset_by_clusters(0, 1), e.len(), "U+{:04X} split from its letter", mark as u32);
    }
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn the_arrows_never_stop_inside_a_letter() {
    let e = he(GEN_1_1);
    let ok = boundaries(e.text());
    for o in walk_right(&e) {
        assert!(ok.contains(&o), "the cursor stopped at {o}, inside a letter");
    }
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_touch_never_lands_inside_a_letter() {
    let e = he(GEN_1_1);
    let ok = boundaries(e.text());
    let line = &e.lines()[0];
    for x in (0..=line.width).step_by((PT / 2) as usize) {
        let p = e.closest_place(x, line.top + line.height / 2);
        assert!(ok.contains(&p.offset), "a touch at x {} landed at {}, inside a letter", x / PT, p.offset);
    }
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn every_letter_of_a_verse_can_be_reached() {
    // Walking right across the line stands at every letter boundary.
    let e = he(GEN_1_1);
    let seen = walk_right(&e);
    for b in boundaries(e.text()) {
        assert!(seen.contains(&b), "never stood at {b}");
    }
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn backspace_takes_a_whole_pointed_letter() {
    // Decision: a pointed letter goes whole, as on macOS and in Word. Some
    // Hebrew keyboards and editors take only the last point; if Hebrew
    // writers want that, this test changes.
    let mut e = he("שָׁלוֹם");
    let end = e.len();
    e.set_selection(std::range::Range { start: end, end });
    e.delete_backward();
    assert_eq!(e.text(), "שָׁלוֹ");
    e.delete_backward();
    assert_eq!(e.text(), "שָׁל");
    e.delete_backward();
    assert_eq!(e.text(), "שָׁ");
    e.delete_backward();
    assert_eq!(e.text(), "");
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn points_stay_in_the_order_they_were_typed() {
    // Unicode's normal form would reorder points by their combining class,
    // which puts some Biblical Hebrew in the wrong order: the text is kept
    // exactly as typed, never normalized.
    for text in ["בּ\u{05B8}", "ב\u{05B8}\u{05BC}", "יְרוּשָׁלַ\u{05B4}ם", "ש\u{05C1}\u{05BC}"] {
        let mut e = he("");
        type_in(&mut e, text);
        assert_eq!(e.text(), text);
    }
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_combining_grapheme_joiner_is_kept_and_takes_no_stop() {
    // CGJ (U+034F) holds two points in an order normal forms would undo,
    // as in Jerusalem's patah and hiriq. It's kept, and the cursor doesn't
    // stop on it.
    let text = "יְרוּשָׁלַ\u{034F}\u{05B4}ם";
    let e = he(text);
    assert_eq!(e.text(), text);
    let ok = boundaries(text);
    let cgj = text.find('\u{034F}').unwrap();
    assert!(!ok.contains(&cgj));
    assert!(!walk_right(&e).contains(&cgj));
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn shin_and_sin_dots_holam_and_qamats_qatan_are_on_their_letters() {
    for text in ["שׁ", "שׂ", "וֹ", "וֺ", "כׇּל", "א\u{05C4}", "א\u{05C5}"] {
        // As many stops as letters, plus the end.
        let e = he(text);
        let letters = text.chars().filter(|c| ('\u{05D0}'..='\u{05EA}').contains(c)).count();
        let mut stops = 1;
        let mut i = 0;
        while i < e.len() {
            i = e.offset_by_clusters(i, 1);
            stops += 1;
        }
        assert_eq!(stops, letters + 1, "{text}");
    }
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_point_with_no_letter_doesnt_break_anything() {
    // A point at the start of a paragraph, or after a space: pasted from a
    // bad text layer, say.
    for text in ["\u{05B8}", "\u{05B8}ב", "a \u{05BC}b", "א\n\u{05B4}ב"] {
        let mut e = he(text);
        let _ = walk_right(&e);
        let end = e.len();
        e.set_selection(std::range::Range { start: end, end });
        while !e.is_empty() {
            e.delete_backward();
        }
    }
}

// MARK: Words

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_pointed_word_is_one_word() {
    let e = he(GEN_1_1);
    let w = words(&e);
    assert_eq!(w.len(), 7, "{w:?}");
    assert_eq!(w[0], "בְּרֵאשִׁ֖ית");
    // The sof pasuq at the end isn't part of the last word.
    assert_eq!(w[6], "הָאָֽרֶץ");
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn words_joined_by_a_maqaf_are_two_words() {
    // Decision: a maqaf joins two words for reading aloud, but each is
    // still a word: a double click takes one, and the maqaf is neither's.
    let e = he("כָּל־הָאָרֶץ");
    assert_eq!(words(&e), ["כָּל", "הָאָרֶץ"]);
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn an_abbreviation_with_gershayim_is_one_word() {
    // With the Hebrew mark, and with the ASCII double quote most people
    // type for it (Unicode's word rules allow both inside a Hebrew word).
    assert_eq!(words(&he("צה״ל")), ["צה״ל"]);
    assert_eq!(words(&he("צה\"ל")), ["צה\"ל"]);
    // And a geresh, in a transliterated name.
    assert_eq!(words(&he("ג׳ורג׳")), ["ג׳ורג׳"]);
    // But a quotation in double quotes is a word in quotes.
    assert_eq!(words(&he("\"שלום\"")), ["שלום"]);
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_paseq_and_sof_pasuq_are_between_words() {
    let e = he("אֵ֥ת ׀ הַשָּׁמַ֖יִם׃");
    assert_eq!(words(&e), ["אֵ֥ת", "הַשָּׁמַ֖יִם"]);
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn sof_pasuq_ends_a_sentence() {
    let e = he("בָּרָא׃ וְהָאָרֶץ הָיְתָה׃");
    let first = e.unit_range_at(0, Unit::Sentence).expect("a sentence");
    assert_eq!(e.text_in(first).trim_end(), "בָּרָא׃");
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_verse_counts_seven_words() {
    // Points and accents don't count as words or split them.
    let e = he(GEN_1_1);
    let mut n = 0;
    let mut i = 0;
    while i < e.len() {
        let next = e.offset_by_units(i, Unit::Word, Edge::End, 1);
        if next <= i {
            break;
        }
        n += 1;
        i = next;
    }
    assert_eq!(n, 7);
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_yiddish_ligature_is_a_letter() {
    let e = he("װאַרשע");
    assert_eq!(words(&e), ["װאַרשע"]);
    assert_eq!(e.offset_by_clusters(0, 1), "װ".len());
}

// MARK: Direction

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_paragraph_that_starts_in_hebrew_runs_right_to_left() {
    let e = he("שלום world");
    // The first letter is at the right.
    assert!(x(&e, 0) > x(&e, at(&e, "world")));
    assert_eq!(e.caret_side().direction, Direction::RightToLeft);
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_paragraph_that_starts_in_english_runs_left_to_right() {
    let e = he("world שלום");
    assert!(x(&e, 0) < x(&e, at(&e, "ש")));
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn digits_or_a_quote_mark_first_dont_decide_the_direction() {
    // The first letter with a direction decides: here, Hebrew.
    for text in ["12 פרקים", "\"שלום\" he said", "(א) the first", "- רשימה"] {
        let e = he(text);
        let first_hebrew = e.text().char_indices().find(|(_, c)| ('\u{05D0}'..='\u{05EA}').contains(c)).unwrap().0;
        let first_latin = e.text().char_indices().find(|(_, c)| c.is_ascii_alphabetic()).map(|(i, _)| i);
        if let Some(latin) = first_latin {
            assert!(x(&e, first_hebrew) > x(&e, latin), "{text}: not right to left");
        }
        assert!(x(&e, 0) >= x(&e, first_hebrew), "{text}: its start isn't at the right");
    }
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn numbers_in_hebrew_read_left_to_right() {
    let e = he("פרק 12, פסוק 345");
    assert!(x(&e, at(&e, "1")) < x(&e, at(&e, "2")));
    assert!(x(&e, at(&e, "3")) < x(&e, at(&e, "4")));
    assert!(x(&e, at(&e, "4")) < x(&e, at(&e, "5")));
    // And the number comes after (left of) "chapter".
    assert!(x(&e, at(&e, "12")) < x(&e, at(&e, "ק")));
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_full_stop_ends_a_hebrew_sentence_on_its_left() {
    let e = he("שלום.");
    let stop = at(&e, ".");
    assert!(x(&e, stop) < x(&e, at(&e, "ם")), "the full stop is on the right of the word");
    // Before English in a Hebrew paragraph, it still belongs to the Hebrew.
    let e = he("שלום. hello");
    let stop = at(&e, ".");
    assert!(x(&e, stop) < x(&e, at(&e, "ם")));
    assert!(x(&e, stop) > x(&e, at(&e, "o")));
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_comma_after_hebrew_in_english_text_stays_english() {
    // In an English paragraph, a comma after a Hebrew word is the English
    // sentence's: it's on the Hebrew's right.
    let e = he("see אבג, then");
    // Before the comma and before the Hebrew's first letter are both the
    // Hebrew's right edge; the letters inside it are left of that.
    let comma = at(&e, ",");
    assert!(x(&e, comma) >= x(&e, at(&e, "א")));
    for h in [at(&e, "ב"), at(&e, "ג")] {
        assert!(x(&e, comma) > x(&e, h));
    }
    // And the comma itself is right of the Hebrew: after it is further right.
    assert!(x(&e, comma + 1) > x(&e, comma));
    assert!(x(&e, comma) < x(&e, at(&e, "then")));
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn an_english_word_sits_between_the_hebrew_words_around_it() {
    let e = he("שלום world טוב");
    let (w, d) = (x(&e, at(&e, "w")), x(&e, at(&e, "d")));
    assert!(w < d, "the English is backwards");
    assert!(x(&e, at(&e, "ש")) > d, "the first Hebrew word isn't on the right");
    assert!(x(&e, at(&e, "ט")) < w, "the last Hebrew word isn't on the left");
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn brackets_face_the_right_way_in_hebrew() {
    // "(שלום)": the opening bracket is at the right, before the word, and
    // the closing one at the left.
    let e = he("(שלום)");
    assert!(x(&e, 0) > x(&e, at(&e, "ש")));
    assert!(x(&e, at(&e, ")")) < x(&e, at(&e, "ם")));
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn an_empty_line_in_hebrew_text_is_hebrew() {
    // Decision: an empty paragraph between Hebrew ones takes their
    // direction (the cursor sits at the right), as Word and Pages do.
    // Unicode's default would make it left to right.
    let e = he("שלום\n\nעולם");
    let empty = at(&e, "\n") + 1;
    assert!(x(&e, empty) > e.width() / 2, "the cursor on the empty line is on the left");
}

// MARK: The cursor

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn the_right_arrow_goes_back_in_hebrew() {
    let e = he("שלום עולם");
    let mid = at(&e, "ו");
    let right = e.place_beside(Place::downstream(mid), true).expect("somewhere to go");
    assert!(right.offset < mid, "right went forward in the text");
    let left = e.place_beside(Place::downstream(mid), false).expect("somewhere to go");
    assert!(left.offset > mid, "left went back in the text");
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn the_arrows_cross_hebrew_and_english_and_come_back() {
    for text in ["say שלום עולם now", "שלום world טוב", "א b ג d ה", "(שלום) 12 [world]"] {
        let e = he(text);
        let there = walk_right(&e);
        for b in boundaries(e.text()) {
            assert!(there.contains(&b), "{text}: never stood at {b}");
        }
        // And back: every step right can be undone by a step left.
        let mut at = Place::downstream(*there.last().unwrap());
        for _ in 0..there.len() * 2 {
            match e.place_beside(at, false) {
                Some(p) if p != at => at = p,
                _ => break,
            }
        }
        let line = &e.lines()[0];
        assert_eq!(at.offset, line.stops[line.visual[0]].offset, "{text}: didn't get back to the left edge");
    }
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn up_and_down_keep_to_the_same_place_in_wrapped_hebrew() {
    let e = wide("אבג דהו זחט יכל מנס עפצ קרש תאב גדה וזח טיכ", 80);
    assert!(e.lines().len() >= 3, "it didn't wrap");
    let start = Place::downstream(at(&e, "ז"));
    let down = e.place_by_lines(start, 1, None).expect("a line below");
    let up = e.place_by_lines(down, -1, None).expect("and back");
    assert!((x(&e, up.offset) - x(&e, start.offset)).abs() < 8 * PT);
}

// MARK: Selection

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn selecting_a_hebrew_word_in_english_is_one_box() {
    let e = he("say שלום now");
    let s = at(&e, "ש");
    let r = e.selection_rects(std::range::Range { start: s, end: s + "שלום".len() });
    assert_eq!(r.len(), 1, "{r:?}");
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_selection_across_a_change_of_direction_covers_what_it_holds() {
    // From "ay" in "say" to the middle of "שלום": on the page, "ay" and the
    // first two Hebrew letters (on the Hebrew's right), which aren't side
    // by side, so two boxes.
    let e = he("say שלום now");
    let start = at(&e, "a");
    let end = at(&e, "ו");
    let r = e.selection_rects(std::range::Range { start, end });
    assert_eq!(r.len(), 2, "{r:?}");
    for b in &r {
        assert!(b.width > 0);
    }
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn deleting_a_selected_pointed_word_leaves_no_stray_points() {
    let mut e = he(GEN_1_1);
    let r = e.unit_range_at(0, Unit::MamreWord).unwrap();
    e.set_selection(r);
    e.delete_backward();
    let first = e.text().chars().next().unwrap();
    assert!(!('\u{0591}'..='\u{05C7}').contains(&first) || first == '\u{05C3}', "a point was left: {:?}", e.text());
}

// MARK: Lines

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_line_never_starts_with_a_point() {
    // A long pointed word in a narrow column breaks inside itself, but only
    // between letters.
    let e = wide("בְּרֵאשִׁ֖יתבָּרָ֣אאֱלֹהִ֑יםאֵ֥תהַשָּׁמַ֖יִם", 40);
    assert!(e.lines().len() > 1, "it didn't break");
    let ok = boundaries(e.text());
    for line in e.lines() {
        assert!(ok.contains(&line.start), "a line starts at {}, inside a letter", line.start);
    }
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_wrapped_hebrew_line_starts_at_the_right() {
    let e = wide("אבג דהו זחט יכל מנס עפצ קרש תאב גדה וזח טיכ", 80);
    for line in &e.lines()[1..] {
        assert!(x(&e, line.start) > e.width() / 2, "a line starts on the left");
    }
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_line_breaks_after_a_maqaf_not_before() {
    let e = wide("אבגדה כָּל־הָאָרֶץ וְכָל־הַשָּׁמַיִם וְכָל־צְבָאָם", 70);
    let maqafs: Vec<usize> = e.text().match_indices('־').map(|(i, _)| i).collect();
    for line in &e.lines()[1..] {
        assert!(!maqafs.contains(&line.start), "a line starts with a maqaf");
    }
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_line_never_starts_with_sof_pasuq_or_a_full_stop() {
    let e = wide("אבג דהו זחט׃ יכל מנס עפצ. קרש תאב גדה׃ וזח טיכ.", 60);
    for line in &e.lines()[1..] {
        let c = e.text()[line.start..].chars().next().unwrap();
        assert!(!matches!(c, '׃' | '.' | ','), "a line starts with {c:?}");
    }
}

// MARK: Typing

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn typing_a_letter_then_its_points_leaves_the_cursor_after_them() {
    let mut e = he("");
    type_in(&mut e, "בְּ");
    assert_eq!(e.text(), "בְּ");
    assert_eq!(e.selection().start, e.len());
    type_in(&mut e, "רֵא");
    assert_eq!(e.text(), "בְּרֵא");
    assert_eq!(e.selection().start, e.len());
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_point_typed_after_a_space_is_kept() {
    // A keyboard can send a point on its own; it's kept, and nothing
    // breaks.
    let mut e = he("שלום ");
    let end = e.len();
    e.set_selection(std::range::Range { start: end, end });
    type_in(&mut e, "\u{05B8}");
    assert_eq!(e.text(), "שלום \u{05B8}");
    let _ = walk_right(&e);
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn hebrew_composed_by_an_input_method_lands_where_it_was_typed() {
    let mut e = he("say  now");
    e.set_selection(std::range::Range { start: 4, end: 4 });
    e.set_marked_text("שָׁ", std::range::Range { start: 0, end: "שָׁ".len() });
    assert_eq!(e.text(), "say שָׁ now");
    e.unmark_text();
    assert_eq!(e.text(), "say שָׁ now");
    assert!(e.marked().is_none());
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn final_letters_are_kept_as_typed() {
    // Hebrew keyboards type final forms themselves: nothing turns a medial
    // letter into a final one or back.
    let mut e = he("");
    type_in(&mut e, "שלומ שלום מים");
    assert_eq!(e.text(), "שלומ שלום מים");
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn presentation_forms_are_kept_as_pasted() {
    // Decision: text pasted from a PDF's text layer often has
    // presentation forms (U+FB1D–U+FB4F: shin with its dot as one
    // character). They're kept as pasted; finding them is search's job,
    // which should treat שׁ and U+FB2A as the same.
    let text = "\u{FB2A}לום \u{FB31}ית";
    let e = he(text);
    assert_eq!(e.text(), text);
    assert_eq!(words(&e).len(), 2);
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn direction_marks_and_controls_pasted_in_dont_trap_the_cursor() {
    // Decision: marks (RLM, LRM, ALM) and embedding, override and isolate
    // controls pasted in are kept. The cursor still reaches everything.
    for text in ["a\u{200F}b", "שלום\u{200E} world", "\u{202B}שלום\u{202C} x", "x \u{2067}שלום\u{2069} y", "\u{202E}abc\u{202C}"] {
        let e = he(text);
        assert_eq!(e.text(), text);
        let seen = walk_right(&e);
        let letters: Vec<usize> = e.text().char_indices().filter(|(_, c)| c.is_alphabetic()).map(|(i, _)| i).collect();
        for l in letters {
            assert!(seen.contains(&l), "{text:?}: never stood at {l}");
        }
    }
}

// MARK: Keeping it

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn utf16_offsets_round_trip_through_pointed_hebrew() {
    // Hebrew is all in the Basic Multilingual Plane (one UTF-16 unit a
    // character); an emoji after it isn't.
    let e = he("בְּרֵאשִׁית 👋 בָּרָא");
    for b in boundaries(e.text()) {
        assert_eq!(e.from_utf16(e.to_utf16(b)), b);
    }
    assert_eq!(e.len_utf16(), e.text().encode_utf16().count());
}

#[test]
#[ignore = "next version: Hebrew edge cases (docs/hebrew-tests.md)"]
fn a_saved_history_brings_back_pointed_hebrew_and_its_english() {
    let (font, fallback) = fonts();
    let mut e = he("");
    type_in(&mut e, "בְּרֵאשִׁית and בָּרָא");
    let history = e.history();
    let text = e.text().to_string();
    let back = Editor::open(&history, "test@here", &text, font, fallback, 17 * PT, 24 * PT, 300 * PT);
    assert_eq!(back.text(), text);
    assert_eq!(
        back.isolates().iter().map(|i| back.text_in(i.range.clone().into()).to_string()).collect::<Vec<_>>(),
        e.isolates().iter().map(|i| e.text_in(i.range.clone().into()).to_string()).collect::<Vec<_>>()
    );
}
