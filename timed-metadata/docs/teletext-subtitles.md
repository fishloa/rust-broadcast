# EBU Teletext (ETSI EN 300 706 V1.2.1) subtitle decode — curated rules

_Source: `private/specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf`
("Enhanced Teletext specification"), 156 pages. Sections transcribed below via
`pdf2md` (text-layer engine, `--report` clean / all tokens verified) except
Table 35/36, which are **bitmap images** in the PDF (not extractable text —
read visually, page images rendered at 200 DPI and inspected directly)._

This document curates only the sections implemented by
`timed-metadata/src/webvtt/teletext.rs`: FEC (Hamming-8/4, odd parity),
packet addressing, page header control bits, the English national option
Latin G0 character set, and basic Level-1 page composition. It is **not** a
full transcription of EN 300 706 (156 pages covering Levels 1.5/2.5/3.5,
DRCS, objects, TOP/FLOF navigation, CA data broadcasting, etc. — out of scope,
see "Documented losses" in the module doc).

## Why this lives in `timed-metadata`, not `dvb-vbi`

`dvb-vbi` (ETSI EN 301 775, the VBI *carriage* spec) explicitly and
repeatedly states that EN 300 706 decode is out of its scope — its own
`docs/vbi.md` says: "EBU Teletext (EN 300 706) itself is out of scope here —
the value of this spec is the carriage layer plus the VPS/WSS/CC/monochrome
data units," and every module doc mentioning `TeletextDataField` repeats this.
This mirrors the project's `mpeg-ts` (generic TS framing) / `dvb-si` (PSI/SI
*content* decode) split: the low-level carriage crate stays framing-only, and
a higher-level crate owns the protocol's actual content decode. So this
module consumes only `dvb_vbi::TeletextDataField`'s raw 42-byte
`txt_data_block` — `dvb-vbi` itself is untouched by this issue.

## §7.1 — Elements of a Teletext packet (PDF pp. 16-17)

A full EN 300 706 packet is 45 bytes: bytes 1-3 are the clock-run-in +
framing code (already stripped by `dvb-vbi`'s carriage layer — its
`txt_data_block` is exactly EN 300 706 bytes 4-45, i.e. the packet address
plus the 40 payload bytes). Bytes are transmitted **LSB first** unless
otherwise stated.

### §7.1.2 — Packet address (bytes 4-5, i.e. `txt_data_block[0..2]`)

Both bytes are Hamming-8/4 coded.

| Function | Byte | Data Bit | Weighting | Range |
|---|---|---|---|---|
| Magazine (`X/` or `M/`) | 4 | 2, 4, 6 | 2^0, 2^1, 2^2 | 0-7 |
| Packet Number (`Y`) | 4 | 8 | 2^0 | |
| Packet Number (`Y`) | 5 | 2, 4, 6, 8 | 2^1..2^4 | 0-31 |

> "magazine number 8: a packet with a magazine value of 0 is referred to as
> belonging to magazine 8" (glossary, PDF p. 12).

`Y` distinguishes three packet types (§7.1.4): the page header (`Y=0`),
directly-displayable rows (`Y=1..25`), and non-displayable enhancement
packets (`Y=26..31`, not implemented here).

### §7.2.1 — Page/magazine association

"Following the page header packet of a page, all subsequent packets with
`Y=1` to `Y=28` inclusive, from the same magazine, relate to that page." Row
packets carry **no page number of their own** — the decoder must track which
page was most recently headed in that magazine. `PageAssembler` implements
exactly this: `Y=1..24` packets are only stored while the magazine's
last-seen header matched the tracked `(magazine, page)`.

## §8 — Byte coding and error protection (PDF pp. 20-21)

### §8.1 — Odd parity

> "In a single 8-bit byte, bit 8 is the parity bit (P) and bits 1 to 7 carry
> the data bits (D). Bit 8 is set so that there is an odd number of bits with
> the value '1' in the byte. **Single bit errors can be detected**" (no
> correction capability is claimed).
>
> Encoding: `P = 1 ⊕ D1 ⊕ D2 ⊕ D3 ⊕ D4 ⊕ D5 ⊕ D6 ⊕ D7`.
> Decoding: "if `D1⊕D2⊕D3⊕D4⊕D5⊕D6⊕D7⊕P = 1`, accept data bits."

Protects the 40 displayable/text bytes of every row packet (`decode_odd_parity`).

### §8.2 — Hamming 8/4

