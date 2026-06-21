# CEA-608 line-21 caption DECODE byte-pair semantics — ANSI/CTA-608-E

_Source: ANSI/CTA-608-E S-2019 ("Line 21 Data Services", April 2008 reaffirmation),
§5.3, §6, §7, §8, Annex B, and the consolidated FCC tables in Annex F.1.1
(Tables 49–53, PDF p.96–101) — copyright CTA, local-only (`specs/cta_608_e_2019.pdf`,
gitignored). Render-verified against the vendored PDF; tables transcribed from the
PDF render, not pdftotext._

This document captures the **DECODE byte-pair semantics** of the EIA/CEA-608 line-21
data service — the meaning of the two-byte control / character codes that `dvb-cc`
already demuxes out of `cc_data()` (`cc_type` 0 = field 1, `cc_type` 1 = field 2;
the digital 608 byte pairs — see [`../ts_101_154/b9-cc-data.md`](../ts_101_154/b9-cc-data.md)).
It folds **into the existing `dvb-cc` crate** — it is **not** a separate crate.

It deliberately **skips** the analog line-21 VBI waveform / clock-run-in / IRE
modulation chapters (§5.2, Figure 2, Table 2): `dvb-cc` is fed the digital
`cc_data_1`/`cc_data_2` bytes, not the analog signal. Only the decode-relevant
byte-pair structure, code tables and command semantics are reproduced here.

> The `dvb-cc` crate is `no_std` and ships typed CEA-608/708 triplets; this doc is
> the spec grounding for interpreting the 608 (`cc_type` 0/1) bytes. No Rust here.

---

## 1. The two-byte structure (§3.2.2 "Character", §5.3, §8.3)

Every line-21 datum is a pair of bytes. Each byte is **7 data bits + 1 odd-parity
bit** in the most-significant bit (b7); the data is "standard ASCII 7-bit plus ODD
parity character codes" (§5.3 / §8.3). A decoder **strips parity** (masks off b7)
to recover the 7-bit value `0x00`–`0x7F`, optionally validating odd parity first.
A character is defined as "a single group of 7 data bits plus a parity symbol"
(§3.2.2). _All hex values in the tables below are the **7-bit, parity-stripped**
values._

Classification of a (stripped) pair, where `b1` is the first byte and `b2` the
second:

| Pair shape (7-bit) | Meaning |
|---|---|
| `b1` in `0x00`–`0x0F` | **(field 2 only) XDS control character** — begins an XDS sub-packet (§8.4 c / §8.6). Never used by caption/Text. |
| `b1` in `0x10`–`0x1F` | **Control code** (PAC / mid-row / misc-control / special / extended-char / tab — see below). The exact `0x1n` value selects the *channel* and the *category*. |
| `b1` in `0x20`–`0x7F` | **Displayable characters**: one or two standard characters (Table 50). `b2` (if `0x20`–`0x7F`) is a second standard character; `b2 = 0x00` is the null padding for a single trailing char. |

A 608 frame that is all-null is the **filler** pattern `0x80 0x80` (i.e. `0x00 0x00`
with parity) — two null characters, transmitted when there is nothing to send.

⚠ The spec does not enumerate a single "is this a control pair" predicate; decoders
conventionally treat `0x10`–`0x1F` in the first byte as the control-code class and
everything `≥0x20` as displayable. The `0x00`–`0x0F` first-byte range is XDS and
appears **only on field 2** (§8.4 c).

### Doubling / repetition (Annex B.14, §8.3)

Caption and Text **control-code** pairs (first byte `0x10`–`0x1F`) are normally
transmitted **twice in a row** for error resilience; the decoder acts on the first
correctly-received copy and **discards an immediately-following identical copy**
(Annex B.14 "Double Control-Byte Pairs"). Displayable-character pairs are **not**
doubled. **Field 2 XDS control codes shall not be repeated** (§8.6.2). To encode
e.g. three carriage returns you must send three CR pairs (§8.3).

---

## 2. Fields, data channels and service mapping (§4.1, Table 1, p.10)

Line 21 carries two independent data streams: **field 1** (`cc_type` 0) and
**field 2** (`cc_type` 1). Within each field, the **control-code first byte** selects
one of two **data channels** (C1/C2). The combination of field + data channel names
the logical service:

_Table 1 — Field 1 and Field 2 Packets (§4.1, PDF p.10)_

