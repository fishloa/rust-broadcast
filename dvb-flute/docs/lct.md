# Layered Coding Transport (LCT) Header

_Source: RFC 5651 §5 (Figures 1-4), transcribed_

LCT is a transport-level **building block** for reliable content / stream delivery
over IP multicast (and unicast). Every packet sent to an LCT session carries an
**LCT header** of variable size. The header is described by ALC (RFC 5775),
FLUTE (RFC 6726) and NORM (RFC 5740, which uses an analogous-but-distinct header —
see `norm.md`).

All integer fields are big-endian (network order). The LCT version number for this
specification is **1**. Bits marked "padding" or "reserved" (`r`) MUST be set to 0
by senders and ignored by receivers.

## Default LCT Header Format (Figure 1, §5.1)

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|   V   | C |PSI|S| O |H|Res|A|B|   HDR_LEN     | Codepoint (CP)|
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
| Congestion Control Information (CCI, length = 32*(C+1) bits)  |
|                          ...                                  |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|  Transport Session Identifier (TSI, length = 32*S+16*H bits)  |
|                          ...                                  |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|   Transport Object Identifier (TOI, length = 32*O+16*H bits)  |
|                          ...                                  |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                Header Extensions (if applicable)              |
|                          ...                                  |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

### Fixed first 32 bits (4 bytes)

| Field | No. of bits | Mnemonic | Meaning |
|-------|-------------|----------|---------|
| V (version) | 4 | uimsbf | LCT version number. This spec = **1**. |
| C (congestion control flag) | 2 | uimsbf | Length of CCI field = `32*(C+1)` bits (see below). |
| PSI (Protocol-Specific Indication) | 2 | bslbf | Usage defined per protocol instantiation; else sender sets 0, receiver ignores. |
| S (TSI flag) | 1 | uimsbf | Number of full 32-bit words in TSI. |
| O (TOI flag) | 2 | uimsbf | Number of full 32-bit words in TOI. |
| H (half-word flag) | 1 | uimsbf | Adds `16*H` bits to **each** of TSI and TOI. |
| Res (reserved) | 2 | bslbf | MUST be 0 / ignored. |
| A (Close Session flag) | 1 | bslbf | 1 = sender about to stop sending for the session. |
| B (Close Object flag) | 1 | bslbf | 1 = sender about to stop sending for the (TOI-identified) object. |
| HDR_LEN | 8 | uimsbf | Total LCT header length in units of 32-bit words. |
| CP (Codepoint) | 8 | uimsbf | Opaque codec identifier for the payload decoder; mapping out-of-band. |

> Bit-layout note: within the first 16-bit word, the sub-fields are packed
> MSB-first in the order shown in Figure 1: `V`(4) `C`(2) `PSI`(2) `S`(1) `O`(2)
> `H`(1) `Res`(2) `A`(1) `B`(1). That is exactly 16 bits.

### Flag-dependent variable fields — THE critical correctness point

The three fields after the fixed word are variable length, driven entirely by the
`C`, `S`, `O`, and `H` flags:

| Field | Length formula | Allowed lengths |
|-------|----------------|-----------------|
| CCI | `32 * (C + 1)` bits | 32, 64, 96, or 128 bits (`C` = 0,1,2,3) |
| TSI | `32*S + 16*H` bits | 0, 16, 32, or 48 bits |
| TOI | `32*O + 16*H` bits | 0, 16, 32, 48, 64, 80, 96, or 112 bits |

CCI length by `C` (§5.1, "Congestion Control Information"):

| C | CCI length |
|---|------------|
| 0 | 32 bits |
| 1 | 64 bits |
| 2 | 96 bits |
| 3 | 128 bits |

Semantics:
- **CCI** is opaque congestion-control state. MUST be 32 bits if `C`=0, 64 if `C`=1,
  96 if `C`=2, 128 if `C`=3.
- **TSI** = `32*S + 16*H` bits. The half-word (`16*H`) contribution is **shared
  with TOI**: `H` adds 16 bits to both the TSI and TOI fields so that the
  *aggregate* TSI+TOI length is always a multiple of 32 bits. So a TSI with `S`=1,
  `H`=1 is `32*1 + 16*1 = 48` bits. If the underlying transport provides the TSI
  (e.g. the 16-bit UDP source port MAY serve as TSI), it MAY be omitted from the
  header; if there is no underlying TSI it MUST be included.
- **TOI** = `32*O + 16*H` bits. The TOI field is either present in *all* packets of
  a session or *never* present. With `O`=0 and `H`=0 the TOI is absent (length 0).
- ⚠ `H` is a single shared bit feeding BOTH the TSI and TOI length formulas. A
  parser MUST add `16*H` to each independently; it is not "one half-word total".
  The aggregate TSI+TOI length `(32*S + 16*H) + (32*O + 16*H) = 32*(S+O) + 32*H` is
  thus always a whole number of 32-bit words.
