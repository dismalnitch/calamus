//! bidi
//!
//! Which way each part of a paragraph runs, for text that mixes scripts
//! running left to right with ones running right to left.
//!
//! Two things decide it, and they're kept apart:
//!
//! - The text's **isolates** ([`Buffer::isolates`](crate::buffer::Buffer::isolates)):
//!   stretches the buffer keeps as set in a direction of their own. They
//!   don't change because of what's typed in them.
//! - **Unicode's bidirectional algorithm** (UAX #9), which reads the
//!   characters. It's run *inside* what the isolates fix: a number in a
//!   Hebrew isolate still reads left to right, and an English word in one
//!   is set left to right, in its place in the Hebrew.
//!
//! So the algorithm does two jobs here. [`levels`] is the one layout uses:
//! given a paragraph and its isolates, the level of every byte (even: left
//! to right; odd: right to left; higher: nested deeper). [`propose`] is
//! the one the editor uses: for text no isolate holds yet, which stretches
//! the algorithm would set against the paragraph, and so should become
//! isolates. Once they are, [`levels`] keeps them whole, where the
//! algorithm alone would re-read them on every keystroke: a Latin letter
//! typed into a Hebrew word would split it in two and swap the halves.
//!
//! A paragraph's own direction ([`base`]) is its first strong character's:
//! a letter of a script that runs one way. It isn't an isolate.

use std::ops::Range;

use unicode_bidi::{BidiInfo, Level};

use crate::buffer::Direction;

/// The isolate characters Unicode has for this, put into a copy of the
/// text to tell the algorithm where the isolates are. Never into the
/// text itself.
const LEFT_TO_RIGHT_ISOLATE: char = '\u{2066}';
const RIGHT_TO_LEFT_ISOLATE: char = '\u{2067}';
const POP_ISOLATE: char = '\u{2069}';

/// An isolate, as far as a paragraph's directions go: the bytes of the
/// paragraph it covers, and which way it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stretch {
    pub range: Range<usize>,
    pub direction: Direction,
}

/// What should become of a paragraph's isolates: `remove` these (indices
/// into the ones [`propose`] was given), and make `mark` one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub remove: Vec<usize>,
    pub mark: Stretch,
}

fn direction(level: Level) -> Direction {
    if level.is_rtl() {
        Direction::RightToLeft
    } else {
        Direction::LeftToRight
    }
}

fn level(n: u8) -> Level {
    Level::new(n).expect("a level the algorithm gave, or 0 or 1")
}

/// The paragraph's own level: 1 if its first strong character runs right
/// to left, else 0.
pub fn base(paragraph: &str) -> u8 {
    BidiInfo::new(paragraph, None)
        .paragraphs
        .first()
        .map_or(0, |p| p.level.number())
}

/// Which way `text`'s first strong character runs: `None` if it has
/// none (spaces, digits, punctuation).
pub fn first_strong(text: &str) -> Option<Direction> {
    use unicode_bidi::BidiClass::{AL, L, R};
    text.chars()
        .find_map(|c| match unicode_bidi::bidi_class(c) {
            L => Some(Direction::LeftToRight),
            R | AL => Some(Direction::RightToLeft),
            _ => None,
        })
}

