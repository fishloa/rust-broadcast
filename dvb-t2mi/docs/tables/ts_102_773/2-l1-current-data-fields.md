## Table 2 — L1-current data fields
_§5.2.4, PDF p.18_

| Field | Field length (bits) | Format | Description |
|---|---|---|---|
| L1PRE | 168 | bflbf | L1 pre-signalling bits in the order defined in clause 7.2.2 of [1], excluding the CRC. |
| L1CONF_LEN | 16 | uimsbf | Length of L1 configurable signalling in bits |
| L1CONF | 8×⌈L1_CONF_LEN/8⌉ | bflbfzpb | L1 configurable post-signalling fields, in the order defined in clause 7.2.3.1 of [1]. |
| L1DYN_CURR_LEN | 16 | uimsbf | Length of L1-dynamic, current frame. |
| L1DYN_CURR | 8×⌈L1DYN_CURR_LEN/8⌉ | bflbfzpb | L1-post "dynamic, current frame" fields in the order defined in clause 7.2.3.2 of [1]. |
| L1EXT_LEN | 16 | uimsbf | Length of L1 extension field, in bits. |
| L1EXT | 8×⌈L1_EXT_LEN/8⌉ | bflbfzpb | L1-post extension field as defined in clause 7.2.3.4 of [1]. |

> **Note:** The field-length formulas use `L1_CONF_LEN`, `L1DYN_CURR_LEN`, and `L1_EXT_LEN` — with underscores — matching the row field names exactly as printed in the PDF.

### How the L1-current framing maps to EN 302 755 §7.2 (everything the parser needs)

This is the authoritative, self-contained description of how the L1 signalling
is carried inside a T2-MI L1-current (`0x10`) / L1-future (`0x11`) payload, so
the parser need not re-derive it from the EN 302 755 PDF. The L1 field layouts
themselves are transcribed in `dvb-bbframe/docs/en_302_755_t2.md` (Figures
25/27/28).

- **`L1PRE` is the 168 *information* bits of L1-pre, EXCLUDING the 32-bit
  `CRC_32`.** EN 302 755 Figure 25 defines L1-pre as 168 information bits + a
  32-bit CRC = 200 bits; the T2-MI carriage drops the CRC (the T2-MI packet has
  its own CRC-32, validated by the pump). 168 bits = exactly 21 bytes, so
  `L1PRE` is byte-aligned and `L1CONF_LEN` starts at byte 21. A standalone (non
  T2-MI) L1-pre block would carry the CRC; the parser computes/validates that
  CRC with `dvb_common::crc32_mpeg2` (EN 302 755 annex F).
- **`L1CONF` is the configurable L1-post fields (EN 302 755 Figure 27)**,
  bit-packed in wire order, then zero-padded up to a byte boundary
  (`bflbfzpb`). Its bit length is `L1CONF_LEN`. The PLP / RF / AUX loop counts
  come from `NUM_PLP`, `NUM_RF`, `NUM_AUX` (NUM_RF is in L1-pre; NUM_PLP and
  NUM_AUX are the first fields of L1CONF). The FEF block is present iff the
  L1-pre `S2` LSB is 1.
- **`L1DYN_CURR` is the dynamic L1-post fields for the current frame
  (Figure 28)**, same bit-packing+zero-pad rule, length `L1DYN_CURR_LEN`. Its
  PLP loop uses the same `NUM_PLP` (same PLP order as L1CONF); the AUX loop uses
  `NUM_AUX`.
- **`L1EXT` is the L1-post extension blocks (Table 37)**, length `L1EXT_LEN`
  (0 when the L1-pre `L1_POST_EXTENSION` bit is 0).
- **L1-future (Table 3)** carries `L1DYN_NEXT` and (TFS only) `L1DYN_NEXT2`
  — each a dynamic block (Figure 28) with its own 16-bit length prefix — then a
  `NUM_INBAND` loop of `(PLP_ID, INBAND_LEN, INBAND)` in-band signalling blocks.

**Worked validation against `tests/fixtures/colombia-capital-t2mi.ts`** (a
conformant capture): the L1-current payload's 67-byte `l1_current_data` parses
as `L1PRE` (21 bytes; TYPE=TS, S1=T2_SISO, GI=1/8, L1_MOD=16-QAM,
NUM_T2_FRAMES=2, T2_VERSION=0010 (v1.3.1), NUM_RF=1) + `L1CONF_LEN=191`
(=35 header + 35 RF×1 + 89 PLP×1 + 32 post-loop, with NUM_PLP=1, NUM_AUX=0, no
FEF) + 24 bytes L1CONF + `L1DYN_CURR_LEN=127` (=71 header + 48 PLP×1 + 8
RESERVED_3) + 16 bytes L1DYN_CURR + `L1EXT_LEN=0`, consuming exactly 67 bytes.
This reconciliation pins every field width in Figures 25/27/28.

