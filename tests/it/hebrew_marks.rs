//! Hebrew marks, drawn: picture snapshots of pointed and cantillated lines,
//! one per font, to show whether each vowel point and accent lands where
//! it belongs on its letter.
//!
//! Next version, like `hebrew.rs`: every test is ignored. Run them with
//!
//!     cargo test --test it hebrew_marks:: -- --ignored
//!
//! Each case is set by Calamus's own [`Editor`] (shaping, bidi, line
//! breaking: what the app draws), then its glyph outlines are filled into a
//! PNG, as `examples/render.rs` draws them into an SVG. The PNG is compared
//! with the reference in `tests/snapshots/hebrew_marks/<case>-<font>.png`.
//! On a mismatch the picture made and a diff (the reference faded, what
//! changed in red) land in `target/hebrew_marks/`, which git ignores.
//!
//! To make or remake the references on purpose:
//!
//!     UPDATE_SNAPSHOTS=1 cargo test --test it hebrew_marks:: -- --ignored
//!
//! A test passing says the picture hasn't changed, not that it's right:
//! whether a mark is where a reader of Hebrew wants it is for a person to
//! judge by eye, once, on the references (docs/hebrew-tests.md).
//!
//! The text is from the Unicode/XML Leningrad Codex (UXLC 2.6), its
//! spelling and mark order kept exactly, each word's verse in a comment.
//! Glyph 0 (a font's missing glyph) is drawn red, so a mark a font hasn't
//! got stands out; there's no fallback font, so nothing hides one.

use std::path::PathBuf;
use std::sync::Arc;

use calamus::Editor;
use calamus::layout::{Font, PT};
use rustybuzz::ttf_parser::{Face, GlyphId, OutlineBuilder};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

/// The fonts each case is drawn in, all SIL Open Font License, in `fonts/`.
const FONTS: &[(&str, &[u8])] = &[
    ("ezra-sil", include_bytes!("../../fonts/SILEOT.ttf")),
    ("ezra-sil-sr", include_bytes!("../../fonts/SILEOTSR.ttf")),
    ("noto-serif-hebrew", include_bytes!("../../fonts/NotoSerifHebrew-Regular.ttf")),
    ("noto-sans-hebrew", include_bytes!("../../fonts/NotoSansHebrew-Regular.ttf")),
];

/// Text size, in points.
const SIZE: f64 = 28.0;
/// Line height, in points.
const LEADING: f64 = 56.0;
/// The width text is set in, in points.
const WIDTH: f64 = 460.0;
/// Pixels per point: 2 is 144 dpi, enough to see a dot's place.
const SCALE: f64 = 2.0;
/// Blank pixels around the text.
const MARGIN: f64 = 16.0;

/// A channel that moved by more than this counts as a changed pixel; less
/// is antialiasing noise.
const CHANNEL_TOLERANCE: u8 = 64;
/// How many changed pixels are let pass. A point moved by a pixel changes
/// some dozens, so this is under any real move of a mark.
const PIXEL_TOLERANCE: usize = 12;

// The cases. Each says what it's there to catch.

/// Ruth 1:1, the whole verse, pointed and cantillated: an ordinary verse,
/// wrapped over lines. It has dagesh with vowels below (בִּימֵי֙), shin with
/// dagesh, holam and shin dot (הַשֹּׁפְטִ֔ים), two pashtas, postpositive, at a
/// word's left end (בִּימֵי֙, לָגוּר֙), a meteg (בָנָֽיו׃), holam male (מוֹאָ֔ב),
/// sin dot (בִּשְׂדֵ֣י), final kaf with shewa (וַיֵּ֨לֶךְ), and a sof pasuq.
const RUTH_1_1: &str = "וַיְהִ֗י בִּימֵי֙ שְׁפֹ֣ט הַשֹּׁפְטִ֔ים וַיְהִ֥י רָעָ֖ב בָּאָ֑רֶץ וַיֵּ֨לֶךְ אִ֜ישׁ מִבֵּ֧ית לֶ֣חֶם יְהוּדָ֗ה לָגוּר֙ בִּשְׂדֵ֣י מוֹאָ֔ב ה֥וּא וְאִשְׁתּ֖וֹ וּשְׁנֵ֥י בָנָֽיו׃";