/// The level of every byte of `paragraph` (one paragraph: no line ends),
/// given its `isolates`, which must nest.
pub fn levels(paragraph: &str, isolates: &[Stretch]) -> Vec<u8> {
    let base = level(base(paragraph));
    if isolates.is_empty() {
        return BidiInfo::new(paragraph, Some(base))
            .levels
            .iter()
            .map(|l| l.number())
            .collect();
    }
    // A copy with the isolates written in. Where several start or end at
    // one place: ends first, the innermost first; then starts, the
    // outermost first.
    let mut marks: Vec<(usize, bool, usize, char)> = Vec::with_capacity(isolates.len() * 2);
    for i in isolates {
        let open = match i.direction {
            Direction::LeftToRight => LEFT_TO_RIGHT_ISOLATE,
            Direction::RightToLeft => RIGHT_TO_LEFT_ISOLATE,
        };
        marks.push((i.range.start, true, usize::MAX - i.range.len(), open));
        marks.push((i.range.end, false, i.range.len(), POP_ISOLATE));
    }
    marks.sort_unstable();
    let mut copy = String::with_capacity(paragraph.len() + marks.len() * 3);
    // For each byte of the copy, whether it's the paragraph's.
    let mut own = Vec::with_capacity(copy.capacity());
    let mut next = marks.iter().peekable();
    for (at, c) in paragraph.char_indices().chain([(paragraph.len(), '\0')]) {
        while let Some(&(_, _, _, mark)) = next.next_if(|m| m.0 <= at) {
            copy.push(mark);
            own.extend(std::iter::repeat_n(false, mark.len_utf8()));
        }
        if at < paragraph.len() {
            copy.push(c);
            own.extend(std::iter::repeat_n(true, c.len_utf8()));
        }
    }
    let info = BidiInfo::new(&copy, Some(base));
    info.levels
        .iter()
        .zip(own)
        .filter(|&(_, own)| own)
        .map(|(l, _)| l.number())
        .collect()
}