> "In a single 8-bit byte, bits 1, 3, 5 and 7 are the protection bits and
> bits 2, 4, 6 and 8 carry the data. **Single bit errors can be identified
> and corrected. Double bit errors can be detected.**"
>
> Wire order (transmission order, LSB first): `P1 D1 P2 D2 P3 D3 P4 D4`.
>
> Encoding:
> ```text
> P1 = 1 ⊕ D1 ⊕ D3 ⊕ D4
> P2 = 1 ⊕ D1 ⊕ D2 ⊕ D4
> P3 = 1 ⊕ D1 ⊕ D2 ⊕ D3
> P4 = 1 ⊕ P1 ⊕ D1 ⊕ P2 ⊕ D2 ⊕ P3 ⊕ D3 ⊕ D4
> ```

Protects packet addresses and page header fields (`decode_hamming_8_4`).
This crate's decoder is a brute-force nearest-codeword search against the
encoding equations above rather than a hand-transcribed "four parity tests
A-D" lookup table (the PDF's table-column extraction for that table was
ambiguous — three columns, P2/D2/P3, collapsed into one cell by the text
extractor — whereas the encoding equations extracted cleanly and identically
across two independent extraction passes). The brute-force search is
mathematically equivalent: this is an extended (7,4) Hamming code with
minimum distance 4, so a genuine double-bit error is never within distance 1
of any valid codeword (proof: if it were, the two codewords it sits near
would be within distance 1+2=3 of each other, contradicting minimum distance
4). Cross-checked against one manually-computed byte
(`hamming_manual_cross_check_against_spec_encoding_equations` test) and
exhaustive round-trip + single/double-bit-error tests over all 16 nibbles.

## §9.3.1 — Page header packet (PDF pp. 24-25)

Page header packets (`Y=0`) comprise the page address, control bits, and
row-0 display data.

### §9.3.1.1 — Page number (bytes 6-7, both Hamming-8/4)

Page units (byte 6) and page tens (byte 7), each a 4-bit nibble, `0x0-0xF`.

### §9.3.1.2 — Page sub-code (bytes 8-11)

S1 (byte 8, full nibble), S2 (byte 9, data bits 2/4/6 — the low 3 bits),
S3 (byte 10, full nibble), S4 (byte 11, data bits 2/4 — the low 2 bits).
`PageHeader` decodes all four (`s1`-`s4`) for spec fidelity, but
`PageAssembler` deliberately ignores sub-code when matching a page (a
documented simplification — multi-subpage rotation, e.g. per-language
subtitle variants sharing one page number, is not distinguished).

### Table 2 — Control bits (PDF p. 24)

| Bit | Location | Function |
|---|---|---|
| C4 Erase Page | byte 9, bit 8 | clear previous transmission's rows before storing the new one |
| C5 Newsflash | byte 11, bit 6 | boxed/inset display |
| C6 Subtitle | byte 11, bit 8 | page is a subtitle page, boxed/inset display |
| C7 Suppress Header | byte 12, bit 2 | row 0 not displayed |
| C8 Update Indicator | byte 12, bit 4 | editorial "changed since last transmission" flag |
| C9 Interrupted Sequence | byte 12, bit 6 | page out of numerical sequence |
| C10 Inhibit Display | byte 12, bit 8 | rows 1-24 not displayed |
| C11 Magazine Serial | byte 13, bit 2 | serial (`1`) vs parallel (`0`) page-termination mode |
| C12,C13,C14 National Option | byte 13, bits 4/6/8 | G0 character set national option (see below) |

`C6` (Subtitle) is the spec's actual mechanism for identifying a subtitle
page — **not** a fixed magazine/page-number convention. (In practice, many
real-world services also happen to use "page 888" as a memorable subtitle
page number, but that is broadcaster convention, not something EN 300 706
mandates; `PageAssembler` matches on the caller-supplied `(magazine, page)`
regardless.)

## §15.1-15.2 — National option character subset (PDF pp. 100-101)