| Name | Field | Data channel | Description |
|---|---|---|---|
| CC1 | 1 | C1 | Primary synchronous caption service |
| CC2 | 1 | C2 | Special non-synchronous-use captions |
| T1  | 1 | C1 | First Text service |
| T2  | 1 | C2 | Second Text service |
| CC3 | 2 | C1 | Secondary synchronous caption service |
| CC4 | 2 | C2 | Special non-synchronous-use captions |
| T3  | 2 | C1 | Third Text service |
| T4  | 2 | C2 | Fourth Text service |
| XDS | 2 | C3 | eXtended Data Service |

CC1 is the primary (usually English) caption track; CC3 is the secondary (often a
second language). CC2/CC4 are the "special non-synchronous" channels. T1–T4 are Text
Mode. **XDS lives only on field 2** as a third "channel" (C3). The hyphen in a name
is optional (CC-1 = CC1).

### How the data channel is selected (Annex F.1.1, Tables 51–53; §8.4)

The data channel is encoded in **which `0x1n` first byte** carries the control code.
The first-byte set is duplicated: a **C1** set and a **C2** set, and field 2 uses a
distinct set so that field-1 and field-2 codes never alias. The canonical mapping
(per the PAC/mid-row/misc tables and §8.4 a/b) is:

| First-byte (7-bit) | Field | Data channel |
|---|---|---|
| `0x10`–`0x17` | 1 | **C1** (CC1 / T1) |
| `0x18`–`0x1F` | 1 | **C2** (CC2 / T2) |
| `0x18`–`0x1F` | 2 | **C1** (CC3 / T3) — _but see field-2 offsets below_ |
| field-2 misc / special | 2 | per §8.4 a/b offsets |

§8.4 gives the field-2 derivation rule precisely, as a byte-level offset from the
field-1 codes:

- **(a)** Miscellaneous control pairs whose first byte falls in `0x14`, `0x20`–`0x14, 0x2F`
  in field 1 → use **`0x15`, `0x20`–`0x15, 0x2F`** in field 2 (i.e. `0x14`→`0x15`).
- **(b)** Misc control pairs whose first byte falls in `0x1C`, `0x20`–`0x1C, 0x2F`
  in field 1 → use **`0x1D`, `0x20`–`0x1D, 0x2F`** in field 2 (i.e. `0x1C`→`0x1D`).
- **(c)** First bytes `0x01`–`0x0F` begin **XDS** data (field 2 only).

This is why Table 52 lists both a "Data Channel 1" (`0x14 nn`) and a "Data Channel 2"
(`0x1C nn`) column for every misc-control code — see §3 below. ⚠ The full
field-1↔field-2 channel-pairing matrix is given relationally across §8.4 and
Tables 51–53 rather than as one master table; the per-code channel columns in
those tables (transcribed below) are the authoritative byte values.

---

## 3. Miscellaneous control codes (RCL/BAS/DER/RU2-4/FON/RDC/TR/RTD/EDM/CR/ENM/EOC) — Table 52 (§F.1.1.4, p.99)

The pop-on / roll-up / paint-on command set. "Data Channel 1" is the field-1 / C1
byte pair; "Data Channel 2" is the field-1 / C2 (= `0x1C`-prefixed) byte pair. For
field 2, apply the §8.4 offsets above (`0x14`→`0x15`, `0x1C`→`0x1D`).

_Table 52 — Miscellaneous Control Codes (7-bit values)_

| Data ch 1 | Data ch 2 | Mnemonic | Command |
|---|---|---|---|
| `14 20` | `1C 20` | **RCL** | Resume Caption Loading (pop-on: load into non-displayed memory) |
| `14 21` | `1C 21` | **BS**  | Backspace |
| `14 22` | `1C 22` | **AOF** | Reserved (formerly Alarm Off) |
| `14 23` | `1C 23` | **AON** | Reserved (formerly Alarm On) |
| `14 24` | `1C 24` | **DER** | Delete to End of Row |
| `14 25` | `1C 25` | **RU2** | Roll-Up Captions — 2 rows |
| `14 26` | `1C 26` | **RU3** | Roll-Up Captions — 3 rows |
| `14 27` | `1C 27` | **RU4** | Roll-Up Captions — 4 rows |
| `14 28` | `1C 28` | **FON** | Flash On |
| `14 29` | `1C 29` | **RDC** | Resume Direct Captioning (paint-on) |
| `14 2A` | `1C 2A` | **TR**  | Text Restart |
| `14 2B` | `1C 2B` | **RTD** | Resume Text Display |
| `14 2C` | `1C 2C` | **EDM** | Erase Displayed Memory |
| `14 2D` | `1C 2D` | **CR**  | Carriage Return |
| `14 2E` | `1C 2E` | **ENM** | Erase Non-Displayed Memory |
| `14 2F` | `1C 2F` | **EOC** | End of Caption (flip memories — pop-on display) |
| `17 21` | `1F 21` | **TO1** | Tab Offset 1 column |
| `17 22` | `1F 22` | **TO2** | Tab Offset 2 columns |
| `17 23` | `1F 23` | **TO3** | Tab Offset 3 columns |