/// What the algorithm would make of `paragraph`'s isolates, one step at a
/// time: `None` when it has nothing to say. `isolates` are the ones
/// straight in the paragraph, not nested in another.
///
/// The algorithm reads the bare characters and finds each stretch set
/// against the paragraph. A stretch no isolate holds becomes one. A
/// stretch that takes in isolates of its own direction and more besides
/// (a second Hebrew word typed after a space, outside the first's
/// isolate) becomes one isolate over all of it, in place of those. A
/// stretch already inside an isolate is left alone, as is anything near
/// an isolate that runs the other way: someone meant that.
pub fn propose(paragraph: &str, isolates: &[Stretch]) -> Option<Proposal> {
    let base = base(paragraph);
    let info = BidiInfo::new(paragraph, Some(level(base)));
    let levels = &info.levels;
    let mut at = 0;
    while at < paragraph.len() {
        if levels[at].number() <= base {
            at += 1;
            continue;
        }
        let start = at;
        let mut lowest = levels[at];
        while at < paragraph.len() && levels[at].number() > base {
            lowest = lowest.min(levels[at]);
            at += 1;
        }
        let found = Stretch {
            range: start..at,
            direction: direction(lowest),
        };
        let touching: Vec<usize> = (0..isolates.len())
            .filter(|&i| {
                isolates[i].range.start < found.range.end
                    && found.range.start < isolates[i].range.end
            })
            .collect();
        if touching
            .iter()
            .any(|&i| isolates[i].direction != found.direction)
        {
            continue;
        }
        let holds = |i: usize| {
            isolates[i].range.start <= found.range.start && found.range.end <= isolates[i].range.end
        };
        if touching.iter().any(|&i| holds(i)) {
            continue;
        }
        let from = touching
            .iter()
            .map(|&i| isolates[i].range.start)
            .fold(found.range.start, usize::min);
        let to = touching
            .iter()
            .map(|&i| isolates[i].range.end)
            .fold(found.range.end, usize::max);
        return Some(Proposal {
            remove: touching,
            mark: Stretch {
                range: from..to,
                direction: found.direction,
            },
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use Direction::{LeftToRight as Ltr, RightToLeft as Rtl};

    fn stretch(text: &str, what: &str, direction: Direction) -> Stretch {
        let at = text.find(what).unwrap();
        Stretch {
            range: at..at + what.len(),
            direction,
        }
    }

    /// The level of each character, as a string of digits.
    fn shown(text: &str, isolates: &[Stretch]) -> String {
        let levels = levels(text, isolates);
        text.char_indices()
            .map(|(i, _)| char::from(b'0' + levels[i]))
            .collect()
    }

    #[test]
    fn a_paragraph_runs_as_its_first_strong_character() {
        assert_eq!(base("abc אבג"), 0);
        assert_eq!(base("אבג abc"), 1);
        assert_eq!(base("123 ... אבג"), 1);
        assert_eq!(base("123 ..."), 0);
        assert_eq!(base(""), 0);
    }

    #[test]
    fn the_first_strong_character() {
        assert_eq!(first_strong("12, אb"), Some(Rtl));
        assert_eq!(first_strong(" (b"), Some(Ltr));
        assert_eq!(first_strong("ع"), Some(Rtl));
        assert_eq!(first_strong("12 . "), None);
    }

    #[test]
    fn levels_without_isolates_are_the_algorithms() {
        assert_eq!(shown("abc אבג def", &[]), "00001110000");
        // A number in Hebrew reads left to right, a level up.
        assert_eq!(shown("ab אב 12 גד", &[]), "00011122111");
        assert_eq!(shown("אבג abc", &[]), "1111222");
    }

    #[test]
    fn an_isolate_keeps_what_the_characters_would_split() {
        // A Latin letter typed into a Hebrew word. Left to the algorithm,
        // the word is two runs with the letter between, in the
        // paragraph's order: its halves change places.
        let text = "abc אבdג def";
        assert_eq!(shown(text, &[]), "000011010000");
        // In an isolate the word is one stretch, and the letter sits in
        // it.
        assert_eq!(shown(text, &[stretch(text, "אבdג", Rtl)]), "000011210000");
    }

    #[test]
    fn an_english_word_in_a_hebrew_sentence() {
        let text = "say אבג abc דהו end";
        // The characters can't say the sentence is one thing.
        assert_eq!(shown(text, &[]), "0000111000001110000");
        // An isolate can, and the algorithm sets the word inside it.
        let sentence = stretch(text, "אבג abc דהו", Rtl);
        assert_eq!(shown(text, &[sentence.clone()]), "0000111122211110000");
        // Nested isolates say the same.
        let word = stretch(text, "abc", Ltr);
        assert_eq!(shown(text, &[sentence, word]), "0000111122211110000");
    }

    #[test]
    fn isolates_that_share_an_edge() {
        let text = "אבג12";
        let outer = Stretch {
            range: 0..8,
            direction: Rtl,
        };
        let inner = Stretch {
            range: 6..8,
            direction: Ltr,
        };
        // In a right-to-left paragraph, a right-to-left isolate is two
        // levels up: the next odd one.
        assert_eq!(shown(text, &[outer.clone(), inner.clone()]), "33344");
        assert_eq!(shown(text, &[inner, outer]), "33344");
    }

    #[test]
    fn text_no_isolate_holds_is_proposed() {
        let text = "abc אבג def";
        assert_eq!(
            propose(text, &[]),
            Some(Proposal {
                remove: vec![],
                mark: stretch(text, "אבג", Rtl)
            })
        );
        assert_eq!(propose(text, &[stretch(text, "אבג", Rtl)]), None);
        assert_eq!(propose("only one way", &[]), None);
        assert_eq!(propose("אבג דהו", &[]), None);
        // Two words and the space between are one stretch.
        let text = "abc אבג דהו def";
        assert_eq!(
            propose(text, &[]),
            Some(Proposal {
                remove: vec![],
                mark: stretch(text, "אבג דהו", Rtl)
            })
        );
    }

    #[test]
    fn a_word_typed_after_an_isolate_joins_it() {
        let text = "abc אבג ד";
        assert_eq!(
            propose(text, &[stretch(text, "אבג", Rtl)]),
            Some(Proposal {
                remove: vec![0],
                mark: stretch(text, "אבג ד", Rtl)
            })
        );
        // And two isolates with Hebrew typed between them become one.
        let text = "x בגד הו zz";
        let isolates = [stretch(text, "בג", Rtl), stretch(text, "הו", Rtl)];
        assert_eq!(
            propose(text, &isolates),
            Some(Proposal {
                remove: vec![0, 1],
                mark: stretch(text, "בגד הו", Rtl)
            })
        );
    }

    #[test]
    fn whats_in_an_isolate_is_left_alone() {
        // The isolate is bigger than what the algorithm would find: it
        // holds a Latin letter. Nothing to propose.
        let text = "abc אבdג def";
        assert_eq!(propose(text, &[stretch(text, "אבdג", Rtl)]), None);
        // An isolate the other way over Hebrew: someone meant it.
        let text = "abc אבג def";
        assert_eq!(propose(text, &[stretch(text, "אבג", Ltr)]), None);
    }
}
