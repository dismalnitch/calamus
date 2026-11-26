//! Draws text as Calamus sets it, to an SVG: the glyphs, and under each
//! line a tick for every caret stop. For looking at layout without an app.
//!
//!     cargo run --example render -- out.svg "abc אבג def" "אבג abc"
//!
//! Each argument after the file is a paragraph. The fonts are the ones in
//! `fonts/`: Inter, falling back to Noto Sans Hebrew.

use std::fmt::Write;
use std::sync::Arc;

use calamus::Editor;
use calamus::layout::{Font, PT};
use rustybuzz::ttf_parser::{Face, GlyphId, OutlineBuilder};

/// An outline as SVG path data, in font units, y up.
struct Path(String);

impl OutlineBuilder for Path {
    fn move_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.0, "M{x} {y}");
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.0, "L{x} {y}");
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let _ = write!(self.0, "Q{x1} {y1} {x} {y}");
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let _ = write!(self.0, "C{x1} {y1} {x2} {y2} {x} {y}");
    }
    fn close(&mut self) {
        self.0.push('Z');
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().expect("a file to write, then the text");
    let text: Vec<String> = args.collect();
    let text = text.join("\n");
    let files: [&[u8]; 2] = [
        include_bytes!("../fonts/Inter-Regular.ttf"),
        include_bytes!("../fonts/NotoSansHebrew-Regular.ttf"),
    ];
    let faces: Vec<Face> = files.iter().map(|f| Face::parse(f, 0).unwrap()).collect();
    let fonts: Vec<Arc<Font>> = files
        .iter()
        .map(|f| Arc::new(Font::from_bytes(f.to_vec()).unwrap()))
        .collect();
    let (size, width) = (28.0, 520.0);
    let pt = |sp: i64| sp as f64 / PT as f64;
    let sp = |points: f64| (points * PT as f64) as i64;
    let editor = Editor::new(
        &text,
        fonts[0].clone(),
        fonts[1..].to_vec(),
        sp(size),
        sp(size * 1.9),
        sp(width),
    );

    let (margin, height) = (20.0, pt(editor.height()));
    let mut svg = String::new();
    let _ = write!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}"><rect width="100%" height="100%" fill="white"/><g transform="translate({margin} {margin})">"#,
        w = width + 2.0 * margin,
        h = height + 2.0 * margin,
    );
    // The edges text is set between.
    let _ = write!(
        svg,
        r##"<rect width="{width}" height="{height}" fill="none" stroke="#ddd"/>"##
    );
    for isolate in editor.isolates() {
        for r in editor.selection_rects(std::range::Range {
            start: isolate.range.start,
            end: isolate.range.end,
        }) {
            let _ = write!(
                svg,
                r##"<rect x="{}" y="{}" width="{}" height="{}" fill="#fde9b8"/>"##,
                pt(r.x),
                pt(r.y) + 4.0,
                pt(r.width),
                pt(r.height) - 16.0
            );
        }
    }
    for line in editor.lines() {
        for g in &line.glyphs {
            let face = &faces[g.font];
            let scale = size / f64::from(face.units_per_em());
            let mut path = Path(String::new());
            if face.outline_glyph(GlyphId(g.id), &mut path).is_none() {
                continue;
            }
            let _ = write!(
                svg,
                r#"<path transform="translate({} {}) scale({scale} -{scale})" d="{}"/>"#,
                pt(g.x),
                pt(line.baseline + g.y),
                path.0
            );
        }
        // The stops, from the left, numbered by their place in the text.
        let bottom = pt(line.top + line.height);
        for (k, &s) in line.visual.iter().enumerate() {
            let stop = line.stops[s];
            let x = pt(stop.x);
            let lift = if k % 2 == 0 { 0.0 } else { 9.0 };
            let _ = write!(
                svg,
                r##"<line x1="{x}" x2="{x}" y1="{}" y2="{}" stroke="#c33" stroke-width="0.7"/><text x="{x}" y="{}" font-size="8" fill="#c33" text-anchor="middle" font-family="monospace">{}</text>"##,
                bottom - 22.0,
                bottom - 14.0,
                bottom - 5.0 + lift - 9.0,
                stop.offset - line.start
            );
        }
    }
    svg.push_str("</g></svg>");
    std::fs::write(&out, svg).expect("the file can be written");
}