/// Dagesh inside a letter that has a vowel below it: the dagesh must sit
/// in the letter's body, the vowel under it, neither on the other.
/// Genesis 1:1 (בְּרֵאשִׁ֖ית: bet, dagesh, shewa) and (הַשָּׁמַ֖יִם: shin, shin
/// dot, dagesh, qamats), and Amos 2:5 (בִּֽיהוּדָ֑ה: bet, dagesh, hiriq and
/// meteg; shuruq, the vav's dagesh, in the same word).
const DAGESH: &str = "בְּרֵאשִׁ֖ית הַשָּׁמַ֖יִם בִּֽיהוּדָ֑ה";

/// Shin and sin dots with holam. The holam of the letter before a shin
/// meets the shin's dot (מֹשֶׁ֖ה, 1 Chronicles 6:34); a sin's own dot meets a
/// holam on the sin (שֹׂנְאֶ֔יךָ, 2 Chronicles 1:11); and a sin with dagesh and
/// holam (וּמַשֹּׂ֥א, 2 Chronicles 19:7). The dots shouldn't collide or merge
/// into one where two are written.
const SHIN_SIN_HOLAM: &str = "מֹשֶׁ֖ה שֹׂנְאֶ֔יךָ וּמַשֹּׂ֥א";

/// Meteg beside a vowel, not on it: beside hiriq (בִּֽיהוּדָ֑ה, Amos 2:5) and
/// beside hataf patah (הֲֽתִבְטַח־, Job 39:11), where the meteg goes left of
/// the hataf's two parts.
const METEG: &str = "בִּֽיהוּדָ֑ה הֲֽתִבְטַח־";

/// Accents stacked on one letter, above and below: merkha and qadma on
/// the he of אֱלֹהִ֥֨ים, tevir and telisha qetana on the final kaf of לְךָ֛֩
/// (both Deuteronomy 5:7, the double cantillation of the Decalogue), and
/// two accents above on one nun, zaqef qatan and pazer (תְּמוּנָ֔֡ה,
/// Deuteronomy 5:8). They should sit side by side, not on each other or on
/// the vowel.
const STACKED: &str = "אֱלֹהִ֥֨ים לְךָ֛֩ תְּמוּנָ֔֡ה";

/// Prepositive accents, drawn at the word's first letter (its right edge)
/// whatever letter they're typed after: telisha gedola (לָ֠כֵן, Amos 5:11),
/// yetiv (כֹּ֚ה, Amos 1:3) and dehi (י֭וֹם, Job 3:3).
const PREPOSITIVE: &str = "לָ֠כֵן כֹּ֚ה י֭וֹם";

/// Postpositive accents, drawn at the word's last letter (its left edge):
/// pashta (שֹׂכְרִים֙, 2 Chronicles 24:12) and telisha qetana (וְשֵׁם֩, Ruth
/// 1:2).
const POSTPOSITIVE: &str = "שֹׂכְרִים֙ וְשֵׁם֩";

/// Final kaf with qamats: the qamats goes inside the kaf's bowl, not
/// under its descender (אֱלֹהֶ֖יךָ, Amos 4:12; לְךָ֛֩, Deuteronomy 5:7, with
/// accents too).
const FINAL_KAF_QAMATS: &str = "אֱלֹהֶ֖יךָ לְךָ֛֩";

/// Holam haser on a vav (U+05BA), a consonantal vav with the holam of the
/// letter before: the dot should sit left above the vav, not over it as a
/// holam male would (מִצְוֺתַ֥י, 1 Chronicles 28:7; עֲוֺנֹֽתֵיכֶֽם׃, Amos 3:2).
/// מוֹאָ֔ב (Ruth 1:1) has a holam male, for comparison.
const HOLAM_HASER: &str = "מִצְוֺתַ֥י עֲוֺנֹֽתֵיכֶֽם׃ מוֹאָ֔ב";

