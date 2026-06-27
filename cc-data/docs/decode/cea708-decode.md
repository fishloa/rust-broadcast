# CEA-708 (DTVCC) decode wire syntax — ANSI/CTA-708-E S-2023

_Source: ANSI/CTA-708-E S-2023, "Digital Television (DTV) Closed Captioning",
§5–§8, Tables 9–31 (PDF p.21–92), render-verified against the vendored PDF
`specs/cta_708_e_2023.pdf` (copyright CTA; local-only, not committed)._

This document transcribes the **bit-level DTVCC decode syntax** — the layers
**above** the `cc_data()` carriage that `cc-data` already implements (ETSI TS
101 154 Table B.9, see [`../ts_101_154/b9-cc-data.md`](../ts_101_154/b9-cc-data.md)).
It is the missing companion to the public-domain conformance model in
[`cea708-conformance.md`](cea708-conformance.md) (47 CFR §79.102), filling the
"Gaps — needs CTA-708-E" section there: the **DTVCC Caption Channel Packet**,
**Service Block**, the **C0/C1/C2/C3/G0/G1/G2/G3 code spaces**, the **C1 command
opcodes**, and the exact **bit-packing of the DF/SWA/SPA/SPC parameter structs**.

The DTVCC protocol is a 5-layer stack (§3, Table 1, PDF p.6):

| Layer | Defined in | What it frames |
|---|---|---|
| Transport | §4 (cc_data()) | byte-pairs in `cc_data()` — **already in cc-data** |
| Packet | §5 | the **Caption Channel Packet (CCP)** — `sequence_number` + `packet_size_code` + packet data |
| Service | §6 | **Service Blocks** — `service_number` + `block_size` + block data |
| Coding | §7 | code spaces C0/C1/C2/C3, G0/G1/G2/G3; command/character syntactic elements |
| Interpretation | §8 | window/pen model; command semantics + the DF/SWA/SPA/SPC bit-fields |

> **Bitstream convention** (§1.2.4, PDF p.2): the lowest-numbered bit in a
> multi-bit numbered field is the **least significant bit**. `uimsbf` =
> unsigned integer, MSB first. `bslbf` = bit string, left bit first. All
> command-coding subscripts below (`id₂ id₁ id₀`, `av₆…av₀`, …) refer to the
> bit number starting from the lsb.

---

## DTVCC Packet Layer — caption_channel_packet() (§5, PDF p.21–22)

A CCP is `n` bytes of caption data where **n ≤ 128 and n is even** (§5, p.21):
a one-byte header followed by `n−1` bytes of data. The header carries
`sequence_number` and `packet_size_code` (Figure 4, p.21):

```
 b7  b6 │ b5  b4  b3  b2  b1  b0
┌───────┼────────────────────────┐
│ Seq.No│   Packet Size (n/2)     │  CCP Header (byte 0)
├───────┴────────────────────────┤
│   Caption Channel Data Byte 1   │
│   Caption Channel Data Byte 2   │  CCP Data
│              ...                │
│   Caption Channel Data Byte n-1 │
└─────────────────────────────────┘
```

### Table 9 — DTVCC Caption Channel Packet Syntax (PDF p.22)

