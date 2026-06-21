# DSM-CC stream-event payload (binary) — `DSM-CC_stream_event_payload_binary()`

_Source: ETSI TS 103 752-1 V1.2.1 §6.3.1, Table 3 + Table 4 (PDF pp.18–20), render-verified_

**NEW binary syntax.** For *distribution* signalling, a full SCTE 35 message
section may be carried inside a **DSM-CC stream event** (rather than directly on a
TS PID). The payload of such a stream event is first built in this binary form,
then **base-64 encoded (IETF RFC 4648 [4])** and inserted as the private data
(`privateDataByte`) of a "do it now" stream-event descriptor per ETSI TS 102 809
[9].

Two carriage modes (selected by `event_type`):
- `event_type == 0` — the SCTE 35 section is conveyed **directly** inline
  (`SCTE_35_section()`).
- `event_type == 1` — the payload **references** a DSM-CC object-carousel file
  (by DVB URI [6]) that contains the SCTE 35 section (used for larger sections).

Section-length limits (§6.3.1): the inline SCTE 35 `section_length` is limited to
**180 bytes** when referencing PTS, or **178 bytes** if a TEMI timeline is used
(max `section_length` field values 177 and 175 respectively). Larger sections must
use the carousel-object reference.

## Table 3 — `DSM-CC_stream_event_payload_binary()`

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `DSM-CC_stream_event_payload_binary() {` | | |
| &nbsp;&nbsp;DVB_data_length | 8 | uimsbf |
| &nbsp;&nbsp;reserved_zero_future_use | 3 | bslbf |
| &nbsp;&nbsp;event_type | 1 | bslbf |
| &nbsp;&nbsp;timeline_type | 4 | uimsbf |
| &nbsp;&nbsp;`if (timeline_type == 0x2) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;temi_component_tag | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;temi_timeline_id | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;reserved_zero_future_use | N*8 ⚠ | bslbf |
| &nbsp;&nbsp;private_data_length | 8 | uimsbf |
| &nbsp;&nbsp;`if (private_data_length > 0) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;private_data_specifier | 32 ⚠ | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<private_data_length-4; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;private_data_byte | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`if (event_type == 1) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;carousel_object_name_length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<carousel_object_name_length; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;char | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`if (event_type == 0) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`SCTE_35_section()` | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

⚠ **OCR-alignment caveats in the render** (PDF p.19) — the "No. of Bits" column
sits beside several syntax lines and the pairing is slightly ambiguous:
- The `reserved_zero_future_use` that precedes `private_data_length` is shown with
  value `N*8`. Read together with the semantics it is a reserved span ("Use of
  these fields may be defined by ETSI in future versions … all bits set to 0").
  Its exact length is not pinned by the table beyond `N*8`; treat as a reserved
  byte-run. The bit values listed in source order after `temi_timeline_id`'s `8`
  are `N*8, 8, 8(private_data_specifier), 32(?), 8`.
- `private_data_specifier` is described as an 8-bit-aligned field assigned per
  ETSI TS 101 162 [5]; the loop guard `private_data_length-4` strongly implies a
  **32-bit** `private_data_specifier` (4 bytes consumed before the byte loop).
  The render's "32" value cell appears against the `private_data_byte` loop row,
  so the 32 belongs to `private_data_specifier`. **⚠ Verify against a clean copy:
  `private_data_specifier` = 32 bits, `private_data_byte` = 8 bits.**

## Semantics (§6.3.1)

- **DVB_data_length** (8) — length, in bytes, of the fields following the
  `DVB_data_length` field up to (and prior to) the `private_data_length` field.
- **reserved_zero_future_use** — reserved; all bits set to `0`; meaning may be
  defined in future versions.
- **event_type** (1) — `1` ⇒ this stream event references a DSM-CC carousel
  object conveying the SCTE 35 section; `0` ⇒ the SCTE 35 section is conveyed
  within this stream event.
- **timeline_type** (4) — identifies the timeline that PTS values in the SCTE 35
  section reference. Values per Table 4.
- **temi_component_tag** (8) — `component_tag` of the referenced TEMI timeline.
  Present only when `timeline_type == 0x2`.
- **temi_timeline_id** (8) — `timeline_id` of the referenced TEMI timeline.
  Present only when `timeline_type == 0x2`.
- **private_data_length** (8) — length in bytes of the following private data.
- **private_data_specifier** — value assignment per ETSI TS 101 162 [5].
- **private_data_byte** (8) — privately defined.
- **carousel_object_name_length** (8) — length of the DVB URI [6] of the carousel
  object containing the SCTE 35 message.
- **char** (8) — a sequence conveying the DVB URI [6] of the DSM-CC carousel
  object.
- **SCTE_35_section** — the entire SCTE 35 `splice_info_section()` from `table_id`
  through `CRC_32` (Table 5 of SCTE 35 [1]).

## Table 4 — Timeline Type (`timeline_type`)

| Value | Meaning |
|-------|---------|
| `0x0` | no timeline used |
| `0x1` | PTS in SCTE 35 message references video PTS |
| `0x2` | PTS in SCTE 35 message references the time in a TEMI timeline associated with the service |
| `0x3`–`0xF` | reserved for future use |

⚠ The render shows the last row label as "`0x2 to 0xF`" for "reserved for future
use", which overlaps the `0x2` value already assigned to TEMI. Read as
**`0x3`–`0xF` reserved** (the `0x2` row is "PTS references TEMI timeline"); the
"0x2" in the reserved-row label is an OCR carry-over from the row above.

## Carriage / encoding (§6.3.1)

1. Build `DSM-CC_stream_event_payload_binary()` (Table 3).
2. **base-64 encode** the whole structure per IETF RFC 4648 [4].
3. Insert the base-64 text as `privateDataByte` of a "do it now" stream-event
   descriptor (ETSI TS 102 809 [9]); transmit immediately.

Per-event payload limit: max **245 bytes** of payload per DSM-CC stream event
(ETSI TS 102 809 [9]); base-64 inflates the binary by 4:3, which is why the
inline `section_length` is capped (180 / 178 bytes — see top of this doc) and
larger sections must go via a carousel object (`event_type == 1`).

(`timeline_type` selection for the two PTS/TEMI conversion paths — §6.3.2 PTS
`= 0x1`, §6.3.3 TEMI `= 0x2`, and §7.2/§7.3 converter steps — is profiling/usage,
not new syntax.)
