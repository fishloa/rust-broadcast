# DVB DAS descriptor — `DVB_DAS_descriptor()`

_Source: ETSI TS 103 752-1 V1.2.1 §5.3.5.16, Table 1 + Table 2 (PDF p.17), render-verified_

**NEW binary syntax.** This is a DVB-private SCTE 35 *splice descriptor* (it sits
in the `splice_descriptor()` loop of a base SCTE 35 `splice_info_section()`, like
any other splice descriptor). It exists "for full equivalence between
`splice_insert()` and `segmentation_descriptor()` methods" — i.e. it lets a
`splice_insert()` command optionally carry the placement-opportunity typing and
UPID that a `segmentation_descriptor()` would otherwise convey. It is **optionally
included within a `splice_insert()` command** (§5.3.5.16).

It is a private splice descriptor identified by `splice_descriptor_tag = 0xF0`
(the SCTE 35 private/DVB tag) and `identifier = 0x4456425F` (ASCII `"DVB_"`),
following the standard SCTE 35 private-descriptor framing.

## Table 1 — `DVB_DAS_descriptor()`

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `DVB_DAS_descriptor() {` | | |
| &nbsp;&nbsp;splice_descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;identifier | 32 | uimsbf |
| &nbsp;&nbsp;break_num | 8 | uimsbf |
| &nbsp;&nbsp;breaks_expected | 8 | uimsbf |
| &nbsp;&nbsp;reserved | 4 | uimsbf |
| &nbsp;&nbsp;equivalent_segmentation_type | 4 | bslbf |
| &nbsp;&nbsp;upid | N*8 | uimsbf |
| `}` | | |

⚠ The PDF render lists the bit counts in source order as `8, 8, 32, 8, 8, 4, 4,
N*8`. The `reserved` and `equivalent_segmentation_type` fields are the two 4-bit
nibbles of one byte (reserved is the high nibble, `equivalent_segmentation_type`
the low nibble). The spec's Mnemonic column marks `reserved` as `uimsbf` and
`equivalent_segmentation_type` as `bslbf` — an inversion of the usual convention
(reserved is normally `bslbf`), but reproduced here verbatim from Table 1; the
4-bit widths and parse are unaffected either way. `upid` is the trailing
variable-length field (`N*8`).

## Semantics (§5.3.5.16)

- **splice_descriptor_tag** (8) — defines the syntax for the private bytes that
  make up the body of this descriptor. **Shall be `0xF0`.**
- **descriptor_length** (8) — length, in bytes, of the descriptor following this
  field.
- **identifier** (32) — identifies the owner of the descriptor. **Shall be
  `0x4456425F`** (ASCII `"DVB_"`).
- **break_num** (8) — position of the break within the programme. Set to `0` if
  not being used.
- **breaks_expected** (8) — number of breaks expected within the programme. Set
  to `0` if not being used.
- **reserved** (4) — reserved.
- **equivalent_segmentation_type** (4) — identifies the `segmentation_type` that
  would be used for the equivalent `segmentation_descriptor` in a `time_signal()`
  command. Values per Table 2.
- **upid** (N*8) — variable-length field identifying the specific placement
  opportunity by a Unique Programme Identifier (UPID), in the URI format described
  in §5.3.5.11 (see [`scte35-profiling.md`](scte35-profiling.md)).

## Table 2 — Equivalent Segmentation Type (`equivalent_segmentation_type`)

| Value | Meaning |
|-------|---------|
| `0x0` | no equivalent |
| `0x1` | Distributor Placement Opportunity (DPO) |
| `0x2` | Provider Placement Opportunity (PPO) |
| `0x3` | Distributor Advertisement (DA) |
| `0x4` | Provider Advertisement (PA) |
| `0x5`–`0xF` | reserved for future use |
