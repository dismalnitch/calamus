//! Set a passage ragged and justified, and write it out as a web page.
//!
//!     cargo run --example dothtml -- out.html [width in points]

use calamus::html::{Page, Set};
use calamus::layout::{Align, Font, PT, Run, Style, TextStyle, set_paragraph};

const TEXT: &str = "I heard from a neighbouring house a voice, as of boy or girl, I know not, chanting, and oft repeating, “Take up and read; Take up and read.” Instantly, my countenance altered, I began to think most intently whether children were wont in any kind of play to sing such words: nor could I remember ever to have heard the like.";

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("where to write the page");
    let width = args.next().map_or(240, |w| w.parse::<i64>().expect("a width in points")) * PT;

    let regular = Font::from_bytes(include_bytes!("../fonts/Inter-Regular.ttf").to_vec()).unwrap();
    let bold = Font::from_bytes(include_bytes!("../fonts/Inter-Bold.ttf").to_vec()).unwrap();
    let text = TextStyle { font: &regular, fallback: &[], language: Some("en"), size: 12 * PT, line_height: 16 * PT };
    let loud = TextStyle { font: &bold, size: 18 * PT, line_height: 24 * PT, ..text };

    // “Take up and read; Take up and read.” in bold, and bigger.
    let from = TEXT.find('“').unwrap();
    let to = TEXT.find('”').unwrap() + '”'.len_utf8();
    let plain = [Run { len: TEXT.len(), style: text }];
    let mixed = [
        Run { len: from, style: text },
        Run { len: to - from, style: loud },
        Run { len: TEXT.len() - to, style: text },
    ];

    let ragged = set_paragraph(TEXT, &plain, &Style { strut: text, align: Align::Ragged }, &[width]);
    let justified = set_paragraph(TEXT, &plain, &Style { strut: text, align: Align::Justified }, &[width]);
    let runs = set_paragraph(TEXT, &mixed, &Style { strut: text, align: Align::Justified }, &[width]);

    let page = Page {
        title: "Tolle lege",
        width,
        margin: 36 * PT,
        gap: 16 * PT,
        widths: false,
        paragraphs: &[
            Set { text: TEXT, runs: &plain, paragraph: &ragged },
            Set { text: TEXT, runs: &plain, paragraph: &justified },
            Set { text: TEXT, runs: &mixed, paragraph: &runs },
        ],
    };
    std::fs::write(&path, page.to_html()).expect("write the page");
}
