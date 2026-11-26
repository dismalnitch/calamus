//! html
//!
//! A set paragraph as a web page that looks the way it was set: the same
//! lines, broken in the same places, every word where the typesetter put
//! it.
//!
//! The browser isn't asked to break lines or to justify them. Each line
//! is a box of its own, as tall as the typesetter made it, and its text
//! flows in it as ordinary inline text, in the same font, which comes
//! along in the page. Only the spaces are ours: each is a box exactly as
//! wide as the typesetter made it, stretched or shrunk. So the browser
//! sets each word, and a difference between its shaper and ours is a
//! word's width, which adds up along one line at most and never moves a
//! line break.
//!
//! The text is ordinary text to the browser: it can be selected, copied,
//! searched and read aloud, a paragraph coming out as one line of text.
//!
//! Lengths are written in CSS points, a scaled point being 1/65536 of one.
//!
//! What's rough:
//! - Each font is in the page whole, as base64. A page of any length wants
//!   them subsetted, or linked.
//! - A cluster no font has is our missing glyph, but the browser will look
//!   for it in the system's fonts.
//! - Face 0 of a collection only.

use std::fmt::Write;

use base64::Engine;
use unicode_linebreak::linebreaks;

use crate::layout::{Font, Glyph, Line, PT, Paragraph, Run, is_line_end, is_space};

/// A paragraph as set, with what it was set from.
#[derive(Debug, Clone, Copy)]
pub struct Set<'a> {
    pub text: &'a str,
    pub runs: &'a [Run<'a>],
    pub paragraph: &'a Paragraph,
}

/// Paragraphs one under another, to be written out as a web page.
#[derive(Debug, Clone, Copy)]
pub struct Page<'a> {
    pub title: &'a str,
    /// How wide the paragraphs were set, in scaled points.
    pub width: i64,
    /// The room around the text, in scaled points.
    pub margin: i64,
    /// The room between one paragraph and the next, in scaled points.
    pub gap: i64,
    /// Whether each line says how wide the typesetter made it
    /// (`data-width`, in points), for checking a browser's lines against.
    pub widths: bool,
    pub paragraphs: &'a [Set<'a>],
}

impl Page<'_> {
    /// The whole page: a document that needs nothing else.
    pub fn to_html(&self) -> String {
        let fonts = Fonts::of(self.paragraphs);
        let mut out = String::new();
        out.push_str("<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n");
        out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
        let _ = writeln!(out, "<title>{}</title>", escape(self.title));
        out.push_str("<style>\n");
        fonts.write_faces(&mut out);
        out.push_str(STYLE);
        out.push_str("</style>\n</head>\n<body>\n");
        let _ = writeln!(out, "<main style=\"margin:{}\">", pt(self.margin));
        self.write_body(&fonts, &mut out);
        out.push_str("\n</main>\n</body>\n</html>\n");
        out
    }

    /// Only the text: the `<div>` holding the lines, for a page that
    /// already has the fonts and the [`STYLE`] rules. The fonts' families
    /// there are `f0`, `f1` and so on, in the order the runs first use
    /// them, each run's own font before its fallbacks.
    pub fn body(&self) -> String {
        let fonts = Fonts::of(self.paragraphs);
        let mut out = String::new();
        self.write_body(&fonts, &mut out);
        out
    }

    fn write_body(&self, fonts: &Fonts, out: &mut String) {
        let height: i64 = self.paragraphs.iter().map(|p| p.paragraph.height).sum::<i64>()
            + self.gap * (self.paragraphs.len().saturating_sub(1)) as i64;
        let _ = write!(out, "<div class=\"set\" style=\"width:{};height:{}\">", pt(self.width), pt(height));
        for (k, set) in self.paragraphs.iter().enumerate() {
            // Paragraphs are boxes in a row too, each as wide as the
            // text, and not `<p>`: copying puts a line end of the
            // browser's own after a `<p>` in some browsers (Firefox) and
            // not in others, and we put in our own.
            if k + 1 < self.paragraphs.len() && self.gap != 0 {
                let _ = write!(out, "<span role=\"paragraph\" style=\"margin-bottom:{}\">", pt(self.gap));
            } else {
                out.push_str("<span role=\"paragraph\">");
            }
            let cuts = cuts(set);
            for (i, line) in set.paragraph.lines.iter().enumerate() {
                // A line end after every paragraph but the last, for the
                // text to be copied with: the lines' boxes are as tall as
                // they're told, so it moves nothing.
                let ends_paragraph = i + 1 == set.paragraph.lines.len() && k + 1 < self.paragraphs.len();
                write_line(set, line, &cuts, fonts, ends_paragraph, self.widths, out);
            }
            out.push_str("</span>");
        }
        out.push_str("</div>");
    }
}

