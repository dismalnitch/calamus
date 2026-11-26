//! The typesetter in the browser: the page hands over a font and a text,
//! and gets back the lines to show, set again whenever the text or the
//! width changes.

use calamus::html::{Page, Set};
use calamus::layout::{Align, Font, PT, Run, Style, TextStyle, set_paragraph};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Typesetter {
    font: Font,
}

#[wasm_bindgen]
impl Typesetter {
    /// A typesetter with one font, from the font file's bytes.
    #[wasm_bindgen(constructor)]
    pub fn new(font: Vec<u8>) -> Result<Typesetter, JsError> {
        Ok(Typesetter { font: Font::from_bytes(font)? })
    }

    /// Set `text`, a paragraph to each line of it, `width` points wide at
    /// `size` on `line_height` points, and give back the lines as HTML.
    /// The font's family in it is `f0`. With `widths`, each line says how
    /// wide it was set, for checking the browser's lines against.
    pub fn set(&self, text: &str, width: f64, size: f64, line_height: f64, justified: bool, widths: bool) -> String {
        let sp = |points: f64| (points * PT as f64).round() as i64;
        let width = sp(width).max(1);
        let text_style = TextStyle {
            font: &self.font,
            fallback: &[],
            language: None,
            size: sp(size),
            line_height: sp(line_height),
        };
        let style = Style { strut: text_style, align: if justified { Align::Justified } else { Align::Ragged } };

        let texts: Vec<&str> = text.split('\n').collect();
        let runs: Vec<[Run; 1]> = texts.iter().map(|t| [Run { len: t.len(), style: text_style }]).collect();
        let paragraphs: Vec<_> =
            texts.iter().zip(&runs).map(|(t, runs)| set_paragraph(t, runs, &style, &[width])).collect();
        let sets: Vec<Set> = texts
            .iter()
            .zip(&runs)
            .zip(&paragraphs)
            .map(|((text, runs), paragraph)| Set { text, runs, paragraph })
            .collect();
        Page { title: "", width, margin: 0, gap: sp(line_height) / 2, widths, paragraphs: &sets }.body()
    }
}