/// A combining grapheme joiner keeping two marks on one letter in the
/// written order: Jerusalem's lamed with qamats (or patah) and an accent or
/// meteg, then the hiriq of the yod read but not written (יְרוּשָׁלָֽ\u{34F}ִם׃, Amos
/// 2:5; וּמִירוּשָׁלַ֖\u{34F}ִם, Amos 1:2). Both marks should show, the CGJ
/// shouldn't draw, and the hiriq should still be under the lamed.
const CGJ: &str = "יְרוּשָׁלָֽ\u{34F}ִם׃ וּמִירוּשָׁלַ֖\u{34F}ִם";

/// An outline into a tiny-skia path, in font units, y up.
struct Outline(PathBuilder);

impl OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

/// `text` set in `bytes` alone and drawn: black glyphs on white, a grey
/// rule on each baseline, and the font's missing glyph in red.
fn draw(text: &str, bytes: &[u8]) -> Pixmap {
    let sp = |points: f64| (points * PT as f64) as i64;
    let px = |sp: i64| sp as f64 / PT as f64 * SCALE;
    let font = Arc::new(Font::from_bytes(bytes.to_vec()).unwrap());
    let editor = Editor::new(text, font, Vec::new(), sp(SIZE), sp(LEADING), sp(WIDTH));
    let face = Face::parse(bytes, 0).unwrap();

    let width = (WIDTH * SCALE + 2.0 * MARGIN).ceil() as u32;
    let height = (px(editor.height()) + 2.0 * MARGIN).ceil() as u32;
    let mut pixmap = Pixmap::new(width, height).unwrap();
    pixmap.fill(Color::WHITE);

    let mut rule = Paint::default();
    rule.set_color_rgba8(200, 200, 200, 255);
    let mut ink = Paint::default();
    ink.set_color_rgba8(0, 0, 0, 255);
    let mut missing = Paint::default();
    missing.set_color_rgba8(220, 0, 0, 255);

    let em = SIZE * SCALE / f64::from(face.units_per_em());
    for line in editor.lines() {
        let baseline = MARGIN + px(line.baseline);
        let mut path = PathBuilder::new();
        path.move_to(MARGIN as f32, baseline as f32);
        path.line_to((MARGIN + WIDTH * SCALE) as f32, baseline as f32);
        let stroke = Stroke { width: 1.0, ..Stroke::default() };
        pixmap.stroke_path(&path.finish().unwrap(), &rule, &stroke, Transform::identity(), None);

        for g in &line.glyphs {
            let mut outline = Outline(PathBuilder::new());
            if face.outline_glyph(GlyphId(g.id), &mut outline).is_none() {
                continue; // a space, or a mark the font draws as nothing
            }
            let Some(path) = outline.0.finish() else { continue };
            let at = Transform::from_row(
                em as f32,
                0.0,
                0.0,
                -em as f32,
                (MARGIN + px(g.x)) as f32,
                (MARGIN + px(line.baseline + g.y)) as f32,
            );
            let paint = if g.id == 0 { &missing } else { &ink };
            pixmap.fill_path(&path, paint, FillRule::Winding, at, None);
        }
    }
    pixmap
}

fn snapshots() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/hebrew_marks")
}

fn failures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/hebrew_marks")
}

/// How many pixels of `a` and `b` differ by more than the tolerance, and a
/// picture of where: `b` faded, the changed pixels red.
fn compare(a: &Pixmap, b: &Pixmap) -> (usize, Pixmap) {
    let mut diff = b.clone();
    let mut changed = 0;
    for (i, (p, q)) in a.pixels().iter().zip(b.pixels()).enumerate() {
        let moved = [
            p.red().abs_diff(q.red()),
            p.green().abs_diff(q.green()),
            p.blue().abs_diff(q.blue()),
            p.alpha().abs_diff(q.alpha()),
        ];
        let out = &mut diff.pixels_mut()[i];
        if moved.iter().any(|&m| m > CHANNEL_TOLERANCE) {
            changed += 1;
            *out = tiny_skia::PremultipliedColorU8::from_rgba(255, 0, 0, 255).unwrap();
        } else {
            let fade = |c: u8| 255 - (255 - c) / 4;
            *out = tiny_skia::PremultipliedColorU8::from_rgba(fade(q.red()), fade(q.green()), fade(q.blue()), 255)
                .unwrap();
        }
    }
    (changed, diff)
}

