# Hebrew edge-case tests

`tests/it/hebrew.rs`: 44 tests of Hebrew, the hard parts. They're marked *next version*: every one is `#[ignore]`d, so none of them turns the build red while the editor's foundations are still moving. Run them with:

```bash
cargo test --test it hebrew:: -- --ignored
```

When the suite is taken on, a test that passes loses its `#[ignore]` and becomes an ordinary test. A failing test is either fixed, or has its expectation changed by a decision, and then it loses its `#[ignore]` too.

These are checks that the editor *behaves*: where the cursor goes, what's a word, what's kept. They can't say whether marks are drawn in the right place on their letters; the picture snapshots below hold that still, and a person judges it.

## Where it stands (26 November 2026, calamus main + this branch)

**42 pass, 2 fail.**

### Failing

| Test | What it found |
|---|---|
| `sof_pasuq_ends_a_sentence` | A sentence runs past a sof pasuq (׃, U+05C3). Unicode's sentence rules don't treat it as the end of a sentence; a reader of Biblical Hebrew does. Fixing it means adding U+05C3 to what ends a sentence (in mamre's units, where Calamus gets sentences). |
| `an_empty_line_in_hebrew_text_is_hebrew` | **Decision.** An empty paragraph between Hebrew ones runs left to right, so the cursor on it sits at the left. Unicode's default does that; Word and Pages give it the direction of the paragraph around it. The test asks for Word's behaviour. |

### Passing, and what they hold the editor to

- **Points and accents**
  - A letter with its points and accents is one character to the cursor. That holds for every non-spacing mark from U+0591 to U+05C7, and with shin and sin dots, holam haser, qamats qatan and the upper and lower dots.
  - Neither an arrow nor a touch ever stops inside a letter, and the arrows reach every letter of a verse.
  - Points are kept in the order typed, never normalized; a combining grapheme joiner is kept and isn't a stop.
  - A point with no letter before it breaks nothing.
- **Backspace** takes a whole pointed letter. **Decision:** some Hebrew keyboards take only the last point. If Hebrew writers want that, the test changes.
- **Words**
  - A pointed word is one word, and a verse counts seven.
  - Words joined by a maqaf are two words, with the maqaf in neither. **Decision.**
  - An abbreviation with gershayim is one word, whether written with ״ or with ASCII ", and so is a name with a geresh. A word in quotation marks is the word alone.
  - A paseq and a sof pasuq sit between words. A Yiddish ligature is a letter.
- **Direction**
  - The first letter with a direction decides the paragraph's; digits, quote marks, brackets or a dash before it don't.
  - Numbers in Hebrew read left to right.
  - A full stop goes to the left of the Hebrew word it ends.
  - A comma after Hebrew in English text stays with the English.
  - An English word sits between the Hebrew words around it.
  - Brackets face the right way.
- **The cursor**
  - The right arrow goes back in the text in Hebrew.
  - The arrows cross Hebrew, English, numbers and brackets and come back.
  - Up and down keep their place in wrapped Hebrew.
- **Selection**
  - A Hebrew word in English selects as one box.
  - A selection across a change of direction is split into the boxes it covers.
  - Deleting a selected pointed word leaves no stray points.
- **Lines**
  - No line starts inside a letter or with a point.
  - A wrapped Hebrew line starts at the right.
  - A line breaks after a maqaf, not before it.
  - No line starts with a sof pasuq or a full stop.
- **Typing**
  - Letters then their points leave the cursor after them.
  - A point typed after a space is kept.
  - What an input method composes lands where it was typed.
  - Final letters are kept as typed.
  - Presentation forms are kept as pasted (**decision**; see search, below).
  - Direction marks and controls pasted in are kept, and don't trap the cursor (**decision**).
- **Keeping it**
  - UTF-16 offsets round-trip through pointed Hebrew and an emoji.
  - A saved history brings back pointed Hebrew with its English.

A pass means the editor did what the test says, with these fonts and at this width. It isn't proof for every font or every keyboard.

## Decisions written into the tests

Each one is a sentence starting "Decision:" in its test. They're for a Hebrew reader to confirm or change, with Kat ideally:

1. **Backspace on a pointed letter.** Whole letter (as now), or the last point first?
2. **Maqaf.** Two words (as now), or one word for selection?
3. **An empty line between Hebrew paragraphs.** Right to left like its neighbours, or Unicode's left to right? The test wants the first; the editor does the second.
4. **Presentation forms pasted from a PDF** (U+FB1D–U+FB4F). Kept as pasted (as now), or turned into letter plus points on the way in?
5. **Direction controls pasted in** (RLM, LRM, embeddings, overrides, isolates). Kept (as now), or turned into Calamus's own isolates and removed?

## Marks drawn in the right place: picture snapshots

`tests/it/hebrew_marks.rs`: 10 tests, each a line of pointed and cantillated Hebrew drawn in four fonts, so 40 pictures. Ignored like the rest of the suite. Run them with:

```bash
cargo test --test it hebrew_marks:: -- --ignored
```

Each line is set by Calamus's own `Editor` (the shaping, bidi and line breaking the app uses), and its glyph outlines are filled into a PNG at 28 pt, 144 dpi (2 px a point), 460 pt wide. The PNG is compared with `tests/snapshots/hebrew_marks/<case>-<font>.png`; more than 12 pixels changed by more than a quarter of their range fails. A failure writes what was drawn (`.actual.png`) and a diff (the reference faded, changed pixels red) to `target/hebrew_marks/`, which git ignores.

To make the references again on purpose, after a change that should move marks (a new font, a shaping fix):

```bash
UPDATE_SNAPSHOTS=1 cargo test --test it hebrew_marks:: -- --ignored
```

then look at every changed picture before committing it.

**Fonts**, all SIL Open Font License, in `fonts/` with their licences: Ezra SIL and Ezra SIL SR 2.51 (`SILEOT.ttf`, `SILEOTSR.ttf`, from SIL; their Hebrew layout code is also MIT, see `EzraSIL-Licenses.txt`), Noto Serif Hebrew and Noto Sans Hebrew (from Google's notofonts). SBL Hebrew isn't here: its licence doesn't allow passing it on. Each line is drawn in one font alone, with no fallback, and a font's missing glyph is drawn red, so a mark the font hasn't got shows.

**The lines**, all from the UXLC (Unicode/XML Leningrad Codex 2.6), marks in its order:

| Case | What it's for |
|---|---|
| `ruth_1_1` | Ruth 1:1, the whole verse, wrapped: an ordinary verse with everything at once. |
| `dagesh` | Dagesh in a letter with a vowel below (בְּ, שָּׁ, בִּֽ). |
| `shin_sin_holam` | A holam before a shin dot (מֹשֶׁ֖ה), a holam with a sin dot (שֹׂנְאֶ֔יךָ), and a sin with dagesh and holam. |
| `meteg` | Meteg beside hiriq, and beside hataf patah. |
| `stacked` | Two accents on one letter, above and below (אֱלֹהִ֥֨ים, לְךָ֛֩), and two above (תְּמוּנָ֔֡ה). |
| `prepositive` | Telisha gedola, yetiv and dehi, which belong at a word's right edge. |
| `postpositive` | Pashta and telisha qetana, which belong at a word's left edge. |
| `final_kaf_qamats` | Qamats inside a final kaf. |
| `holam_haser` | Holam haser on vav (U+05BA), beside a holam male to compare. |
| `cgj` | A combining grapheme joiner keeping Jerusalem's two vowels on the lamed. |

**What the snapshots can and can't say.** A pass means the picture hasn't changed. It doesn't mean the picture is right: that's for a reader of Hebrew to judge once, by eye, on the references, and again whenever they're remade. Calamus doesn't move marks itself; where they land is what the font's tables say, through rustybuzz. So a wrong mark is usually the font's, and the fix is a different font or a font feature, not layout code.

**What Claude saw in the references (28 November 2026, not a Hebraist's judgment):**

- Ezra SIL and Ezra SIL SR: every case looked as intended. Stacked accents sit side by side, prepositive and postpositive accents go to the word's right and left edges, meteg sits left of hiriq, the CGJ keeps the hiriq visible beside the qamats and meteg. Hataf patah with meteg is one composed glyph in this font.
- Noto Serif Hebrew: the hiriq after a CGJ is drawn on the meteg (and in the second word on the tipha), so it can't be seen. The meteg is drawn on top of a hiriq, and in the middle of a hataf patah. Pazer is drawn on top of zaqef qatan. A holam before a shin all but touches the shin dot. Holam haser goes left of the vav, as it should.
- Noto Sans Hebrew: the hiriq after a CGJ is hidden the same way. Pazer is stacked on top of zaqef qatan instead of beside it. Holam haser is drawn in exactly the same place as holam male, so מִצְוֺת and מוֹ look the same. The meteg sits left of hiriq and of hataf patah.
- Both Noto fonts: telisha gedola sits on the top of the lamed's mast instead of clear to its right.
- No font drew a missing glyph (red) in any case.

**For a Hebrew reader to judge, by eye:** whether each mark is where a reader expects it, in each font; whether a holam and a sin dot on one letter should show as one dot or two; where yetiv and dehi belong under their first letter; and which of these fonts Calamus should offer for pointed and cantillated text.

## Edge cases the editor has no way to test yet

Each of these needs something the editor doesn't have. Add a test when it does:

- **Search.** Pointed text should match unpointed text, and with or without cantillation. Presentation forms should match letter plus points. Canonically equivalent orders of points should match, and so should final and medial forms where a search wants them to.
- **Fonts that lack cantillation.** Falling back for accents a font hasn't got, without splitting a letter between two fonts.
- **Ketiv and qere,** and other scholarly marks: brackets for restorations, sigla, superscript verse numbers.
- **Three scripts in a line:** Hebrew, Greek and English together, as in a commentary.
- **Spell checking** that ignores points and knows words joined by a maqaf.
- **Copying out:** what lands on the clipboard (plain text with direction marks, or not) and how it pastes into Word.
- **Undo,** once the editor has it: undoing a typed point, and undoing across a change of direction.