/// The rules the lines need, whatever the fonts.
pub const STYLE: &str = "\
html{-webkit-text-size-adjust:none;text-size-adjust:none}
body{margin:0}
.set{font-size:0;line-height:0}
.set>span{display:inline-block;width:100%;vertical-align:top}
.set>span>span{display:inline-block;width:100%;vertical-align:top;white-space:pre;
font-kerning:normal;font-variant-ligatures:normal;font-synthesis:none;
font-optical-sizing:none;letter-spacing:0;word-spacing:0}
.set i{display:inline-block;width:0}
.set s{display:inline-block;white-space:pre;text-decoration:none}
";

/// The fonts a page's paragraphs are set in, each once.
struct Fonts<'a>(Vec<&'a Font>);

impl<'a> Fonts<'a> {
    fn of(paragraphs: &[Set<'a>]) -> Self {
        let mut fonts: Vec<&Font> = Vec::new();
        for set in paragraphs {
            for run in set.runs {
                for font in run.style.fonts() {
                    if !fonts.iter().any(|f| std::ptr::eq(*f, font)) {
                        fonts.push(font);
                    }
                }
            }
        }
        Fonts(fonts)
    }

    /// The number in `font`'s family name.
    fn index(&self, font: &Font) -> usize {
        self.0.iter().position(|f| std::ptr::eq(*f, font)).expect("every run's fonts were gathered")
    }

    /// An `@font-face` for each.
    fn write_faces(&self, out: &mut String) {
        for (i, font) in self.0.iter().enumerate() {
            let _ = write!(out, "@font-face{{font-family:f{i};src:url(data:font/ttf;base64,");
            base64::engine::general_purpose::STANDARD.encode_string(font.data(), out);
            out.push_str(")}\n");
        }
    }
}

/// Where one stretch of text shaped as a whole ends and the next begins:
/// every place a line may break, and every change of run. Byte offsets,
/// in order.
fn cuts(set: &Set) -> Vec<usize> {
    let mut cuts: Vec<usize> = linebreaks(set.text).map(|(at, _)| at).collect();
    let mut end = 0;
    for run in set.runs {
        end += run.len;
        cuts.push(end);
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts
}

/// A stretch of a line's glyphs that the browser can shape as we did: one
/// run, one font, no break inside, all spaces or none.
struct Stretch {
    /// The first byte of its text.
    start: usize,
    /// The first glyph's place on the line.
    x: i64,
    run: usize,
    font: usize,
    space: bool,
}

fn stretches(set: &Set, line: &Line, cuts: &[usize]) -> Vec<Stretch> {
    let is_space_at = |g: &Glyph| set.text[g.cluster..].chars().next().is_some_and(is_space);
    let mut out: Vec<Stretch> = Vec::new();
    let mut last: Option<&Glyph> = None;
    for g in &line.glyphs {
        let space = is_space_at(g);
        let new = match (last, out.last()) {
            (Some(prev), Some(s)) => {
                g.run != s.run
                    || g.font != s.font
                    || space != s.space
                    // A cut between the glyph before and this one.
                    || cuts[cuts.partition_point(|&c| c <= prev.cluster)..].first().is_some_and(|&c| c <= g.cluster)
            }
            _ => true,
        };
        if new {
            out.push(Stretch { start: g.cluster, x: g.x, run: g.run, font: g.font, space });
        }
        last = Some(g);
    }
    out
}

fn write_line(
    set: &Set,
    line: &Line,
    cuts: &[usize],
    fonts: &Fonts,
    ends_paragraph: bool,
    widths: bool,
    out: &mut String,
) {
    // The line's box, and a strut as tall as the line reaches above its
    // baseline. The text asks for no height of its own (a line height of
    // nothing), so the strut alone says where the baseline is, whatever
    // the browser takes the font's ascent to be.
    out.push_str("<span");
    if widths {
        let _ = write!(out, " data-width=\"{}\"", number(line.width as f64 / PT as f64));
    }
    let _ = write!(
        out,
        " style=\"height:{}\"><i style=\"height:{}\"></i>",
        pt(line.height),
        pt(line.baseline - line.top)
    );
    let stretches = stretches(set, line, cuts);
    if stretches.is_empty() {
        // Nothing to draw, but the line's spaces are still its text.
        write_spaces(&set.text[line.text.clone()], 0, out);
    }
    // The run and font of the span that's open.
    let mut open: Option<(usize, usize)> = None;
    for (i, s) in stretches.iter().enumerate() {
        let next = stretches.get(i + 1);
        let text = &set.text[s.start..next.map_or(line.text.end, |next| next.start)];
        if open != Some((s.run, s.font)) {
            if open.is_some() {
                out.push_str("</span>");
            }
            let style = &set.runs[s.run].style;
            let _ = write!(
                out,
                "<span style=\"font:{}/0 f{}\"",
                pt(style.size),
                fonts.index(style.font_at(s.font)),
            );
            if let Some(language) = style.language {
                let _ = write!(out, " lang=\"{}\"", escape(language));
            }
            out.push('>');
            open = Some((s.run, s.font));
        }
        if s.space {
            // As wide as the typesetter made it: to the next word.
            write_spaces(text, next.map_or(line.width, |next| next.x) - s.x, out);
            continue;
        }
        // The last stretch's text runs on to the spaces and line end the
        // line broke at, which take no room.
        let content = text.trim_end_matches(|c| is_space(c) || is_line_end(c));
        out.push_str(&escape(content));
        write_spaces(&text[content.len()..], 0, out);
    }
    if open.is_some() {
        out.push_str("</span>");
    }
    if ends_paragraph {
        out.push('\n');
    }
    out.push_str("</span>");
}

/// Spaces as text, in a box `width` wide: the browser sets what's between
/// the spaces and we say how far apart that is. Line ends are left out.
fn write_spaces(text: &str, width: i64, out: &mut String) {
    let spaces: String = text.chars().filter(|&c| !is_line_end(c)).collect();
    if !spaces.is_empty() {
        let _ = write!(out, "<s style=\"width:{}\">{spaces}</s>", pt(width));
    }
}

/// Scaled points as a CSS length in points.
fn pt(sp: i64) -> String {
    let mut s = number(sp as f64 / PT as f64);
    s.push_str("pt");
    s
}

/// A number to five places, without the zeros it doesn't need.
fn number(n: f64) -> String {
    let s = format!("{n:.5}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".to_string() } else { s.to_string() }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{number, pt};
    use crate::layout::PT;

    #[test]
    fn lengths_are_points() {
        assert_eq!(pt(12 * PT), "12pt");
        assert_eq!(pt(0), "0pt");
        assert_eq!(pt(PT / 2), "0.5pt");
        assert_eq!(pt(-PT / 4), "-0.25pt");
        assert_eq!(number(-0.000001), "0");
    }
}