/// Draws `text` in every font and checks each picture against its
/// reference, or writes the references with `UPDATE_SNAPSHOTS=1`. Every
/// font is drawn before failing, so one run shows every mismatch.
#[track_caller]
fn snapshot(case: &str, text: &str) {
    let update = std::env::var_os("UPDATE_SNAPSHOTS").is_some_and(|v| v != "0" && !v.is_empty());
    let mut wrong = Vec::new();
    for &(font, bytes) in FONTS {
        let name = format!("{case}-{font}.png");
        let made = draw(text, bytes);
        let reference = snapshots().join(&name);
        if update {
            std::fs::create_dir_all(snapshots()).unwrap();
            made.save_png(&reference).unwrap();
            continue;
        }
        let problem = match Pixmap::load_png(&reference) {
            Err(e) => Some(format!("{name}: no reference ({e}); make it with UPDATE_SNAPSHOTS=1")),
            Ok(old) if (old.width(), old.height()) != (made.width(), made.height()) => {
                std::fs::create_dir_all(failures()).unwrap();
                made.save_png(failures().join(format!("{case}-{font}.actual.png"))).unwrap();
                Some(format!(
                    "{name}: {}x{} now, {}x{} in the reference",
                    made.width(),
                    made.height(),
                    old.width(),
                    old.height()
                ))
            }
            Ok(old) => {
                let (changed, diff) = compare(&made, &old);
                (changed > PIXEL_TOLERANCE).then(|| {
                    std::fs::create_dir_all(failures()).unwrap();
                    made.save_png(failures().join(format!("{case}-{font}.actual.png"))).unwrap();
                    diff.save_png(failures().join(format!("{case}-{font}.diff.png"))).unwrap();
                    format!("{name}: {changed} pixels changed")
                })
            }
        };
        wrong.extend(problem);
    }
    assert!(
        wrong.is_empty(),
        "pictures differ from tests/snapshots/hebrew_marks (what they are now is in target/hebrew_marks):\n  {}",
        wrong.join("\n  ")
    );
}

#[test]
#[ignore = "next version: Hebrew picture snapshots"]
fn ruth_1_1() {
    snapshot("ruth_1_1", RUTH_1_1);
}

#[test]
#[ignore = "next version: Hebrew picture snapshots"]
fn dagesh_in_a_letter_with_a_vowel_below() {
    snapshot("dagesh", DAGESH);
}

#[test]
#[ignore = "next version: Hebrew picture snapshots"]
fn shin_and_sin_dots_with_holam() {
    snapshot("shin_sin_holam", SHIN_SIN_HOLAM);
}

#[test]
#[ignore = "next version: Hebrew picture snapshots"]
fn meteg_beside_a_vowel() {
    snapshot("meteg", METEG);
}

#[test]
#[ignore = "next version: Hebrew picture snapshots"]
fn accents_stacked_on_one_letter() {
    snapshot("stacked", STACKED);
}

#[test]
#[ignore = "next version: Hebrew picture snapshots"]
fn prepositive_accents() {
    snapshot("prepositive", PREPOSITIVE);
}

#[test]
#[ignore = "next version: Hebrew picture snapshots"]
fn postpositive_accents() {
    snapshot("postpositive", POSTPOSITIVE);
}

#[test]
#[ignore = "next version: Hebrew picture snapshots"]
fn final_kaf_with_qamats() {
    snapshot("final_kaf_qamats", FINAL_KAF_QAMATS);
}

#[test]
#[ignore = "next version: Hebrew picture snapshots"]
fn holam_haser_on_vav() {
    snapshot("holam_haser", HOLAM_HASER);
}

#[test]
#[ignore = "next version: Hebrew picture snapshots"]
fn combining_grapheme_joiner() {
    snapshot("cgj", CGJ);
}