Mode / command semantics (§3.2.1 acronyms, §6.1, §7, Annex B):

- **RCL** selects **pop-on** style: subsequent characters load into non-displayed
  memory; **EOC** then flips non-displayed↔displayed (the caption "pops on").
- **RU2/RU3/RU4** select **roll-up** style with a 2/3/4-row window; **CR** rolls the
  window up one row and clears the base row. The base row is the bottom row; rows
  roll upward into it (§3.2.2 "Base Row", "Window").
- **RDC** selects **paint-on** style: characters paint directly to the displayed
  memory as received (Annex B.7).
- **EDM** erases displayed memory; **ENM** erases non-displayed memory. A blank
  caption can be made via ENM + EOC (§3.2.2 "Erase Display").
- **DER** erases from the cursor to the end of the row (memory + control codes to its
  right on the same row) (§7.4).
- **BS** moves the cursor one column left, erasing the char/mid-row at that cell
  (Annex B.12 / §7.4).
- **FON** = flash on. **TR** (Text Restart) erases the Text screen and homes the
  cursor; **RTD** (Resume Text Display) initiates / resumes Text Mode (§7.4).
- **TO1/TO2/TO3** = tab offset: advance the cursor 1/2/3 columns within the current
  row, with the same column-positioning effect on either decoder type; in Text Mode
  a tab offset has the same effect as in caption mode (§7.4, Annex B.4).

⚠ AOF/AON (`0x22`/`0x23`) are **reserved** (formerly "Alarm Off/On") — a conforming
decoder ignores them; do not act on them.

---

## 4. Preamble Address Codes (PACs) — Table 53 (§F.1.1.5, p.101)

A PAC is a two-byte control code that sets the **row**, and either an **indent +
white colour** or a **colour/italics + underline** for the text that follows. The
**first byte** selects row-group + data channel/field; the **second byte** selects
the row-within-group + the attribute (colour/underline, or indent/underline).

The table below is the full Table 53. The **first byte** depends on the row; the
**second byte** depends on the attribute. Read it as: pick the row column for the
first byte, then the attribute row for the second byte. All values are 7-bit
(parity-stripped).

### 4a. PAC first byte by row + channel (Table 53, top block)

| Row | Field1/C1 (DataCh1) | Field1/C2·Field2/C1 (DataCh2) |
|---|---|---|
| 1  | `11` | `19` |
| 2  | `11` | `19` |
| 3  | `12` | `1A` |
| 4  | `12` | `1A` |
| 5  | `15` | `1D` |
| 6  | `15` | `1D` |
| 7  | `16` | `1E` |
| 8  | `16` | `1E` |
| 9  | `17` | `1F` |
| 10 | `17` | `1F` |
| 11 | `10` | `18` |
| 12 | `13` | `1B` |
| 13 | `13` | `1B` |
| 14 | `14` | `1C` |
| 15 | `14` | `1C` |

("Data Channel 1" / "Data Channel 2" are the spec's column labels; the C2 / field-2
column is the `+0x08` companion of each Data-Channel-1 byte. Row 11 is the odd one
out — first byte `0x10`/`0x18`.)

### 4b. PAC second byte by attribute (Table 53, bottom block)

The second byte distinguishes two columns per row: **even-row** vs **odd-row** within
the row pair (the spec lists them under each Row1…Row15 column; for a given row, the
second byte is one of the two listed values — the table pairs rows so each row maps
to exactly one of the two second-byte columns). Values:

| Attribute (colour / indent + underline) | 2nd byte (col A) | 2nd byte (col B) |
|---|---|---|
| White                | `40` | `60` |
| White Underline      | `41` | `61` |
| Green                | `42` | `62` |
| Green Underline      | `43` | `63` |
| Blue                 | `44` | `64` |
| Blue Underline       | `45` | `65` |
| Cyan                 | `46` | `66` |
| Cyan Underline       | `47` | `67` |
| Red                  | `48` | `68` |
| Red Underline        | `49` | `69` |
| Yellow               | `4A` | `6A` |
| Yellow Underline     | `4B` | `6B` |
| Magenta              | `4C` | `6C` |
| Magenta Underline    | `4D` | `6D` |
| White Italics        | `4E` | `6E` |
| White Italics Underline | `4F` | `6F` |
| Indent 0             | `50` | `70` |
| Indent 0 Underline   | `51` | `71` |
| Indent 4             | `52` | `72` |
| Indent 4 Underline   | `53` | `73` |
| Indent 8             | `54` | `74` |
| Indent 8 Underline   | `55` | `75` |
| Indent 12            | `56` | `76` |
| Indent 12 Underline  | `57` | `77` |
| Indent 16            | `58` | `78` |
| Indent 16 Underline  | `59` | `79` |
| Indent 20            | `5A` | `7A` |
| Indent 20 Underline  | `5B` | `7B` |
| Indent 24            | `5C` | `7C` |
| Indent 24 Underline  | `5D` | `7D` |
| Indent 28            | `5E` | `7E` |
| Indent 28 Underline  | `5F` | `7F` |