| Field | No. of bits | Mnemonic |
|---|---|---|
| `caption_channel_packet() {` | | |
| &nbsp;&nbsp;`sequence_number` | 2 | uimsbf |
| &nbsp;&nbsp;`packet_size_code` | 6 | uimsbf |
| &nbsp;&nbsp;`for (i = 0; i < packet_data_size; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`packet_data[i]` | 8 | bslbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

**Field semantics** (§5.1, p.22):

- **`sequence_number`** (bits b7–b6): a 2-bit rolling counter, 0→3, used to
  detect packet loss. On a detected discontinuity the decoder discards any
  partially-accumulated data and performs **Reset** processing for every
  existing service (see §8.9.5). Because there are only four values, packet
  losses that are a multiple of four are undetectable.
- **`packet_size_code`** (bits b5–b0): the number of **byte-pairs** in the CCP
  *including the header byte*.
- **`packet_data_size`** — number of `packet_data` iterations (the data bytes
  following the header):
  - if `packet_size_code == 0` → `packet_data_size = 127`
  - else → `packet_data_size = (packet_size_code * 2) − 1`
- **`packet_data`** — the array of bytes, organised into **Service Blocks** (§6).
  Over-length packets are undefined; decoders may ignore data beyond
  `packet_size_code`.

CCP framing within `cc_data()` (the carriage layer already in cc-data): the
first byte-pair of a CCP is marked `cc_type = 11` (CCP start), continuation
pairs `cc_type = 10`. A CCP ends on either a subsequent CCP header
(`cc_valid=1, cc_type=11`) or after `packet_size_code` bytes are processed
(§4.3.3, p.14). Shortened packets are valid; decoders must **not** reset on a
short CCP (§4.6, p.21). A syntactic element partially received at CCP end is
discarded.

---

## DTVCC Service Layer — Service Blocks (§6, PDF p.23–26)

The Caption Channel is divided into up to **63 logical services** (§6.1, p.23):

- **Service #0**: shall not be used.
- **Service #1**: Primary Caption Service (primary-language verbatim captions).
- **Service #2**: Secondary Language Service.
- **Service #3–#6**: standard services (not pre-assigned).
- **Service #7–#63**: 57 extended services (require the extended header).

Each Service Block is a 1-or-2-byte header followed by 1–31 data bytes
(Figure 6, p.24).

### Table 10 — Service Block Syntax (PDF p.24)

| Field | No. of bits | Mnemonic |
|---|---|---|
| `Service_block() {` | | |
| &nbsp;&nbsp;`service_number` | 3 | uimsbf |
| &nbsp;&nbsp;`block_size` | 5 | uimsbf |
| &nbsp;&nbsp;`if (service_number == b'111' && block_size != 0) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`null_fill` | 2 | `'00'` |
| &nbsp;&nbsp;&nbsp;&nbsp;`extended_service_number` | 6 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`if (service_number != 0) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i = 0; i < block_size; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`Block_data` | 8 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

### Standard Service Block Header — §6.2.1, Figure 7 (PDF p.24)

One byte: 3 high bits = Service Number (`sn₂ sn₁ sn₀`), 5 low bits = Block Size
(`bs₄ bs₃ bs₂ bs₁ bs₀`).

```
 b7   b6   b5  │ b4   b3   b2   b1   b0
┌──────────────┼─────────────────────────┐
│ sn₂  sn₁ sn₀ │ bs₄  bs₃  bs₂  bs₁  bs₀  │
└──────────────┴─────────────────────────┘
```

- `block_size` ranges 1–31 = number of data bytes following the header.
- `block_size = 0` is only valid in a Null Service Block Header.

### Extended Service Block Header — §6.2.2, Figure 8 (PDF p.25)

For services 7–63 a **2-byte** header is used. The first byte has the standard
format with the 3 high bits (`sn₂–sn₀`) **fixed to `111`** (value 7), signalling
the extended form. The second byte's low 6 bits carry the
`extended_service_number` (`sn₅…sn₀`, range 7–63); its top 2 bits are
`null_fill = '00'`. The block size is in the low 5 bits of the **first** byte.

```
            b7   b6   b5  │ b4   b3   b2   b1   b0
byte 0 :  ┌──────────────┼─────────────────────────┐
          │  1    1    1  │ bs₄  bs₃  bs₂  bs₁  bs₀  │   (sn = 7 escape)
          └──────────────┴─────────────────────────┘
            b7   b6   b5   b4   b3   b2   b1   b0
byte 1 :  ┌────────┬──────────────────────────────┐
          │ 0   0  │ sn₅  sn₄  sn₃  sn₂  sn₁  sn₀   │   extended_service_number
          └────────┴──────────────────────────────┘
```

Extended service numbers < 7 are not permitted (they use the standard header).

### Null Service Block Header — §6.2.3, Figure 9 (PDF p.25)

All eight bits = 0 (`service_number == 0` and `block_size == 0`). Signals that
there are no more Service Blocks for the decoder to process in this CCP. A Null
Service Block Header shall be inserted as the last Service Block in the CCP if
space permits; encoders should null-fill (zero) the CCP buffer first.

```
 b7  b6  b5  b4  b3  b2  b1  b0
┌──────────────────────────────┐
│  0   0   0   0   0   0   0   0 │
└──────────────────────────────┘
```

**Tiling rules** (§6.2.5, p.25): Service Blocks are time-division multiplexed
sequentially into CCPs and **shall not cross CCP boundaries**. If a service's
data exceeds the current CCP, it is truncated to fit and the remainder placed
in a **new Service Block (with its own header)** in a subsequent CCP. More than
one Service Block for the same service may appear in one CCP.

---

## Coding Layer — Code Spaces (§7, PDF p.27–41)

The 256-position code space is split into four code groups (§7.1, Table 11,
p.27–28); each group has a standard and an extended set:

| Group | Range | Standard set | Extended set |
|---|---|---|---|
| **CL** | 0x00–0x1F | C0 (Miscellaneous Control) | C2 (Extended Control 1) |
| **GL** | 0x20–0x7F | G0 (ASCII printable) | G2 (Extended Misc. chars) |
| **CR** | 0x80–0x9F | C1 (Caption Command Control) | C3 (Extended Control 2) |
| **GR** | 0xA0–0xFF | G1 (ISO 8859-1 Latin-1) | G3 (future chars / icons) |

**Extension via EXT1** (§7.1.1, p.29): the extended sets (C2, C3, G2, G3) are
accessed by prefixing a code with **`EXT1` = 0x10** (a C0 code). `EXT1` is only
active for the two-byte (or longer) extended sequence it begins. Example: the
G3 closed-caption icon = `0x10, 0xA0`.

**Unused codes** (§7.1.2, p.30): syntactic elements whose first byte is `EXT1`
with an undefined/reserved second byte, or an undefined first byte, are
reserved. Decoders skip them per their size class (1/2/3/variable byte) so
unknown future extensions are ignored cleanly.

### Code Set C0 — Miscellaneous Control Codes (§7.1.4, Table 13, PDF p.30)

C0 = 0x00–0x1F. Three length classes (§7.1.4.1–.3, p.30–31):

- **0x00–0x0F**: one-byte syntactic elements.
- **0x10–0x17**: two-byte. (`0x10 = EXT1` may be 2+ bytes.)
- **0x18–0x1F**: three-byte. (Only `0x18 = P16` is currently specified.)

Defined C0 codes:

| Code | Mnemonic | Bytes | Meaning |
|---|---|---|---|
| 0x00 | **NUL** | 1 | Null (ignored) |
| 0x03 | **ETX** | 1 | End of Text — terminates a caption text segment when not followed by a C1 command (§8.10.4) |
| 0x08 | **BS** | 1 | Back Space — moves entry point back one position |
| 0x0C | **FF** | 1 | Form Feed — erases all window text and moves cursor to (0,0); equivalent to ClearWindows + SetPenLocation(0,0) |
| 0x0D | **CR** | 1 | Carriage Return — moves entry point to start of next row; rolls window up if next row is below the visible window (NOT the code-group CR) |
| 0x0E | **HCR** | 1 | Horizontal Carriage Return — moves entry point to start of current row and erases all text on that row |
| 0x10 | **EXT1** | 2+ | Code-space extension prefix (selects C2/C3/G2/G3) |
| 0x18 | **P16** | 3 | Sets 16-bit character addressing; the two following bytes address a 16-bit code set |

Undefined codes: 0x00–0x0F skipped (1 byte); 0x11–0x17 skipped with the
following byte (2 bytes); 0x18–0x1F skipped with the following two bytes
(3 bytes) (§7.1.4, p.30–31).

### Code Set C1 — Caption Command Control Codes (§7.1.5, Table 14, PDF p.32)

C1 = 0x80–0x9F. The full opcode map (Table 14, p.32):

| Opcode | Cmd | | Opcode | Cmd |
|---|---|---|---|---|
| 0x80 | CW0 | | 0x90 | SPA |
| 0x81 | CW1 | | 0x91 | SPC |
| 0x82 | CW2 | | 0x92 | SPL |
| 0x83 | CW3 | | 0x93 | *reserved* |
| 0x84 | CW4 | | 0x94 | *reserved* |
| 0x85 | CW5 | | 0x95 | *reserved* |
| 0x86 | CW6 | | 0x96 | *reserved* |
| 0x87 | CW7 | | 0x97 | SWA |
| 0x88 | CLW | | 0x98 | DF0 |
| 0x89 | DSW | | 0x99 | DF1 |
| 0x8A | HDW | | 0x9A | DF2 |
| 0x8B | TGW | | 0x9B | DF3 |
| 0x8C | DLW | | 0x9C | DF4 |
| 0x8D | DLY | | 0x9D | DF5 |
| 0x8E | DLC | | 0x9E | DF6 |
| 0x8F | RST | | 0x9F | DF7 |

Undefined window command codes **0x93–0x96** are one-byte reserved commands
(§7.1.5.1, p.32). The exact byte-count and parameter packing of each command is
in the [C1 command reference](#c1-command-reference--8105-pdf-p6383) below.

### Code Set G0 — ASCII Printable Characters (§7.1.6, Table 15, PDF p.33)

G0 = 0x20–0x7F. Standard ASCII printable set **with one substitution**: the
character at **0x7F is the musical-note (eighth-note ♪) glyph**, not ASCII DEL.
Every G0 code is a single-byte syntactic element. The underline character
**0x5F** is the prescribed substitute for unsupported G3 graphic symbols
(see §7.1.9).

### Code Set G1 — ISO 8859-1 Latin-1 (§7.1.7, Table 16, PDF p.34)

G1 = 0xA0–0xFF, the full ISO 8859-1 Latin-1 / Windows-ANSI set. Code
**0xA0 = NBS** (non-breaking space); since `wordwrap = 0`, NBS renders the same
as a normal space.

### Code Set G2 — Extended Miscellaneous Characters (§7.1.8, Table 17, PDF p.35)

Accessed as `EXT1 (0x10)` + a base code in 0x20–0x7F (two-byte element).
Notable G2 code points:

| Code | Char | Meaning |
|---|---|---|
| 0x20 | **TSP** | Transparent Space — no fg/bg; window fill shows through |
| 0x21 | **NBTSP** | Non-Breaking Transparent Space (= TSP since wordwrap=0) |
| 0x25 | … | Horizontal ellipsis |
| 0x2A | Š | Latin S-caron |
| 0x2C | Œ | Latin OE ligature |
| 0x30 | ■ | Solid block (fills the cell with the text foreground color) |
| 0x31 | ‘ | Open single quote |
| 0x32 | ’ | Close single quote |
| 0x33 | " | Open double quote |
| 0x34 | " | Close double quote |
| 0x35 | • | Bold bullet |
| 0x39 | ™ | Trademark symbol |
| 0x3A | š | Latin s-caron (small) |
| 0x3C | œ | Latin oe ligature (small) |
| 0x3D | SM | Service Mark symbol |
| 0x3F | Ÿ | Latin Y-diaeresis (capital) |
| 0x76–0x79 | ⅛ ⅜ ⅝ ⅞ | Fraction characters |
| 0x7A–0x7F | ⎢ ⎤ ⎣ — ⎦ ⎡ | Box/border-drawing characters |

**Undefined G2 codes** (§7.1.8.1, p.35): display a space (0x20) or underline
(0x5F). The full substitution mapping for non-rendered G2 chars is **Table 28**
(see [G2 substitution](#g2-character-substitution--table-28-pdf-p87)).

### Code Set G3 — Future Expansion (§7.1.9, Table 18, PDF p.36)

Accessed as `EXT1 (0x10)` + a base code in 0xA0–0xFF (two-byte element).
Currently the **single** code **0xA0 = the closed-caption [CC] icon**; the rest
is reserved. Unsupported G3 symbols substitute the G0 underscore (0x5F).

### Code Set C2 — Extended Control Code Set 1 (§7.1.10, Table 19, PDF p.37)

Reserved for future control/command codes. Accessed as `EXT1 (0x10)` + base
code in 0x00–0x1F. Length is implied by the base code so non-implementing
decoders can skip (Table 20, p.38):

| Base code | Total element | Skip sequence |
|---|---|---|
| 0x00–0x07 | 2 bytes (0 data) | `EXT1, ExtCode` |
| 0x08–0x0F | 3 bytes (1 data) | `EXT1, ExtCode, <data1>` |
| 0x10–0x17 | 4 bytes (2 data) | `EXT1, ExtCode, <data1>, <data2>` |
| 0x18–0x1F | 5 bytes (3 data) | `EXT1, ExtCode, <data1>, <data2>, <data3>` |

### Code Set C3 — Extended Control Code Set 2 (§7.1.11, Table 21, PDF p.38–41)

Reserved. Accessed as `EXT1 (0x10)` + base code in 0x80–0x9F.

**Fixed-length** codes 0x80–0x8F (§7.1.11.1, Table 22, p.38–39):

| Base code | Total element | Skip sequence |
|---|---|---|
| 0x80–0x87 | 6 bytes (4 data) | `EXT1, ExtCode, <data1..4>` |
| 0x88–0x8F | 7 bytes (5 data) | `EXT1, ExtCode, <data1..5>` |

**Variable-length** codes 0x90–0x9F (§7.1.11.2, Table 23, p.40–41): each has a
1-byte header after the command code — a 2-bit **Type** field (b7–b6), a fixed
`'0'` (b5), and a 5-bit **Length** field (b4–b0). Type values:

| Type (b7:b6) | Meaning |
|---|---|
| 00 | Beginning of Command (BOC) |
| 01 | Continuation of Command (COC) |
| 10 | End of Command (EOC) |
| 11 | One-segment Command (OSC) |

Length ranges 0–27 = data bytes following the header. To skip:
`N = (data1 & 0x3F) + 1` bytes after the header (Table 23, p.41). When
decoding, the Type field is ignored for skipping; bytes are skipped on Length
alone.

---

## C1 command reference — §8.10.5 (PDF p.63–83)

Below, every command's opcode and exact parameter bit-packing. Subscripts are
bit numbers from the lsb; `parm1`, `parm2`, … are successive parameter bytes.

### SetCurrentWindow — CW0…CW7 (§8.10.5.1, PDF p.64)

**Opcode** `0x80 + id`, id = 0–7. One byte, no parameters.

```
 b7  b6  b5  b4  b3 │ b2   b1   b0
┌───────────────────┼───────────────┐
│  1   0   0   0   0 │ id₂  id₁  id₀ │  command
└───────────────────┴───────────────┘
```

Directs subsequent SWA/SPA/SPC/SPL and caption text to window `id` (which must
already exist via DefineWindow).

### DefineWindow — DF0…DF7 (§8.10.5.2, PDF p.65–67)

**Opcode** `0x98 + id`, id = 0–7. **6 parameter bytes** (7 bytes total).

```
            b7    b6    b5  │ b4    b3  │ b2    b1    b0
command : ┌────────────────┴───────────┼──────────────────┐
          │  1    0    0     1     1    │ id₂   id₁   id₀   │  0x98+id
parm1   : │  0    0    v   │  rl  │ cl  │ p₂    p₁    p₀    │
parm2   : │  rp │ av₆  av₅  av₄  av₃  av₂  av₁  av₀          │
parm3   : │  ah₇ ah₆  ah₅  ah₄  ah₃  ah₂  ah₁  ah₀          │
parm4   : │  ap₃ ap₂  ap₁  ap₀ │ rc₃   rc₂   rc₁   rc₀       │
parm5   : │  0    0  │ cc₅  cc₄  cc₃  cc₂  cc₁  cc₀          │
parm6   : │  0    0  │ ws₂  ws₁  ws₀ │ ps₂   ps₁   ps₀       │
          └─────────────────────────────────────────────────┘
```

Exact bit assignment (Command Coding table, p.66):

| Byte | Bits | Field | Meaning |
|---|---|---|---|
| cmd | b7–b3 = `10011` | — | DF opcode high bits |
| cmd | b2–b0 | `id` (window ID) | 0–7 |
| parm1 | b7–b6 | (reserved `00`) | |
| parm1 | b5 | `v` (visible) | 1=YES, 0=NO |
| parm1 | b4 | `rl` (row lock) | 1=YES, 0=NO |
| parm1 | b3 | `cl` (column lock) | 1=YES, 0=NO |
| parm1 | b2–b0 | `p` (priority) | 0–7 (0 = highest) |
| parm2 | b7 | `rp` (relative positioning) | 1 = av/ah are percentages |
| parm2 | b6–b0 | `av` (anchor vertical) | 0–74 (abs, 16:9 & 4:3) or 0–99 (rp=1) |
| parm3 | b7–b0 | `ah` (anchor horizontal) | 0–209 (16:9 abs), 0–159 (4:3 abs), or 0–99 (rp=1) |
| parm4 | b7–b4 | `ap` (anchor point) | 0–8 |
| parm4 | b3–b0 | `rc` (row count) | virtual rows − 1, 0–11 (rc=2 → 3 rows) |
| parm5 | b7–b6 | (reserved `00`) | |
| parm5 | b5–b0 | `cc` (column count) | virtual cols − 1, 0–31 (4:3) / 0–41 (16:9) |
| parm6 | b7–b6 | (reserved `00`) | |
| parm6 | b5–b3 | `ws` (window style ID) | 0 = auto (style 1 on create; no change on update); 1–7 = preset (Table 26) |
| parm6 | b2–b0 | `ps` (pen style ID) | 0 = auto (pen style 1 on create); 1–7 = preset (Table 27) |

Notes: `priority` 0 = highest (on top). `anchor point` 0–8 = which of the
window's 9 anchor positions the av/ah point addresses (see anchor diagram
Figure 13). On create, the window fill colour is applied and pen location set
to (0,0). DefineWindow makes the defined window current. An update with
identical parameters is ignored. On an *update* (resize/move) of an existing
window, pen location and pen attributes are unaffected. The window style and pen
style presets preload attributes that may later be overridden by SWA/SPA/SPC.

### ClearWindows — CLW (§8.10.5.3, PDF p.70)

**Opcode** `0x88`. **1 parameter byte** = 8-bit window bitmap.

```
            b7   b6   b5   b4   b3   b2   b1   b0
command : │  1    0    0    0    1    0    0    0 │  0x88
parm1   : │  w₇   w₆   w₅   w₄   w₃   w₂   w₁   w₀ │  window map
```

`window map` (w): bit `n` set ⇒ window ID `n` is affected. Clearing a window
fills it with the window fill colour and moves its pen position to (0,0).

### DisplayWindows — DSW (§8.10.5.5, PDF p.72)

**Opcode** `0x89`. **1 parameter byte** = window bitmap (same `w₇…w₀` layout as
CLW). Makes the mapped, existing windows visible (does not change current
window ID).

### HideWindows — HDW (§8.10.5.6, PDF p.73)

**Opcode** `0x8A`. **1 parameter byte** = window bitmap. Removes mapped windows
from the display; definitions, current window ID and text are retained; state →
hidden.

### ToggleWindows — TGW (§8.10.5.7, PDF p.74)

**Opcode** `0x8B`. **1 parameter byte** = window bitmap. Each mapped window
toggles its display/hide status.

### DeleteWindows — DLW (§8.10.5.4, PDF p.71)

**Opcode** `0x8C`. **1 parameter byte** = window bitmap. Deletes the mapped
window definitions. If the current window is deleted, current window ID becomes
"unknown" and must be re-set via SetCurrentWindow or DefineWindow.

> All five window-map commands (CLW 0x88, DSW 0x89, HDW 0x8A, TGW 0x8B,
> DLW 0x8C) share one parameter byte: an 8-bit map where bit position `n`
> (b`n`) addresses window ID `n`; 1 = act on it, 0 = leave alone.

### Delay — DLY (§8.10.5.12, PDF p.81)

**Opcode** `0x8D`. **1 parameter byte** = `tenths of seconds`.

```
            b7   b6   b5   b4   b3   b2   b1   b0
command : │  1    0    0    0    1    1    0    1 │  0x8D
parm1   : │  t₇   t₆   t₅   t₄   t₃   t₂   t₁   t₀ │  tenths of seconds
```

Suspends interpretation of the current service's command buffer for `t/10`
seconds. `t` = 1–255 → 0.1–25.5 s. Ends on: timeout, a DelayCancel, the input
buffer filling, or a service Reset.

### DelayCancel — DLC (§8.10.5.13, PDF p.82)

**Opcode** `0x8E`. **No parameters** (1 byte). Terminates any active Delay.

```
            b7   b6   b5   b4   b3   b2   b1   b0
command : │  1    0    0    0    1    1    1    0 │  0x8E
```

### Reset — RST (§8.10.5.14, PDF p.83)

**Opcode** `0x8F`. **No parameters** (1 byte). Re-initialises the service for
which it is received.

```
            b7   b6   b5   b4   b3   b2   b1   b0
command : │  1    0    0    0    1    1    1    1 │  0x8F
```

### SetPenAttributes — SPA (§8.10.5.9, PDF p.77–78)

**Opcode** `0x90`. **2 parameter bytes**.

```
            b7   b6  │ b5   b4  │ b3   b2  │ b1   b0
command : │  1    0     0    1     0    0     0    0 │  0x90
parm1   : │ tt₃  tt₂   tt₁  tt₀ │ o₁   o₀  │ s₁   s₀ │
parm2   : │  i  │ u  │ et₂  et₁  et₀ │ fs₂  fs₁  fs₀  │
```

| Byte | Bits | Field | Values |
|---|---|---|---|
| parm1 | b7–b4 | `tt` (text tag) | 0–15 (see text-tag table below) |
| parm1 | b3–b2 | `o` (offset) | 0=SUBSCRIPT, 1=NORMAL, 2=SUPERSCRIPT |
| parm1 | b1–b0 | `s` (pen size) | 0=SMALL, 1=STANDARD, 2=LARGE |
| parm2 | b7 | `i` (italics) | 1=YES, 0=NO |
| parm2 | b6 | `u` (underline) | 1=YES, 0=NO |
| parm2 | b5–b3 | `et` (edge type) | 0=NONE, 1=RAISED, 2=DEPRESSED, 3=UNIFORM, 4=LEFT_DROP_SHADOW, 5=RIGHT_DROP_SHADOW |
| parm2 | b2–b0 | `fs` (font style) | 0–7 (see font-style table below) |

**Font style** (`fs`, §8.5.3 / §8.10.5.9, p.77):

| Value | Font |
|---|---|
| 0 | Default (undefined) |
| 1 | Monospaced with serifs |
| 2 | Proportionally spaced with serifs |
| 3 | Monospaced without serifs |
| 4 | Proportionally spaced without serifs |
| 5 | Casual |
| 6 | Cursive |
| 7 | Small capitals |

**Text tag** (`tt`, §8.5.9 / §8.10.5.9, p.77):

| Value | Tag | | Value | Tag |
|---|---|---|---|---|
| 0 | Dialog | | 8 | Song lyrics |
| 1 | Source or speaker ID | | 9 | Sound effect description |
| 2 | Electronically reproduced voice | | 10 | Musical score description |
| 3 | Dialog in non-primary language | | 11 | Expletive |
| 4 | Voiceover | | 12–14 | (undefined) |
| 5 | Audible translation | | 15 | Text not to be displayed |
| 6 | Subtitle translation | | | |
| 7 | Voice quality description | | | |

If `text tag ≠ 0` it takes priority and the other SPA attributes are ignored
(§8.10.5.9 example, p.78).

### SetPenColor — SPC (§8.10.5.10, PDF p.79)

**Opcode** `0x91`. **3 parameter bytes**. Each colour is 2 bits per RGB
component (see [colour representation](#colour-representation--88-tables-3031-pdf-p9192)).

```
            b7   b6  │ b5   b4  │ b3   b2  │ b1   b0
command : │  1    0     0    1     0    0     0    1 │  0x91
parm1   : │ fo₁  fo₀ │ fr₁  fr₀ │ fg₁  fg₀ │ fb₁  fb₀ │  fg opacity + fg color
parm2   : │ bo₁  bo₀ │ br₁  br₀ │ bg₁  bg₀ │ bb₁  bb₀ │  bg opacity + bg color
parm3   : │  0    0  │ er₁  er₀ │ eg₁  eg₀ │ eb₁  eb₀ │  edge color
```

| Byte | Bits | Field | Values |
|---|---|---|---|
| parm1 | b7–b6 | `fo` (fg opacity) | 0=SOLID, 1=FLASH, 2=TRANSLUCENT, 3=TRANSPARENT |
| parm1 | b5–b4 | `fr` (fg red) | 0–3 |
| parm1 | b3–b2 | `fg` (fg green) | 0–3 |
| parm1 | b1–b0 | `fb` (fg blue) | 0–3 |
| parm2 | b7–b6 | `bo` (bg opacity) | 0=SOLID, 1=FLASH, 2=TRANSLUCENT, 3=TRANSPARENT |
| parm2 | b5–b4 | `br` (bg red) | 0–3 |
| parm2 | b3–b2 | `bg` (bg green) | 0–3 |
| parm2 | b1–b0 | `bb` (bg blue) | 0–3 |
| parm3 | b7–b6 | (reserved `00`) | |
| parm3 | b5–b4 | `er` (edge red) | 0–3 |
| parm3 | b3–b2 | `eg` (edge green) | 0–3 |
| parm3 | b1–b0 | `eb` (edge blue) | 0–3 |

Character edges have the same opacity as `fg opacity` (no separate edge opacity).

### SetPenLocation — SPL (§8.10.5.11, PDF p.80)

**Opcode** `0x92`. **2 parameter bytes**.

```
            b7   b6  │ b5   b4   b3 │ b2   b1   b0
command : │  1    0     0    1    0    0    1    0 │  0x92
parm1   : │  0    0     0    0  │ r₃   r₂   r₁   r₀ │  row (b3–b0)
parm2   : │  0    0  │ c₅   c₄   c₃   c₂   c₁   c₀   │  column (b5–b0)
```

| Byte | Bits | Field | Values |
|---|---|---|---|
| parm1 | b7–b4 | (reserved `0000`) | |
| parm1 | b3–b0 | `row` (r) | 0–14 |
| parm2 | b7–b6 | (reserved `00`) | |
| parm2 | b5–b0 | `column` (c) | 0–31 (4:3) / 0–41 (16:9) |

Repositions the pen cursor in the current window. Row=0 is the top row,
column=0 the leftmost; not affected by print direction or justification.
If justification ≠ LEFT, the row/column behaviour is governed by the print
direction (§8.10.5.11, p.80).

### SetWindowAttributes — SWA (§8.10.5.8, PDF p.75–76)

**Opcode** `0x97`. **4 parameter bytes**.

```
            b7   b6  │ b5   b4  │ b3   b2  │ b1   b0
command : │  1    0     0    1     0    1     1    1 │  0x97
parm1   : │ fo₁  fo₀ │ fr₁  fr₀ │ fg₁  fg₀ │ fb₁  fb₀ │  fill opacity + fill color
parm2   : │ bt₁  bt₀ │ br₁  br₀ │ bg₁  bg₀ │ bb₁  bb₀ │  border type(lo) + border color
parm3   : │ bt₂ │ ww │ pd₁  pd₀ │ sd₁  sd₀ │ j₁   j₀  │
parm4   : │ es₃  es₂  es₁  es₀ │ ed₁  ed₀ │ de₁  de₀  │
```

| Byte | Bits | Field | Values |
|---|---|---|---|
| parm1 | b7–b6 | `fo` (fill opacity) | 0=SOLID, 1=FLASH, 2=TRANSLUCENT, 3=TRANSPARENT |
| parm1 | b5–b4 | `fr` (fill red) | 0–3 |
| parm1 | b3–b2 | `fg` (fill green) | 0–3 |
| parm1 | b1–b0 | `fb` (fill blue) | 0–3 |
| parm2 | b7–b6 | `bt₁ bt₀` (border type, low 2 bits) | combined with bt₂ below |
| parm2 | b5–b4 | `br` (border red) | 0–3 |
| parm2 | b3–b2 | `bg` (border green) | 0–3 |
| parm2 | b1–b0 | `bb` (border blue) | 0–3 |
| parm3 | b7 | `bt₂` (border type, high bit) | 3-bit `border type` = `bt₂ bt₁ bt₀` |
| parm3 | b6 | `ww` (wordwrap) | 1=YES, 0=NO (must be NO per §9.8) |
| parm3 | b5–b4 | `pd` (print direction) | 0=LEFT_TO_RIGHT, 1=RIGHT_TO_LEFT, 2=TOP_TO_BOTTOM, 3=BOTTOM_TO_TOP |
| parm3 | b3–b2 | `sd` (scroll direction) | 0=LEFT_TO_RIGHT, 1=RIGHT_TO_LEFT, 2=TOP_TO_BOTTOM, 3=BOTTOM_TO_TOP |
| parm3 | b1–b0 | `j` (justify) | 0=LEFT, 1=RIGHT, 2=CENTER, 3=FULL |
| parm4 | b7–b4 | `es` (effect speed) | 0–15, ×0.5 s (1 → 0.5 s … 15 → 7.5 s) |
| parm4 | b3–b2 | `ed` (effect direction) | 0=LEFT_TO_RIGHT, 1=RIGHT_TO_LEFT, 2=TOP_TO_BOTTOM, 3=BOTTOM_TO_TOP |
| parm4 | b1–b0 | `de` (display effect) | 0=SNAP, 1=FADE, 2=WIPE |

> ⚠ **border type bit-split** (PDF p.76, Figure in §8.10.5.8): the
> `border type` is a **3-bit** value `bt₂ bt₁ bt₀` split across two bytes —
> its low two bits `bt₁ bt₀` are parm2 b7–b6 and its high bit `bt₂` is parm3
> b7. The Command Coding figure shows parm2 starting with `bt₁ bt₀` and parm3
> starting with `bt₂`. Values: 0=NONE, 1=RAISED, 2=DEPRESSED, 3=UNIFORM,
> 4=SHADOW_LEFT, 5=SHADOW_RIGHT (§8.10.5.8 parameter list, p.75).

Display effects: SNAP = window pops on/off; FADE = fades on/off at `effect
speed`; WIPE = swipes on/off at `effect speed` in `effect direction` (wipe-off
goes the opposite direction). Minimum conformance requires SNAP; if FADE/WIPE
unimplemented, all windows snap and `effect speed` is ignored (§9.9.6, p.90).

---

## Supporting value tables

### Predefined Window Styles — Table 26 (PDF p.68)

`window style ID` (ws) in DefineWindow selects one of these presets:

| ID | Justify | Print dir | Scroll dir | WordWrap | Display effect | Fill color | Fill opacity | Border type | Usage |
|---|---|---|---|---|---|---|---|---|---|
| 1 | LEFT | L→R | BOTTOM→TOP | NO | SNAP | (0,0,0) Black | SOLID | NONE | NTSC PopUp |
| 2 | LEFT | L→R | BOTTOM→TOP | NO | SNAP | n/a | TRANSPARENT | NONE | PopUp w/o black bg |
| 3 | CNTR | L→R | BOTTOM→TOP | NO | SNAP | (0,0,0) Black | SOLID | NONE | NTSC centered PopUp |
| 4 | LEFT | L→R | BOTTOM→TOP | YES | SNAP | (0,0,0) Black | SOLID | NONE | NTSC RollUp |
| 5 | LEFT | L→R | BOTTOM→TOP | YES | SNAP | n/a | TRANSPARENT | NONE | RollUp w/o black bg |
| 6 | CNTR | L→R | BOTTOM→TOP | YES | SNAP | (0,0,0) Black | SOLID | NONE | NTSC centered RollUp |
| 7 | LEFT | TOP→BOTTOM | RIGHT→LEFT | NO | SNAP | (0,0,0) Black | SOLID | NONE | Ticker tape |

### Predefined Pen Styles — Table 27 (PDF p.69)

`pen style ID` (ps) in DefineWindow selects one of these presets:

| ID | Pen size | Font | Offset | Italics | Underline | Edge type | FG color | FG opacity | BG color | BG opacity | Edge color | Usage |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | STNDR | 0 | NORMAL | NO | NO | NONE | (2,2,2) White | SOLID | (0,0,0) Black | SOLID | n/a | Default NTSC |
| 2 | STNDR | 1 | NORMAL | NO | NO | NONE | (2,2,2) White | SOLID | (0,0,0) Black | SOLID | n/a | NTSC Mono w/ serif |
| 3 | STNDR | 2 | NORMAL | NO | NO | NONE | (2,2,2) White | SOLID | (0,0,0) Black | SOLID | n/a | NTSC Prop w/ serif |
| 4 | STNDR | 3 | NORMAL | NO | NO | NONE | (2,2,2) White | SOLID | (0,0,0) Black | SOLID | n/a | NTSC Mono w/o serif |
| 5 | STNDR | 4 | NORMAL | NO | NO | NONE | (2,2,2) White | SOLID | (0,0,0) Black | SOLID | n/a | NTSC Prop w/o serif |
| 6 | STNDR | 3 | NORMAL | NO | NO | UNIFRM | (2,2,2) White | SOLID | n/a | TRANSPARENT | (0,0,0) Black | Mono w/o serif, bordered |
| 7 | STNDR | 4 | NORMAL | NO | NO | UNIFRM | (2,2,2) White | SOLID | n/a | TRANSPARENT | (0,0,0) Black | Prop w/o serif, bordered |

### Cursor movement after drawing / on CR — Table 24 (PDF p.50)

For `justify = LEFT` (other combinations are not permitted):

| Print direction | Scroll direction | Cursor movement | Carriage-return behavior |
|---|---|---|---|
| L→R | TOP→BOTTOM | increment column | decrement row, column=0 |
| L→R | BOTTOM→TOP | increment column | increment row, column=0 |
| R→L | TOP→BOTTOM | decrement column | decrement row, column=max |
| R→L | BOTTOM→TOP | decrement column | increment row, column=max |
| TOP→BOTTOM | L→R | increment row | decrement column, row=0 |
| TOP→BOTTOM | R→L | increment row | increment column, row=0 |
| BOTTOM→TOP | L→R | decrement row | decrement column, row=max |
| BOTTOM→TOP | R→L | decrement row | increment column, row=max |

### Colour representation — §8.8, Tables 30/31 (PDF p.91–92)

Every colour in SPC/SWA is **2 bits per RGB component** (R,G,B each 0–3), so 64
possible colours. A decoder must support at least the **8-colour minimum list**
(Table 30) or the **22-colour alternative list** (Table 31).

**Minimum 8-colour list (Table 30):**

| Color | R | G | B |
|---|---|---|---|
| Black | 0 | 0 | 0 |
| White | 2 | 2 | 2 |
| Red | 2 | 0 | 0 |
| Green | 0 | 2 | 0 |
| Blue | 0 | 0 | 2 |
| Yellow | 2 | 2 | 0 |
| Magenta | 2 | 0 | 2 |
| Cyan | 0 | 2 | 2 |

Mapping a received RGB onto the 8-colour list (§8.8, p.92): component value
**1 → 0**, **2 → 2 (unchanged)**, **3 → 2**. (E.g. (1,2,3) → (0,2,2);
(3,3,3) → (2,2,2); (1,1,1) → (0,0,0).)

The 22-colour list (Table 31) uses components 0–3 with a finer per-component
mapping algorithm (§8.8 a/b, p.92). Decoders supporting neither list must
support all 64 RGB combinations.

### Screen coordinate resolutions & limits — Table 29 (PDF p.87)

`anchor vertical`/`anchor horizontal` in DefineWindow are authored against the
**maximum** grid; a decoder at the minimum resolution divides the provided
coordinates by 5.

| Aspect | Max anchor resolution (v × h) | Min anchor resolution (v × h) | Max displayed rows | Max chars per row |
|---|---|---|---|---|
| 4:3 | 75 × 160 | 15 × 32 | 4 | 32 |
| 16:9 | 75 × 210 | 15 × 42 | 4 | 42 |
| other | 75 × (5×H) | 15 × H* | 4 | * |

\*H = 32 × (screen width relative to 4:3); e.g. 16:9 → H = 32 × 4/3 ≈ 42.

### G2 character substitution — Table 28 (PDF p.87)

When a G2 character cannot be rendered, substitute as follows:

| G2 char | G2 code | Substitute (G0/G1) |
|---|---|---|
| open single quote ‘ | 0x31 | G0 single quote `'` (0x27) |
| close single quote ’ | 0x32 | G0 single quote `'` (0x27) |
| open double quote " | 0x33 | G0 double quote `"` (0x22) |
| close double quote " | 0x34 | G0 double quote `"` (0x22) |
| bold bullet • | 0x35 | G1 bullet · (0xB7) |
| ellipsis … | 0x25 | G0 underscore `_` (0x5F) |
| ⅛ | 0x76 | G0 percent `%` (0x25) |
| ⅜ | 0x77 | G0 percent `%` (0x25) |
| ⅝ | 0x78 | G0 percent `%` (0x25) |
| ⅞ | 0x79 | G0 percent `%` (0x25) |
| vertical border ⎢ | 0x7A | G0 stroke `\|` (0x7C) |
| upper-right border ⎤ | 0x7B | G0 dash `-` (0x2D) |
| lower-left border ⎣ | 0x7C | G0 dash `-` (0x2D) |
| horizontal border — | 0x7D | G0 dash `-` (0x2D) |
| lower-right border ⎦ | 0x7E | G0 dash `-` (0x2D) |
| upper-left border ⎡ | 0x7F | G0 dash `-` (0x2D) |

All unsupported C2/C3/G3 codes are skipped by their implied byte count; all
unsupported G3 graphic symbols substitute the G0 underscore (0x5F).

---

## Ambiguity / verification notes

- ⚠ **SWA `border type` 3-bit split** (PDF p.76): the bit-field figure splits
  `border type` as low-2-bits in parm2 (b7:b6) + high-bit in parm3 (b7). This
  is rendered clearly in the Command Coding diagram but is easy to mis-pack —
  flagged for the implementer. Candidate alternative readings were checked
  against the SWA example (`0x97,0x64,0x53,0x88,0x22` → border type = 5 =
  SHADOW_RIGHT, p.76): parm3 = 0x88 = `1000 1000`, so bt₂ (b7) = 1; parm2 =
  0x53 = `0101 0011`, so bt₁ bt₀ (b7:b6) = 01 → `101`b = 5. ✓ confirms the
  split as transcribed.
- ⚠ **SPA pen size `s` range**: §8.10.5.9 enumerates `[SMALL, STANDARD, LARGE]
  == [0, 1, 2]` (2-bit field, value 3 reserved/undefined).
- The DefineWindow example (`0x9A,0x38,0x4A,0xD1,0x8B,0x0F,0x11`, p.66–67) was
  used to cross-check the parm1–6 packing (window id=2, visible=YES,
  rl=YES, cl=YES, priority=0, rp=0, av=0x4A=74, ah=0xD1=209, ap=8, rc=11,
  cc=15, ws=2, ps=1) — all consistent with the bit assignment above. ✓
- The `0x7F` G0 musical-note substitution (replacing ASCII DEL) is confirmed in
  Table 15 (p.33) and the §7.1.6 prose.
