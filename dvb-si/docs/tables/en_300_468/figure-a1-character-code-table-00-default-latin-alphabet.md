## Figure A.1 — Character code table 00 (default Latin alphabet)
_Annex A, PDF p. 159 — hand-transcribed from the figure image (the geometry
extractor only captures `Table N` captions; figures are reproduced manually).
Source: `specs/etsi_en_300_468_v01.19.01_dvb_si.pdf`, EN 300 468 V1.19.1
(2025-02), "Character code table 00 - Latin alphabet with Unicode equivalents"._

> NOTE (verbatim from the spec): "This table is a superset of ISO/IEC 6937 [37]
> with addition of the Euro symbol (U+20AC) in position 0xA4."

Rows 0x20–0x7E are identical to 7-bit US-ASCII (U+0020–U+007E). Rows 0x80–0x9F
are unused by this figure (Annex A.2 assigns control codes there). Each cell
below is `glyph U+codepoint`; `—` marks an undefined (grey) position.

| | -0 | -1 | -2 | -3 | -4 | -5 | -6 | -7 | -8 | -9 | -A | -B | -C | -D | -E | -F |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **A-** | NBSP 00A0 | ¡ 00A1 | ¢ 00A2 | £ 00A3 | € 20AC | ¥ 00A5 | — | § 00A7 | ¤ 00A4 | ' 2018 | " 201C | « 00AB | ← 2190 | ↑ 2191 | → 2192 | ↓ 2193 |
| **B-** | ° 00B0 | ± 00B1 | ² 00B2 | ³ 00B3 | × 00D7 | µ 00B5 | ¶ 00B6 | · 00B7 | ÷ 00F7 | ' 2019 | " 201D | » 00BB | ¼ 00BC | ½ 00BD | ¾ 00BE | ¿ 00BF |
| **C-** | — | ◌̀ 0300 | ◌́ 0301 | ◌̂ 0302 | ◌̃ 0303 | ◌̄ 0304 | ◌̆ 0306 | ◌̇ 0307 | ◌̈ 0308 | — | ◌̊ 030A | ◌̧ 0327 | — | ◌̋ 030B | ◌̨ 0328 | ◌̌ 030C |
| **D-** | ― 2015 | ¹ 00B9 | ® 00AE | © 00A9 | ™ 2122 | ♪ 266A | ¬ 00AC | ¦ 00A6 | — | — | — | — | ⅛ 215B | ⅜ 215C | ⅝ 215D | ⅞ 215E |
| **E-** | Ω 2126 | Æ 00C6 | Đ 0110 | ª 00AA | Ħ 0126 | — | Ĳ 0132 | Ŀ 013F | Ł 0141 | Ø 00D8 | Œ 0152 | º 00BA | Þ 00DE | Ŧ 0166 | Ŋ 014A | ŉ 0149 |
| **F-** | ĸ 0138 | æ 00E6 | đ 0111 | ð 00F0 | ħ 0127 | ı 0131 | ĳ 0133 | ŀ 0140 | ł 0142 | ø 00F8 | œ 0153 | ß 00DF | þ 00FE | ŧ 0167 | ŋ 014B | SHY 00AD |

The C- row holds **non-spacing diacritical marks** (Unicode combining
codepoints): in ISO 6937 wire order the mark byte *precedes* the base letter
(`0xC8 0x75` → ü). Positions 0xC0, 0xC9 and 0xCC are undefined.

Implemented by `dvb-si/src/text/mod.rs` (`iso_6937_single`, `combining_mark`,
`combine`); pinned by the `figure_a1_*` tests in the same file.
