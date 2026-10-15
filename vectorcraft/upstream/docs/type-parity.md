# Type parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-12 · **Change:** minor (Full-name font matching (#1258); Windows font-service QA pending; scores unchanged; Middle Eastern digit types: Character panel menu › Digits, `{digits}`; the font menu's script filter lists Thai fonts; Thai, Lao, Khmer and Myanmar lines break between words; the caret steps over whole clusters such as Thai consonants with their marks; Thai typed in a font without Thai falls back to a font made for Thai; earlier: Proportional Metrics in vertical type, `vpal`, #966) · **Target:** Adobe Illustrator 2026 (30.x)

Type is Illustrator's largest single area by use (weight 13 of 105 across "Type core" and "Type advanced" in
[target-app-parity.md](target-app-parity.md)). This checklist lists the features one by one. Status is from the
code and tests (`crates/text`, `crates/engine/src/cmd/type*.rs`, `textstyles.rs`, `crates/tools/src/text.rs`)
against Illustrator's public documentation; **estimated** unless noted.

## Summary

| Part | % | Remaining (h) |
|---|---:|---:|
| Type core (point, area, path type, Character/Paragraph, styles, threading, OpenType, fonts) | 80% | 14–19 |
| Type advanced (CJK composition, Middle Eastern, spelling, Touch Type, Retype, variable fonts) | 50% | 20–28 |

## Checklist

| Feature | Status | Notes |
|---|---|---|
| Point, area and type-on-a-path type; vertical variants | done / partial | vertical point, area and path type have initial support |
| Area Type Options (rows, columns, inset, first baseline, Auto Size, Shrink Text to Fit) | done | |
| Threaded text, Fit Headline, Text Wrap | done | |
| Type on a Path effects and options; brackets | done | #429 |
| Character and Paragraph panels, styles with overrides | done | mixed-value display for differing paragraphs missing |
| Character size and horizontal scale after object/group scaling | done | #1034: fields and setters compensate the stored affine; point size follows the transformed em-height axis, not the glyph ink bounds. Range edits, keyboard increments, leading and baseline shift use document points; rotation, reflection and shear remain editable. |
| Preserve font sizes while scaling a group | missing | Separate enhancement; scaling still scales text with the other artwork. |
| OpenType panel, Glyphs panel, alternates | done | Highlight Alternate Glyphs preference unused (#394) |
| Font menu: samples, live preview, filters, favourites; font matching by any name; Find Font | done | #339; script filter: Japanese, Latin, Thai |
| Missing glyphs set in an installed font made for their script (Thai, Lao, Khmer, Myanmar first; then any font that has them) | done | Thai: Leelawadee UI, Tahoma, Thonburi, Sukhumvit Set, Noto Sans Thai, TLWG |
| Full font-name matching (OpenType name ID 4) | done | #1258: loaded/cataloged faces and Missing Fonts searches; localized names and variable default faces; Windows font-service QA pending |
| Variable fonts | partial | named instances as styles; axis sliders missing |
| Colour fonts (OpenType SVG / COLR) | partial | |
| Every-line and Single-line composers | done | tuned by eye, never compared side by side |
| Hyphenation… and Justification… options | missing | |
| Optical Margin Alignment | missing | menu stub |
| Tab stops and leaders; Tabs panel | partial | Tabs panel and stops with leaders in the model (`text.tabs.set`); leader drawing not verified against the docs |
| Hidden characters (Show Hidden Characters) | done | `type.hiddenCharacters` |
| Spell check (Check Spelling, Auto Spell Check, custom dictionary) | missing | menu stubs; needs an openly licensed dictionary |
| Smart Punctuation, Change Case, Find & Replace | done | |
| Caret: the arrows and Delete step over whole clusters (Thai consonant with its marks, combining accents, emoji); Backspace removes one character | done | |
| Touch Type tool | missing | listed in the toolbar, not implemented |
| Retype (identify fonts in outlines/images) | missing | Illustrator's uses a cloud model |
| Snap to Glyph | missing | menu stub |
| Thai, Lao, Khmer, Myanmar: shaping (marks, reordering), lines broken between words, word-by-word caret and double-click | done | words found by ICU4X's line segmenter: dictionaries on the desktop, LSTM models on the web |
| Middle Eastern: bidirectional layout, Paragraph Direction | done | |
| Middle Eastern: digit types (Character panel menu › Digits: Default, Arabic, Hindi, Farsi) | done | `text.setFormat` / `text.setRangeStyle {digits}`; the text keeps the typed digits, the glyphs and SVG export follow the style |
| Middle Eastern: Character Direction, kashidas, Middle Eastern composers, split caret | missing | |
| CJK: input methods (marked text, candidate window) | done | |
| CJK: kinsoku (Hard, Soft, None), Line-end Punctuation Half Width, tate-chu-yoko (auto) | done / partial | custom kinsoku sets, the Mojikumi Settings dialog and the other mojikumi sets missing |
| CJK: Proportional Metrics (`palt` in horizontal type, `vpal` in vertical type) | done | #966 |
| CJK: ruby, manual tate-chu-yoko | missing | #633 |
| Inline symbols in text | beyond Illustrator | not scored |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-12 | minor | Full-name font matching and Missing Fonts searches (#1258): synthetic static/variable font regressions; Windows font-service QA pending; scores and hours unchanged |
| 2026-10-11 | minor | Middle Eastern: digit types done (Character panel menu › Digits: Default, Arabic, Hindi, Farsi; `text.setFormat` / `text.setRangeStyle {digits}`); Character Direction, kashidas, composers and the split caret still missing; scores unchanged at this rounding |
| 2026-10-11 | minor | Thai, Lao, Khmer and Myanmar words for the caret (Ctrl/Alt+arrows) and double-click; combining marks count as part of a word; score unchanged (estimated) |
| 2026-10-11 | minor | Font menu: a Thai script filter beside Japanese and Latin; score unchanged (estimated) |
| 2026-10-11 | minor | Thai, Lao, Khmer and Myanmar text breaks its lines between words (ICU4X line segmenter) instead of only at spaces; score unchanged (estimated) |
| 2026-10-11 | minor | The Type tool's arrows and Delete move by grapheme cluster, never between a Thai consonant and its vowel or tone marks; score unchanged (estimated) |
| 2026-10-11 | minor | Thai (Lao, Khmer, Myanmar) missing from the font in use falls back to an installed font made for the script instead of whichever face covering it loaded first; score unchanged (estimated) |
| 2026-10-11 | minor | Proportional Metrics done: `palt` in horizontal type, `vpal` in vertical type (#966, #1082); ruby and manual tate-chu-yoko still missing |
| 2026-10-10 | minor | #1034: transformed character controls and regression coverage; preserving group font sizes remains a separate enhancement. Scores/hours unchanged at this rounding. |
| 2026-10-10 | major | First checklist, split out of the parity table's Type core and Type advanced rows |