**Column A vs B = the two rows of a row-pair.** In Table 53, each "Row N" column
gives the *second byte*: column A values (`0x40`–`0x5F`) belong to one row of the
pair, column B values (`0x60`–`0x7F`) to the other. Concretely, reading the original
Table 53 columns: rows 1,3,5,7,9,11(=`10`),12,14 use the **`0x40`–`0x5F`** (A)
second-byte; rows 2,4,6,8,10,13,15 use **`0x60`–`0x7F`** (B). ⚠ The spec encodes
row-within-pair purely via this second-byte high-nibble (`0x4_/0x5_` vs `0x6_/0x7_`);
a decoder reconstructs the row from (first-byte row-group, second-byte 0x40-vs-0x60).

**Indent semantics:** indent N = N columns from the left of the 32-column row
(0,4,8,…,28). **Note (Table 53):** *all* indent codes (second byte `0x50`–`0x5F` /
`0x70`–`0x7F`) assign **white** as the colour attribute. The colour/italics codes
(second byte `0x40`–`0x4F` / `0x60`–`0x6F`) set indent 0 implicitly. The
least-significant bit of the second byte = **underline** on/off in every case.

New **mid-screen PACs** (rows usable for mid-screen positioning) are described in
Annex B.2 (TeleCaption I/II decoders). If no PAC follows a CR in roll-up style, the
default new-row position is **Indent 0** (§3.2.2 default a).

---

## 5. Mid-row codes (colour / italics + underline) — Table 51 (§F.1.1.3, p.99)

A mid-row code sets the foreground attribute **from the cursor onward on the current
row**, occupying one cell (displayed as a space on standard decoders). It turns off
italics and flash; the LSB controls underline (§6.2). Same behaviour on both decoder
types (§6.2 final line).

_Table 51 — Mid-Row Codes (7-bit values)_

| Data ch 1 | Data ch 2 | Attribute |
|---|---|---|
| `11 20` | `19 20` | White |
| `11 21` | `19 21` | White Underline |
| `11 22` | `19 22` | Green |
| `11 23` | `19 23` | Green Underline |
| `11 24` | `19 24` | Blue |
| `11 25` | `19 25` | Blue Underline |
| `11 26` | `19 26` | Cyan |
| `11 27` | `19 27` | Cyan Underline |
| `11 28` | `19 28` | Red |
| `11 29` | `19 29` | Red Underline |
| `11 2A` | `19 2A` | Yellow |
| `11 2B` | `19 2B` | Yellow Underline |
| `11 2C` | `19 2C` | Magenta |
| `11 2D` | `19 2D` | Magenta Underline |
| `11 2E` | `19 2E` | Italics |
| `11 2F` | `19 2F` | Italics Underline |

---

## 6. Background / foreground attribute codes — Table 3 (§6.2, p.17)

Optional (extended decoders). Each code incorporates an automatic backspace (BS) for
back-compat: a standard decoder shows the space and ignores the code; an extended
decoder shows a space carrying the new colour/opacity. Background codes set colour +
opacity (opaque / semi-transparent) until end-of-row or the next background code.
**Note (Table 3): "B" as the 2nd letter of a mnemonic means Blue, so Black is "A".**

_Table 3 — Background and Foreground Attribute Codes (7-bit values)_

