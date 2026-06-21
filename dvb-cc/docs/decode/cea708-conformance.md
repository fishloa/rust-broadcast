# CEA-708 caption decoder conformance model — 47 CFR §79.102

_Source: 47 CFR §79.102 ("Closed caption decoder requirements for digital
television receivers"), US Government public domain (Title 47, Code of Federal
Regulations) + CEA-708 (EIA-708-B / ANSI-CTA-708-E) as incorporated by reference.
Transcribed from the authoritative eCFR / Cornell LII text._

This document captures the **decode CONFORMANCE / SEMANTIC model** that the FCC
regulation imposes on DTV closed-caption decoders. It is the public-domain
grounding for the CEA-708 (DTVCC) decode layer that folds **into the existing
`dvb-cc` crate** (above the `cc_data()` carriage already implemented from ETSI TS
101 154 Table B.9 — see [`../ts_101_154/b9-cc-data.md`](../ts_101_154/b9-cc-data.md)).

§79.102 mandates the decoder's display/rendering behaviour and which parts of the
CEA-708 code spaces, services, window/pen model, colour/opacity/edge tables and
screen-coordinate limits a conforming decoder must implement. It does **not**
reproduce the bit-level DTVCC packet/service-block framing nor the exact command
opcode byte values — those live in the (copyright, cart-gated) CTA standards. See
[**Gaps — needs CTA-708-E**](#gaps--needs-cta-708-e) for the precise list.

> **Status:** this is the semantic/conformance model from the public-domain
> regulation. The numeric limits and enumerations below are vendorable verbatim.
> The wire syntax is NOT here (it is not in §79.102).

---

## Paragraph map (§79.102 (a)–(t))

| ¶ | Topic |
|---|---|
| (a) | Applicability + compliance dates (DTV receivers, tuners, converter boxes) |
| (b) | General requirement: decode captions delivered per **EIA-708-B** (Digital Television Closed Captioning) |
| (c) | The **six** standard caption services (Caption Service #1 … #6) |
| (d) | Code-space organisation + character-set support (C0/C1/G0/G1/G2) |
| (e) | Screen coordinates / display grid (anchor resolution) |
| (f) | Caption window display + text handling |
| (g) | Window text painting + justification |
| (h) | Window colours + borders |
| (i) | Predefined window + pen styles |
| (j) | Pen size requirements (standard / large / small) |
| (k) | Font-style support (8 required fonts) |
| (l) | Character offsetting (subscript/superscript) — optional |
| (m) | Pen styles (normal / italic / underline minimum) |
| (n) | Foreground colour + opacity |
| (o) | Background colour + opacity |
| (p) | Character edges |
| (q) | Colour representation + mapping |
| (r) | Character rendition considerations |
| (s) | Service synchronisation + buffer requirements |
| (t) | Viewer settings + persistence |

---

## (b) Baseline standard

A conforming decoder **must be capable of decoding closed-captioning information
delivered pursuant to EIA-708-B: _Digital Television (DTV) Closed Captioning_.**
(EIA-708-B is the FCC-incorporated edition; the current commercial revision is
ANSI/CTA-708-E. The bit-level syntax is in that standard, not in §79.102.)

## (c) Caption services

Decoders **must decode and process data for the six standard services, Caption
Service #1 through Caption Service #6.** Higher-numbered "extended" services are
not mandated by the regulation.

## (d) Code-space support

A conforming decoder must support, **in their entirety**, the following CEA-708
code spaces:

| Code space | Mandate |
|---|---|
| **C0** (0x00–0x1F) | Required in entirety (miscellaneous control codes) |
| **G0** (0x20–0x7F) | Required in entirety (ASCII-derived character set) |
| **C1** (0x80–0x9F) | Required in entirety (caption control codes) |
| **G1** (0xA0–0xFF) | Required in entirety (Latin-1 character set) |
| **G2** (extended) | Required for the **specific characters listed below**, with substitution for the rest |
| **C2, C3, G3** | **Optional** (extended control / extended character sets) |

### G2 character support + substitution

The decoder must support the following G2 characters (the regulation enumerates a
minimum set), and for any G2 character it does not render, must apply the
specified **substitution** to a supported G0/G1 glyph:

| G2 character (class) | Required / substitution |
|---|---|
| Transparent space | Required (rendered as a space that does not overwrite background) |
| Non-breaking transparent space | Required |
| Solid block | Required |
| Trademark symbol (™) | Required |
| Open / close single + double quotation marks | Substitute the G0 ASCII quotation equivalents `'` / `"` |
| Fraction characters (⅛ ¼ ½ ¾ …) | Substitute `%` (percent sign) |
| Horizontal / vertical border + corner / box-drawing characters | Substitute dashes (`-`) or strokes (`|`) as appropriate |
| Latin-1 / accented additions | Required where listed; otherwise substitute the nearest unaccented G0 letter |

> The exact G2 code-point → glyph map and the complete substitution table are in
> CEA-708-E (the regulation enumerates the *requirement*, the standard gives the
> per-byte table). See Gaps.

---

## (e) Screen coordinates / anchor resolution

Positioning is **coordinate-based** (a continuous grid), not a discrete anchor-ID
table. Providers author against the *maximum* resolution; a decoder rendering at
the *minimum* resolution **divides the provided horizontal and vertical screen
coordinates by 5** (the minimum equals a 1/5 reduction of the maximum grid).

| Aspect ratio | Minimum grid (v × h) | Maximum grid (v × h) |
|---|---|---|
| **4:3** | 15 × 32 | 75 × 160 |
| **16:9** | 15 × 42 | 75 × 210 |

- The **minimum grid** (15v × 32h for 4:3; 15v × 42h for 16:9) covers the entire
  **safe-title area**.
- A decoder must support **at least 4 rows** of caption text displayed
  simultaneously (regulation caps the simultaneously-displayed rows at 4 for the
  baseline requirement).

## (f)/(g) Window display + text painting

The decoder must support the window text model: text **justification**
(left / center / right / full), **print direction** (left-to-right, right-to-left,
top-to-bottom, bottom-to-top), **scroll direction**, **word-wrap**, and the
**display effects** (snap / fade / wipe) used to present a window.

## (h)/(i) Window colours, borders + predefined styles

- Window **fill colour + opacity** and **border type + colour** must be supported.
- Border types include **none / raised / depressed / uniform / shadow-left /
  shadow-right** (the CEA-708 border-type enumeration).
- The decoder must implement the **predefined window styles (1–7)** and
  **predefined pen styles (1–7)** so that a service selecting a preset gets the
  standard rendering (the preset tables — e.g. NTSC pop-on, roll-up, centered —
  are defined in CEA-708-E).

---

## (j) Pen size

Three sizes are required: **standard, large, small**. Dimensions are expressed
relative to the **safe-title area**:

| Size | Constraint |
|---|---|
| **Standard** | Tallest character ≤ **1/15** of safe-title-area height; widest character ≤ **1/32** of safe-title-area width (4:3) or **1/42** (16:9) |
| **Large** | Widest character no wider than **1/32** of the safe-title area (16:9) — i.e. proportionally larger than standard |
| **Small** | Required, proportionally smaller than standard (regulation requires support; exact ratio per CEA-708-E) |

The 1/15 (vertical) and 1/32 ÷ 1/42 (horizontal) ratios match the **minimum grid**
(15v × 32h / 15v × 42h), so a standard pen cell maps one grid cell.

## (k) Font styles — 8 required

A conforming decoder must support **eight** font styles (mapping the CEA-708
`font_tag`):

| # | Font style |
|---|---|
| 0 | Default / undefined |
| 1 | Monospaced **with** serifs |
| 2 | Proportionally spaced **with** serifs |
| 3 | Monospaced **without** serifs |
| 4 | Proportionally spaced **without** serifs |
| 5 | Casual |
| 6 | Cursive |
| 7 | Small capitals |

## (l) Character offsetting — optional

Subscript / normal / superscript offsetting is **optional**.

## (m) Pen styles — text tags

At minimum the decoder must support **normal, italic, and underline**. (The
CEA-708 pen attribute also carries the font tag, pen size, offset, edge type and
colours covered by the other paragraphs.)

---

## (n)/(o) Foreground + background colour and opacity

Both foreground (pen) and background must support the four **opacity** options:

| Opacity | Meaning |
|---|---|
| **Solid** | Fully opaque |
| **Translucent** | Partially transparent (background shows through) |
| **Transparent** | Fully transparent (not rendered) |
| **Flashing** | Alternates between opaque and transparent |

Decoders must let viewers **choose among the colour / opacity options**, and a
viewer's choice overrides the provider's specification (see (t)).

## (p) Character edges

The decoder must support **separate edge colour and edge type** control. The
CEA-708 edge-type enumeration is:

| Edge type |
|---|
| None |
| Raised |
| Depressed |
| Uniform (outline) |
| Left drop shadow |
| Right drop shadow |

## (q) Colour representation + mapping

Each colour component (R, G, B) is a 2-bit value (0–3). A conforming decoder must
support **at least one** of two palettes, with deterministic mapping of
unsupported component values:

### Minimum 8-colour palette

Black, white, red, green, blue, yellow, magenta, cyan — built from RGB components
of **0 or 2 only**. Mapping of received component values onto the 8-colour set:

| Received component | Maps to |
|---|---|
| 0 | 0 |
| 1 | 0 |
| 2 | 2 |
| 3 | 2 |

### Alternative 22-colour palette

A richer set adding gray, bright variants and dark variants, using component
values **0–3**. Unsupported intermediate values map per the regulation's
finer-grained algorithm (component values are quantised toward the nearest
supported level rather than collapsed to 0/2).

> A decoder is conformant if it implements **either** palette with the stated
> mapping; the 8-colour palette is the floor.

## (r) Character rendition

General rendition considerations (anti-aliasing, the relationship between pen
size, font and the safe-title grid) — the regulation requires legible rendering
consistent with the above tables; it does not impose a specific rasteriser.

## (s) Service synchronisation + buffers

The decoder must keep caption services **synchronised** with the program and
provide sufficient **buffering** to handle the DTVCC data rate without dropping
caption commands. (The DTVCC byte budget — 9 600 bit/s, tied to `cc_count` per
frame — is set by the carriage layer; see Table B.9.)

## (t) Viewer settings + persistence

- The decoder **must** offer a setting that displays captions **as intended by the
  caption provider** (the default).
- The decoder **must** allow a viewer's chosen appearance settings to **persist
  until the viewer changes them — including across power cycles** (when the TV is
  turned off and on again).

---

## Gaps — needs CTA-708-E

§79.102 gives the decoder **requirements + the 708 semantic model** above, but does
**NOT** reproduce the bit-level wire syntax or the exact byte-value tables. The
following are **awaiting ANSI/CTA-708-E** (copyright CTA; free-to-download but
cart-gated; **not yet vendored**) and must NOT be guessed from the regulation:

### DTVCC transport / framing (CTA-708-E)
- **DTVCC packet header** layout — `sequence_number`, `packet_size_code`, and how
  packet size maps to byte count.
- **Service block header** layout — `service_number` (1–6 standard; 7 = extended-
  service escape) and `block_size`; the standard/extended service-number encoding.
- How service blocks tile into a DTVCC packet; the null-service / padding rules.

### Command code-space byte encodings (CTA-708-E)
- **C0** (0x00–0x1F) — control-code byte values + parameter lengths (ETX, BS, FF,
  CR, HCR, and the 1-/2-/3-byte command ranges).
- **C1** (0x80–0x9F) — the caption-command opcodes and their parameter formats:
  `CWx` (SetCurrentWindow 0–7), `CLW`/`DSW`/`HDW`/`TGW`/`DLW` (window bitmaps),
  `DLY`/`DLC`/`RST`, `SPA` (SetPenAttributes), `SPC` (SetPenColor), `SPL`
  (SetPenLocation), `SWA` (SetWindowAttributes), `DFx` (DefineWindow 0–7) — exact
  opcode bytes + the bit-packing of each parameter struct.
- **G0 / G1** byte-to-glyph mapping detail (the regulation names the sets; the
  per-code-point glyphs are in the standard).
- **G2** extended-character code points + the **complete substitution table**
  (the regulation lists the *required* characters + substitution *rules*; the
  per-byte map is in CTA-708-E).
- **C2 / C3 / G3** encodings (optional, but needed if implemented).

### Preset tables (CTA-708-E)
- The exact field values behind **predefined window styles 1–7** and **predefined
  pen styles 1–7** (paragraph (i) mandates support; the standard defines each
  preset's attribute values).

### CEA-608 (line-21) — needs CTA-608-E
- The **line-21 control-code + Preamble Address Code (PAC) byte values**, the
  608 colour/style codes, and roll-up / pop-on / paint-on timing — for the
  `cc_type` 0/1 (608) path that `dvb-cc` already demuxes. These are in **ANSI/CTA-
  608-E** (also copyright CTA, not vendored).

Until CTA-608-E / 708-E are vendored, a decode implementation is **grounded** for:
the service model (6 services), the code-space mandate (C0/G0/C1/G1 full, G2
subset + substitution, C2/C3/G3 optional), the window/pen/font/colour/opacity/edge
**enumerations + counts**, the screen-coordinate grid + anchor-resolution limits,
pen-size ratios, safe-title relationship, and viewer-persistence behaviour — and
**blocked** on the per-byte opcode/packet/PAC encodings listed above.
