//! Caret stops: every place on a line where a text cursor can stand (tolege#236,
//! the first milestone of editing on Idem, tolege#216).
//!
//! # Why this exists
//!
//! A note editor needs a cursor, a selection and taps that land in the
//! right place. `UITextView` can't be that editor: it lays text out itself,
//! so its cursor would sit where *Apple* broke the line, not where Idem did.
//! So tolege has its own text view. UIKit still draws the cursor, the
//! selection highlight, the handles and the magnifier (`UITextInteraction`),
//! but it asks our view where things are:
//!
//! - where is the cursor at this place in the text? (a rectangle)
//! - what place in the text is under this touch?
//! - which rectangles cover this selection?
//! - what place is one line up, or down, from here?
//!
//! UIKit asks those constantly, many times per keystroke. If each one
//! crossed the bridge into Rust, the bridge would stop being "calculate
//! this" and become a chatty, copy-heavy conversation with UIKit (decided
//! against, 2026-11-12). So the rule is **Rust computes, Swift looks
//! things up**: each edit makes one call, re-setting the paragraph that
//! changed, and with each line Idem hands back a small table of **caret
//! stops**. Swift answers all four questions from that table with plain
//! lookups and binary searches.
//!
//! # What a caret stop is
//!
//! A place the cursor can stand (a byte offset into the paragraph's text)
//! and how far along the line it is (`x`, in scaled points from the line's
//! start, like [`Glyph::x`](crate::layout::Glyph::x)). A line's stops go in text order, from the
//! line's first character to just past its last.
//!
//! For "hello" set with one glyph per letter, the stops are at offsets
//! 0, 1, 2, 3, 4, 5: before each letter and after the last. Their x is
//! each glyph's x, and for the last stop, the last glyph's x plus its
//! width. That much is easy. The rest of this comment is why the whole
//! job isn't.
//!
//! # What makes it hard
//!
//! **Glyphs have no widths yet.** [`Glyph`](crate::layout::Glyph) has an `x` but not an advance,
//! so the right edge of a line's last glyph is unknown. The shaper knows
//! each advance (`Shaped::advance` in lib.rs); it's dropped when glyphs are
//! placed. A stop after the last glyph needs it.
//!
//! **Characters aren't what the cursor steps over; grapheme clusters
//! are.** "é" can be one character or two (e + a combining accent); a
//! flag is two regional-indicator characters; a family emoji is several
//! people joined by zero-width joiners; a Devanagari syllable is several
//! characters drawn together. The cursor must never stop inside one, and
//! one Backspace deletes a whole cluster. Unicode defines these boundaries
//! (UAX #29, "extended grapheme clusters"); the `unicode-segmentation`
//! crate implements them. It isn't a dependency yet.
//!
//! **Glyphs and clusters don't line up one to one.**
//! - *One glyph, several clusters:* a ligature. "fi" is often one glyph
//!   covering two letters, and the cursor must still stop between f and i.
//!   With no better information, share the glyph's width out evenly by the
//!   number of clusters it covers, as browsers do. (Fonts can say exactly
//!   where a ligature's carets go, in the GDEF table's ligature caret list;
//!   even shares are the usual fallback.)
//! - *Several glyphs, one cluster:* a letter and its accent drawn as two
//!   glyphs, or an Indic syllable. One stop, at the cluster's start
//!   [`Glyph::cluster`](crate::layout::Glyph::cluster) says where each glyph's characters begin; glyphs
//!   sharing a cluster value belong together.
//!
//! **Spaces at a soft break take no room.** A line that broke at a space
//! holds that space in its text range ([`Line::text`](crate::layout::Line::text)), but the space isn't
//! drawn and doesn't widen the line. Stops after it can't be further right
//! than the line's end.
//!
//! **One offset, two places: affinity.** Where a line breaks softly, the
//! offset after the break is both the end of one line and the start of th
//! next. The cursor can show at either: after the last word of line 3, or
//! before the first word of line 4. UIKit calls this the position's
//! *affinity* (upstream: the line before; downstream: the line after). It
//! falls out naturally if that offset is a stop on *both* lines: the last
//! stop of one and the first of the next. Swift's positions carry which
//! one they mean.
//!
//! **A line end (`\n`) isn't a place to the right of the text.** The cursor
//! can stand before it, at the end of the line, but not after it on the
//! same line: after it is the start of the next paragraph.
//!
//! **An empty paragraph** sets as one empty line ([`set_paragraph`](crate::layout::set_paragraph)'s
//! promise), and it still needs one stop, at x = 0, or there's nowhere to
//! put the cursor in a blank line.
//!
//! **Justified lines** stretch and shrink their spaces. Glyph x already
//! includes that, so stops read from glyph positions come out right; stops
//! computed from advances alone wouldn't.
//!
//! **Right to left is coming (tolege#221).** Hebrew and Arabic set right to left,
//! and mixed lines run both ways at once. So nothing here may assume x
//! grows with the offset: keep stops in *text* order, each with its own x,
//! and let the lookups search by x when they need to.
//!
//! **Offsets are bytes here, UTF-16 in Swift.** Rust strings are UTF-8, so
//! offsets in Idem are byte offsets. The bridge (`tolege-ffi`) already
//! turns byte offsets into UTF-16 ones for glyph clusters; stops need the
//! same treatment there, not here.
//!
//! # Where this could live
//!
//! Your call: a field on [`Line`](crate::layout::Line), filled in while
//! lines are set (`set_lines` already walks every glyph), or a function
//! from a finished line. The bridge wants the stops with every line either
//! way, so they cross in the same call as the glyphs.
//!
//! # How to know it's right
//!
//! The same way the rest of Idem is tested: random paragraphs, every
//! line checked. For instance: every grapheme boundary in a line's range
//! has exactly one stop on that line; no stop falls inside a cluster; the
//! first and last stops are the line's ends; at a soft break, the last stop
//! of one line and the first of the next share an offset; in left-to-right
//! text, x never decreases along the stops; no stop lies past the line's
//! width.
//!

use crate::units::{Heading, Offset};

pub enum Affinity {
    Upstream,
    Downstream,
}

pub struct Caret {
    pub position: Offset,
    pub affinity: Affinity,
    text: &str,
}

// impl Default for Caret {
//     fn default(text: &str) -> Self {
//         Self {
//             position: Offset {
//                 offset: 0,
//                 due: Heading::East,
//                 text: text,
//             },
//             affinity: Affinity::Downstream,
//         }
//     }
// }
use unicode_segmentation::UnicodeSegmentation;

impl Caret {
    // TODO: add options
    pub fn new(text: &str) -> Self {
        Self {
            position: Offset {
                offset: 0,
                due: Heading::East,
            },
            affinity: Affinity::Downstream,
            text,
        }
    }

    // interface:
    // fn position - gets the caret position in em at this grapheme/glyph/cluster location
    // fn line - gets the line index of this grapheme/glyph/cluster
    // fn
    // fn
    // where is the cursor? (at this place in the text (rectangle))
    // I touched <here>, which place in the text is that?
    // which rectangles cover this selection? (boxes and spaces)
    // what place is one line above this location? below?

    pub fn position(&self) {
        if self.position.offset >= self.text.len() {
            panic!("at the disco")
        }
        let g = self.text.grapheme_indices(true);
        // self.text[self.position.offset]
        todo!();
    }
}