| Data ch 1 | Data ch 2 | Mnemonic | Description |
|---|---|---|---|
| `10 20` | `18 20` | BWO | Background White, Opaque |
| `10 21` | `18 21` | BWS | Background White, Semi-transparent |
| `10 22` | `18 22` | BGO | Background Green, Opaque |
| `10 23` | `18 23` | BGS | Background Green, Semi-transparent |
| `10 24` | `18 24` | BBO | Background Blue, Opaque |
| `10 25` | `18 25` | BBS | Background Blue, Semi-transparent |
| `10 26` | `18 26` | BCO | Background Cyan, Opaque |
| `10 27` | `18 27` | BCS | Background Cyan, Semi-transparent |
| `10 28` | `18 28` | BRO | Background Red, Opaque |
| `10 29` | `18 29` | BRS | Background Red, Semi-transparent |
| `10 2A` | `18 2A` | BYO | Background Yellow, Opaque |
| `10 2B` | `18 2B` | BYS | Background Yellow, Semi-transparent |
| `10 2C` | `18 2C` | BMO | Background Magenta, Opaque |
| `10 2D` | `18 2D` | BMS | Background Magenta, Semi-transparent |
| `10 2E` | `18 2E` | BAO | Background Black (A), Opaque |
| `10 2F` | `18 2F` | BAS | Background Black (A), Semi-transparent |
| `17 2D` | `1F 2D` | BT  | Background Transparent |
| `17 2E` | `1F 2E` | FA  | Foreground Black |
| `17 2F` | `1F 2F` | FAU | Foreground Black Underline |

---

## 7. Special characters — Table 49 (§F.1.1.1, p.96)

Two-byte. Each is preceded by **`0x11`** for data channel 1 (or **`0x19`** for data
channel 2); the second byte is in `0x30`–`0x3F`. E.g. `0x19 0x37` = musical note on
data channel 2. (Field 2: the first byte uses the §8.4 channel offset.)

_Table 49 — Special Characters (2nd byte; Example, Alternate)_

| 2nd byte | Symbol | Alternate | Description |
|---|---|---|---|
| `30` | ® | (note) | Registered mark symbol |
| `31` | ° |   | Degree sign |
| `32` | ½ |   | One-half |
| `33` | ¿ |   | Inverse query (inverted question mark) |
| `34` | ™ | (note) | Trademark symbol |
| `35` | ¢ |   | Cents sign |
| `36` | £ |   | Pounds Sterling sign |
| `37` | ♪ |   | Music note |
| `38` | à | A | Lower-case a, grave accent |
| `39` | (space) |   | **Transparent space** |
| `3A` | è | E | Lower-case e, grave accent |
| `3B` | â | A | Lower-case a, circumflex |
| `3C` | ê | E | Lower-case e, circumflex |
| `3D` | î | I | Lower-case i, circumflex |
| `3E` | ô | O | Lower-case o, circumflex |
| `3F` | û | U | Lower-case u, circumflex |

(The "Alternate" column = the fallback glyph an upper-case-only decoder substitutes.)

---

## 8. Standard character set — Table 50 (§F.1.1.2 / §6.4.1, p.97–98)

The basic North-American set. **One byte each** (same code in either data channel),
`0x20`–`0x7F`, after parity strip. It is ASCII with 608-specific substitutions:
several ASCII slots carry accented Latin letters and symbols (e.g. `0x2A` = á,
`0x5C` = é, `0x5E`–`0x60` = í/ó/ú, `0x7B` = ç, `0x7C` = ÷, `0x7D`/`0x7E` = Ñ/ñ,
`0x7F` = solid block ■). `0x7C` (÷) and `0x7F` (■) replace the ASCII `|` and DEL.

_Table 50 — Standard Characters (7-bit value → glyph; Alternate = upper-case fallback)_

| Hex | Glyph | Alt | Description |
|---|---|---|---|
| `20` | (space) |  | Standard space |
| `21` | ! |  | Exclamation mark |
| `22` | " |  | Quotation mark |
| `23` | # |  | Pounds (number) sign |
| `24` | $ |  | Dollar sign |
| `25` | % |  | Percentage sign |
| `26` | & |  | Ampersand |
| `27` | ' |  | Apostrophe |
| `28` | ( |  | Open parenthesis |
| `29` | ) |  | Close parenthesis |
| `2A` | á | A | Lower-case a, acute accent |
| `2B` | + |  | Plus sign |
| `2C` | , |  | Comma |
| `2D` | - |  | Minus (hyphen) sign |
| `2E` | . |  | Period |
| `2F` | / |  | Slash |
| `30`–`39` | 0–9 |  | Numerals zero…nine |
| `3A` | : |  | Colon |
| `3B` | ; |  | Semicolon |
| `3C` | < |  | Less-than sign |
| `3D` | = |  | Equal sign |
| `3E` | > |  | Greater-than sign |
| `3F` | ? |  | Question mark |
| `40` | @ |  | At sign |
| `41`–`5A` | A–Z |  | Upper-case alphabet |
| `5B` | [ |  | Open bracket |
| `5C` | é | E | Lower-case e, acute accent |
| `5D` | ] |  | Close bracket |
| `5E` | í | I | Lower-case i, acute accent |
| `5F` | ó | O | Lower-case o, acute accent |
| `60` | ú | U | Lower-case u, acute accent |
| `61`–`7A` | a–z |  | Lower-case alphabet |
| `7B` | ç | C | Lower-case c, cedilla |
| `7C` | ÷ |  | Division sign |
| `7D` | Ñ |  | Upper-case N, tilde |
| `7E` | ñ | N | Lower-case n, tilde |
| `7F` | ■ |  | Solid block |