> "these national option sub-sets are selected by the C12, C13 and C14
> control bits in the page header" ... "At levels 1 and 1.5 the national
> option sub-set in use on the page is defined by the C12, C13 and C14
> control bits in the page header **alone** and, in theory, this will result
> in an ambiguous reference to an entry in table 32" — i.e. at Level 1 (this
> crate's scope) the 3-bit value indexes Table 32's first ("Latin 0") group
> only, values `000`-`110` = English/German/Swedish-Finnish-Hungarian/
> Italian/French/Portuguese-Spanish/Czech-Slovak, `111` reserved.
> `(c12<<2)|(c13<<1)|c14` is the packed 3-bit value ("bit 10 = C12, bit 9 =
> C13, bit 8 = C14" per Table 32 note 1).

`NationalOption` decodes and labels all 8 values (spec fidelity — every
field value has a name), but only `NationalOption::English`'s character
substitutions are applied by `latin_g0_char` (see below).

## Table 35/36 — Latin G0 character set (PDF pp. 106-107, **bitmap images**)

Unlike every other table cited above, Table 35 (Latin G0 Primary Set) and
Table 36 (Latin National Option Sub-sets) render as bitmap glyph charts in
the PDF (`natopt_1.bmp`/`natopt_2.bmp` placeholders in the text layer) — no
text to extract. Read visually (page images rendered via `pdftoppm -r 200`
and inspected directly):

- **Table 35** confirms the base Latin G0 set is 7-bit ASCII `0x20`-`0x7F`
  laid out as columns `2-7` (`B7 B6 B5` = `010..111`) × rows `0-F`
  (`B4 B3 B2 B1`), **except** 13 shaded ("reserved") positions where a
  national option substitutes a different glyph when the page is read
  directly (as opposed to via a `X/26` enhancement packet, out of scope
  here): `0x23 0x24 0x40 0x5B 0x5C 0x5D 0x5E 0x5F 0x60 0x7B 0x7C 0x7D 0x7E`.
  Position `0x7F` is a full block (note 4: "occupies an area ... a rectangle
  surrounded by the background colour").
- **Table 36**'s "English" row gives the substitution at each of those 13
  positions:

  | Code | Glyph | Unicode |
  |---|---|---|
  | `0x23` | £ | U+00A3 POUND SIGN |
  | `0x24` | $ | U+0024 (unchanged from base) |
  | `0x40` | @ | U+0040 (unchanged from base) |
  | `0x5B` | ← | U+2190 LEFTWARDS ARROW |
  | `0x5C` | ½ | U+00BD VULGAR FRACTION ONE HALF |
  | `0x5D` | → | U+2192 RIGHTWARDS ARROW |
  | `0x5E` | ↑ | U+2191 UPWARDS ARROW |
  | `0x5F` | # | U+0023 NUMBER SIGN |
  | `0x60` | (horizontal bar) | U+2015 HORIZONTAL BAR — **judgment call**: the spec's bitmap glyph is a plain horizontal rule; this crate picked U+2015 as the closest Unicode codepoint. Worth double-checking against a reference decoder if exact fidelity matters. |
  | `0x7B` | ¼ | U+00BC VULGAR FRACTION ONE QUARTER |
  | `0x7C` | ‖ | U+2016 DOUBLE VERTICAL LINE |
  | `0x7D` | ¾ | U+00BE VULGAR FRACTION THREE QUARTERS |
  | `0x7E` | ÷ | U+00F7 DIVISION SIGN |

  The other 12 national options in Table 36 (Czech/Slovak, Estonian, French,
  German, Italian, Lettish/Lithuanian, Polish, Portuguese/Spanish, Rumanian,
  Serbian/Croatian/Slovenian, Swedish/Finnish/Hungarian, Turkish) are visible
  in the same chart but **not implemented** — `latin_g0_char` falls back to
  the base ASCII/IRV glyph for every option other than English (a documented
  gap, not a silent mis-render: the base glyph is still a plausible, just
  not nationally-correct, rendering).

## Documented scope (what is deliberately NOT implemented)

- **Enhancement packets** `X/26` (character/attribute overwrite — Level 1.5),
  `X/27`/`X/28`/`M/29` (page linking, character-set re-designation, side
  panels, CLUTs — Levels 2.5/3.5) are not processed. Only basic Level-1 page
  composition (`X/0` header + `X/1`-`X/24` rows) is decoded.
- **Styling**: Level-1 spacing-attribute control codes (`0x00`-`0x1F`,
  clause 12.2 — colour, flash, double-height, box mode) are rendered as a
  plain space, matching this crate's existing CEA-608/708 extractors'
  documented no-styling policy.
- **Sub-code / multi-subpage**: ignored for page matching.
- **National options**: only English's Table 36 substitutions are applied.

## Fixture strategy (honesty note)

No real DVB VBI-teletext-bearing capture exists in this workspace's
`fixtures/` tree at the time of writing (searched for teletext/VBI PIDs;
found only `fixtures/dvb-vbi/vbi_data_field.bin`, itself a hand-built
synthetic fixture, not a broadcast capture). EN 300 706 also has no worked
byte-level page example in its annexes to lift verbatim. Per this project's
established fallback (`fixtures/cc/cea608_cc1_synthetic.txt` sets the
precedent), the committed fixture
`fixtures/teletext/teletext_subtitle_synthetic.txt` is **constructed**, using
this crate's own verified `encode_hamming_8_4`/`encode_odd_parity` (proven
correct by the exhaustive round-trip tests above) to produce spec-valid wire
bytes for a synthetic magazine-8/page-0x88 subtitle page — not a captured
broadcast stream.