- **HDR_LEN** is the authoritative total header length (× 32 bits). The presence of
  Header Extensions is inferred when `HDR_LEN` exceeds the length of the fixed +
  CCI + TSI + TOI portion. Total LCT header (incl. all extensions and optional
  fields) cannot exceed 255 32-bit words.

## Header-Extension Fields (§5.2)

Header extensions occupy the header space beyond the fixed/CCI/TSI/TOI portion, up
to `HDR_LEN`. Each extension begins with an 8-bit **HET** (Header Extension Type).
Two formats exist, chosen by the HET value range (Figure 2):

### Variable-length extension (HET 0..127)

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|  HET (<=127)  |       HEL     |                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+                               +
.                                                               .
.              Header Extension Content (HEC)                   .
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | No. of bits | Mnemonic | Meaning |
|-------|-------------|----------|---------|
| HET | 8 | uimsbf | Extension type, **0..127** for variable-length form. |
| HEL | 8 | uimsbf | Header Extension Length, in 32-bit words = length of the *whole* extension (incl. HET+HEL). Present ONLY for HET 0..127. |
| HEC | `32*HEL − 16` bits | — | Extension content; variable, sized by HEL. |

### Fixed-length extension (HET 128..255)

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|  HET (>=128)  |       Header Extension Content (HEC)          |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | No. of bits | Mnemonic | Meaning |
|-------|-------------|----------|---------|
| HET | 8 | uimsbf | Extension type, **128..255** for fixed-length form. |
| HEC | 24 | — | Fixed-length extension content (exactly one 32-bit word total, incl. HET). NO HEL field. |

Semantics:
- HET 0..127 → variable length; HEL **MUST** be present.
- HET 128..255 → fixed length (one 32-bit word); HEL **MUST NOT** be present.
- Every extension is a multiple of 32 bits. Unrecognized extensions are ignored
  (forward-compatible). Extensions MUST be processed before any congestion-control
  action.

## LCT Header Extension Type registry (§5.2.1, §9.2)

| HET value | Name | Form | Reference / meaning |
|-----------|------|------|---------------------|
| 0 | EXT_NOP | variable | No-Operation. Content ignored by receivers. MUST be supported. |
| 1 | EXT_AUTH | variable | Packet authentication. Format out-of-band. MUST be recognized. |
| 2 | EXT_TIME | variable | Timing info (SCT/ERT/SLC). MUST be recognized. |

IANA allocation ranges for the LCT Header Extension Type namespace (§9.1):

| Range | Form | Allocation policy |
|-------|------|-------------------|
| 0..63 | variable-length | IETF Review |
| 64..127 | variable-length | Specification Required |
| 128..191 | fixed-length | IETF Review |
| 192..255 | fixed-length | Specification Required |

(ALC's EXT_FTI = 64, FLUTE's EXT_FDT = 192 and EXT_CENC = 193, NORM's EXT_FTI = 64 /
EXT_RATE = 128 / EXT_CC = 3 are registered against this namespace — see the
respective docs.)

## EXT_TIME Header Extension (§5.2.2, Figures 3-4)

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|     HET = 2   |    HEL >= 1   |         Use (bit field)       |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                       first time value                        |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
...            (other time values (optional)                  ...
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | No. of bits | Mnemonic | Meaning |
|-------|-------------|----------|---------|
| HET | 8 | uimsbf | = 2 |
| HEL | 8 | uimsbf | ≥ 1; total extension length in 32-bit words. |
| Use | 16 | bitfield | Indicates which time value(s) follow (see below). |
| time value(s) | 32 each | uimsbf | 0 or more, in the order dictated by the Use flags. |

The 16-bit **Use** bit field (Figure 4; bit positions 16..31 of the first word):

```
+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
|SCT|SCT|ERT|SLC|   reserved    |          PI-specific          |
|Hi |Low|   |   |    by LCT     |              use              |
+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+---+
```

| Use sub-field | bits | Meaning |
|---------------|------|---------|
| SCT-High | 1 | Sender Current Time, seconds (32-bit value follows). |
| SCT-Low | 1 | SCT sub-second (1/2^32 s units). If set, SCT-High MUST also be set. |
| ERT | 1 | Expected Residual Time, in seconds (32-bit value follows). |
| SLC | 1 | Session Last Changed time, in seconds (32-bit value follows). |
| reserved by LCT | 4 | MUST be 0 / ignored. |
| PI-specific use | 8 | Out of scope of RFC 5651. |

Semantics:
- When several time values are present they MUST appear in this order: SCT-High,
  then SCT-Low, then ERT, then SLC. Each present flag contributes one 32-bit value.
- SCT-High / SCT-Low / SLC: when NTP is used, these are the MS / LS 32 bits of a
  64-bit NTP timestamp (seconds relative to 1900-01-01 00:00 GMT).
- ERT: number of seconds of expected residual transmission for the current object
  (the TOI object if TOI present, else the session's single object).
- HEL carries the total length so receivers can skip the extension.