(Per §6.4.1 this is the 112-char "basic set": upper/lower alphabet, the accented
letters á à â ç é è ê í ñ Ñ ó ô ú û, punctuation, numerals, music note, standard
space, transparent space, solid block. ⚠ The single-byte set above only covers the
`0x20`–`0x7F` half; à/è/â/ê/î/ô/û and the music note are the *Special* chars in
Table 49, §7. **NOTE (§6.4.1):** the ¼/¾ glyphs of TeleCaption I were replaced by
the ® and ™ symbols.)

---

## 9. Extended Western-European character sets — Tables 5–10 (§6.4.2, p.20–25)

Optional. Up to 64 extra accented/symbol characters for correct typography in
Spanish, French, Portuguese, German, Danish (plus Italian/Finnish/Swedish if all 64
present). **Two bytes each:** the first byte is **`0x12`** (data channel 1) or
**`0x13`** for the second 16/32-char block, with **`0x1A`/`0x1B`** the data-channel-2
equivalents; the second byte is `0x20`–`0x3F` (§6.4.2 final paragraph).

Each extended char carries an **automatic BS**: on receipt the cursor moves one
column left (erasing the standard fallback char the provider sent first), then the
extended glyph is displayed. So a provider sends e.g. `u` then the `ü` extended code;
a basic decoder keeps `u` and ignores the code, an extended decoder shows `ü`. ⚠ An
extended char cannot occupy the 32nd column (32-char-per-row limit, §6.4.2 NOTE2).

### Two first-byte blocks

| Block | DataCh1 first byte | DataCh2 first byte | 2nd-byte range | Tables |
|---|---|---|---|---|
| Block 1 (Spanish + Misc + French) | `0x12` | `0x1A` | `0x20`–`0x3F` | 5, 6, 7 |
| Block 2 (Portuguese + German + Danish) | `0x13` | `0x1B` | `0x20`–`0x3F` | 8, 9, 10 |

_Table 5 — Spanish (first byte `12`/`1A`)_

| DataCh1 | DataCh2 | Symbol | Description |
|---|---|---|---|
| `12 20` | `1A 20` | Á | Capital A, acute |
| `12 21` | `1A 21` | É | Capital E, acute |
| `12 22` | `1A 22` | Ó | Capital O, acute |
| `12 23` | `1A 23` | Ú | Capital U, acute |
| `12 24` | `1A 24` | Ü | Capital U, diaeresis/umlaut |
| `12 25` | `1A 25` | ü | small u, diaeresis/umlaut |
| `12 26` | `1A 26` | ‘ | opening single quote |
| `12 27` | `1A 27` | ¡ | inverted exclamation mark |

_Table 6 — Miscellaneous (first byte `12`/`1A`)_

| DataCh1 | DataCh2 | Symbol | Description |
|---|---|---|---|
| `12 28` | `1A 28` | * | Asterisk |
| `12 29` | `1A 29` | ' | plain (non-curled) single quote |
| `12 2A` | `1A 2A` | — | em dash |
| `12 2B` | `1A 2B` | © | Copyright |
| `12 2C` | `1A 2C` | ℠ | Service mark |
| `12 2D` | `1A 2D` | • | round bullet |
| `12 2E` | `1A 2E` | “ | opening double quotes |
| `12 2F` | `1A 2F` | ” | closing double quotes |

_Table 7 — French (first byte `12`/`1A`)_

| DataCh1 | DataCh2 | Symbol | Description |
|---|---|---|---|
| `12 30` | `1A 30` | À | Capital A, grave |
| `12 31` | `1A 31` | Â | Capital A, circumflex |
| `12 32` | `1A 32` | Ç | Capital C, cedilla |
| `12 33` | `1A 33` | È | Capital E, grave |
| `12 34` | `1A 34` | Ê | Capital E, circumflex |
| `12 35` | `1A 35` | Ë | Capital E, diaeresis/umlaut |
| `12 36` | `1A 36` | ë | small e, diaeresis/umlaut |
| `12 37` | `1A 37` | Î | Capital I, circumflex |
| `12 38` | `1A 38` | Ï | Capital I, diaeresis/umlaut |
| `12 39` | `1A 39` | ï | small i, diaeresis/umlaut |
| `12 3A` | `1A 3A` | Ô | Capital O, circumflex |
| `12 3B` | `1A 3B` | Ù | Capital U, grave |
| `12 3C` | `1A 3C` | ù | small u, grave |
| `12 3D` | `1A 3D` | Û | Capital U, circumflex |
| `12 3E` | `1A 3E` | « | opening guillemets |
| `12 3F` | `1A 3F` | » | closing guillemets |

_Table 8 — Portuguese (first byte `13`/`1B`)_

| DataCh1 | DataCh2 | Symbol | Description |
|---|---|---|---|
| `13 20` | `1B 20` | Ã | Capital A, tilde |
| `13 21` | `1B 21` | ã | small a, tilde |
| `13 22` | `1B 22` | Í | Capital I, acute |
| `13 23` | `1B 23` | Ì | Capital I, grave |
| `13 24` | `1B 24` | ì | small i, grave |
| `13 25` | `1B 25` | Ò | Capital O, grave |
| `13 26` | `1B 26` | ò | small o, grave |
| `13 27` | `1B 27` | Õ | Capital O, tilde |
| `13 28` | `1B 28` | õ | small o, tilde |
| `13 29` | `1B 29` | { | opening brace |
| `13 2A` | `1B 2A` | } | closing brace |
| `13 2B` | `1B 2B` | \ | backslash |
| `13 2C` | `1B 2C` | ^ | caret |
| `13 2D` | `1B 2D` | _ | underbar |
| `13 2E` | `1B 2E` | \| | pipe |
| `13 2F` | `1B 2F` | ~ | tilde |

_Table 9 — German (first byte `13`/`1B`)_

| DataCh1 | DataCh2 | Symbol | Description |
|---|---|---|---|
| `13 30` | `1B 30` | Ä | Capital A, diaeresis/umlaut |
| `13 31` | `1B 31` | ä | small a, diaeresis/umlaut |
| `13 32` | `1B 32` | Ö | Capital O, diaeresis/umlaut |
| `13 33` | `1B 33` | ö | small o, diaeresis/umlaut |
| `13 34` | `1B 34` | ß | eszett (small sharp s) |
| `13 35` | `1B 35` | ¥ | yen |
| `13 36` | `1B 36` | ¤ | non-specific currency sign |
| `13 37` | `1B 37` | \| | Vertical bar |

_Table 10 — Danish (first byte `13`/`1B`)_

| DataCh1 | DataCh2 | Symbol | Description |
|---|---|---|---|
| `13 38` | `1B 38` | Å | Capital A, ring |
| `13 39` | `1B 39` | å | small a, ring |
| `13 3A` | `1B 3A` | Ø | Capital O, slash |
| `13 3B` | `1B 3B` | ø | small o, slash |
| `13 3C` | `1B 3C` | ⌜ | upper-left corner |
| `13 3D` | `1B 3D` | ⌝ | upper-right corner |
| `13 3E` | `1B 3E` | ⌞ | lower-left corner |
| `13 3F` | `1B 3F` | ⌟ | lower-right corner |

(Order = relative importance to the North-American audience; a partial
implementation should add chars in this order so each language block is self-contained
in a block of 8/16. NOTE1 §6.4.2: the opening single quote `12 26` is the mirror of
the basic-set apostrophe, intended as a curled opening quote.)

### Asian two-byte set selection — Table 4 (§6.3, p.18, Informative)

`0x17 nn` (DataCh1) / `0x1F nn` (DataCh2) select alternate two-byte character sets —
**not FCC-compatible**, not for North-American distribution:

| DataCh1 | DataCh2 | Selects |
|---|---|---|
| `17 24` | `1F 24` | standard line-21 set, **normal** size |
| `17 25` | `1F 25` | standard line-21 set, **double** size |
| `17 26` | `1F 26` | first private character set |
| `17 27` | `1F 27` | second private character set |
| `17 28` | `1F 28` | PRC set: GB 2312-80 |
| `17 29` | `1F 29` | Korean set: KSC 5601-1987 |
| `17 2A` | `1F 2A` | first registered character set |

---

## 10. XDS — eXtended Data Services framing (field 2 only) — §8.6, §9 (p.33–36)

XDS is a third data "channel" carried **only on field 2** (`cc_type` 1). It interleaves
program-/system-metadata packets into the space-available nulls of the field-2 stream.
It is distinguished from caption/Text by its **control characters**, whose first byte
is in `0x01`–`0x0F` — a range never used by caption or Text codes (§8.6.1, §8.6.2).

### XDS character kinds (§8.6.1)

| Kind | Range (first/role) | Role |
|---|---|---|
| **Control** | first byte `0x01`–`0x0F` | mode switch; always the 1st byte of a pair. Begins a sub-packet (Start/Continue) or ends it (End). |
| **Type** | 2nd byte after a Control, `0x01`–`0x7F` | identifies the packet type within the class. |
| **Checksum** | 2nd byte after the End control, `0x00`–`0x7F` | packet integrity check. |
| **Informational** | `0x00`, `0x20`–`0x7F` | the payload chars (`0x01`–`0x1F` forbidden); `0x00` = null placeholder. |

### Packet framing (§8.6.2, §8.6.3, §9.1, §9.2)

- A packet begins with a **Start/Type** control pair. The **odd** control value
  (Start) vs **even** (Continue) of a class share the same number; the control number
  determines the packet **Class** (Current, Future, Channel Info, Miscellaneous,
  Public Service, Reserved, Private — §3.2.2 "Packet Class", 7 classes).
- To resume a suspended packet, a **Continue/Type** pair is sent; its Type must equal
  the Start's Type.
- A packet ends with the single **End** control code **`0x0F`** followed by one
  **Checksum** data byte to complete the pair (§8.6.2).
- **Checksum (§8.6.3):** the 7-bit value such that the sum of the Start char, Type
  char, all Informational chars, and the End + Checksum chars equals zero mod 128
  (two's-complement of the sum of those bytes). Continue/Type pairs are **not** part
  of the checksum.
- **Packet length (§8.6.6):** ≤ 32 Informational characters.
- XDS uses **ODD parity** like the rest of line 21 (§8.6.2). Field-2 XDS control codes
  are **not repeated** (unlike caption/Text doubled controls).
- A packet may be suspended/interrupted by another packet type, or by resuming a
  caption/Text transmission; it is terminated by beginning another packet of the same
  class+type (§8.6.7, §8.6.8).

⚠ The exact per-class control-code byte values and the full XDS Type/Class catalogue
(§9.5 — Current/Future/Channel-Info/Misc/Public-Service classes, Tables 14–44) are
out of scope for the *caption* decode path. `dvb-cc` decodes captions (CC1–CC4 / Text);
XDS is a separate metadata channel. The framing above is what a caption decoder needs
to **detect and skip** XDS sub-packets on field 2 (a `0x01`–`0x0F` first byte = XDS,
hand off / ignore for caption purposes). Transcribe §9 / Tables 14–44 separately if an
XDS metadata decoder is ever required.

---

## Decode quick-reference (first-byte → category)

After stripping parity to 7 bits, for the **first byte** of a pair:

| First byte (7-bit) | Category | Where |
|---|---|---|
| `0x00` | null filler (`0x00 0x00`) | §1 |
| `0x01`–`0x0F` | **XDS** control (field 2 only) | §10 |
| `0x10` / `0x18` | bg/fg attr (`0x10 0x20-2F`); PAC row 11; mid-row uses `0x11` | §4, §6 |
| `0x11` / `0x19` | mid-row codes (`0x_ 0x20-2F`); **special chars** (`0x_ 0x30-3F`); PAC rows 1–2 | §4, §5, §7 |
| `0x12` / `0x1A` | **extended chars** block 1 (`0x_ 0x20-3F`); PAC rows 3–4 | §4, §9 |
| `0x13` / `0x1B` | **extended chars** block 2 (`0x_ 0x20-3F`); PAC rows 12–13 | §4, §9 |
| `0x14` / `0x1C` | **misc control** (RCL…EOC); PAC rows 14–15 | §3, §4 |
| `0x15` / `0x1D` | (field-2 misc-control offset of `0x14`/`0x1C`); PAC rows 5–6 | §3, §4 |
| `0x16` / `0x1E` | PAC rows 7–8 | §4 |
| `0x17` / `0x1F` | **tab offsets** + bg/fg (`0x17 0x21-2F`); Table-4 set-select; PAC rows 9–10 | §3, §6, §9 |
| `0x20`–`0x7F` | **displayable** standard char(s), 1 or 2 per pair | §8 |

⚠ Several first-byte values are overloaded (e.g. `0x11` carries mid-row, special
chars *and* PAC rows): the **second byte's range** disambiguates (`0x20`–`0x2F` =
mid-row/attr, `0x30`–`0x3F` = special char, `0x40`–`0x7F` = PAC). A decoder must
branch on both bytes.
