# Transport stream packet + adaptation field — ITU-T H.222.0 (08/2023) §2.4.3 = ISO/IEC 13818-1

Transcribed from `specs/itu_t_h222_0_202308_mpeg2_systems.pdf` (pp.48–52), pdf2md
value-verified, then structure-cleaned. Bit-widths, hex values and mnemonics are
verbatim from the spec; control-flow (`if`/`for`) is shown as indented syntax rows
per the spec's tabular convention.

## §2.4.3.1 Transport stream — Table 2-1

```
MPEG_transport_stream() {
    do {
        transport_packet()
    } while (nextbits() == sync_byte)
}
```

## §2.4.3.2 Transport stream packet layer — Table 2-2 (transport_packet)

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| `transport_packet() {` | | |
| &nbsp;&nbsp;sync_byte | 8 | bslbf |
| &nbsp;&nbsp;transport_error_indicator | 1 | bslbf |
| &nbsp;&nbsp;payload_unit_start_indicator | 1 | bslbf |
| &nbsp;&nbsp;transport_priority | 1 | bslbf |
| &nbsp;&nbsp;PID | 13 | uimsbf |
| &nbsp;&nbsp;transport_scrambling_control | 2 | bslbf |
| &nbsp;&nbsp;adaptation_field_control | 2 | bslbf |
| &nbsp;&nbsp;continuity_counter | 4 | uimsbf |
| &nbsp;&nbsp;`if (adaptation_field_control == '10' \|\| adaptation_field_control == '11') {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;adaptation_field() | | |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`if (adaptation_field_control == '01' \|\| adaptation_field_control == '11') {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i = 0; i < N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;data_byte | 8 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

## §2.4.3.3 Semantics — transport stream packet layer

- **sync_byte** — fixed 8-bit field, value `'0100 0111'` (**0x47**). Sync_byte emulation in the choice of values for other regularly occurring fields (e.g. PID) should be avoided.
- **transport_error_indicator** — 1-bit flag. When `'1'`, at least one uncorrectable bit error exists in the packet. May be set by entities external to the transport layer; once `'1'`, shall not be reset to `'0'` unless the erroneous bits have been corrected.
- **payload_unit_start_indicator** — 1-bit flag with normative meaning for packets carrying PES packets (§2.4.3.6) or section data (Table 2-31, §2.4.4.5). For PES payload: `'1'` = this packet's payload commences with the first byte of a PES packet (and exactly one PES packet starts here); `'0'` = no PES packet starts here. For section payload: if the packet carries the first byte of a section, the value shall be `'1'` (first payload byte is the `pointer_field`); else `'0'` (no `pointer_field`). For null packets, shall be `'0'`. Meaning for packets carrying only private data is not defined here.
- **transport_priority** — 1-bit. `'1'` = greater priority than other same-PID packets without the bit set.
- **PID** — 13-bit field indicating the type of data in the payload. Reserved values per Table 2-3.
- **transport_scrambling_control** — 2-bit field, scrambling mode of the *payload* (header + adaptation field are never scrambled). Null packets: `'00'`. See Table 2-4.
- **adaptation_field_control** — 2-bit, see Table 2-5. Decoders shall discard packets with value `'00'`. Null packets: `'01'`.
- **continuity_counter** — 4-bit, increments per packet of the same PID, wraps to 0 after max. Shall not increment when `adaptation_field_control` is `'00'` or `'10'`. May be discontinuous when `discontinuity_indicator` is `'1'` (§2.4.3.4). Duplicate packets (exactly two consecutive, same PID) carry the same value with `adaptation_field_control` `'01'`/`'11'`.
- **data_byte** — contiguous bytes from PES packets, sections, stuffing, or private data per the PID. Null packets (PID 0x1FFF): any value. Count `N` = 184 − adaptation_field() byte count.

### Table 2-3 — PID table

| Value | Description |
|---|---|
| 0x0000 | Program association table |
| 0x0001 | Conditional access table |
| 0x0002 | Transport stream description table |
| 0x0003 | IPMP control information table |
| 0x0004 | Adaptive streaming information (see Note 2) |
| 0x0005..0x000F | Reserved |
| 0x0010..0x1FFE | May be assigned as network_PID, Program_map_PID, elementary_PID, or other purposes |
| 0x1FFF | Null packet |

> NOTE 1 — Packets with PID 0x0000, 0x0001, and 0x0010–0x1FFE are allowed to carry a PCR.
> NOTE 2 — Payload syntax defined in §5.10.3.3.5 of ISO/IEC 23009-1.

### Table 2-4 — transport_scrambling_control values

| Value | Description |
|---|---|
| '00' | Not scrambled |
| '01' | User-defined |
| '10' | User-defined |
| '11' | User-defined |

### Table 2-5 — adaptation_field_control values

| Value | Description |
|---|---|
| '00' | Reserved for future use by ISO/IEC (decoders shall discard) |
| '01' | No adaptation_field, payload only |
| '10' | adaptation_field only, no payload |
| '11' | adaptation_field followed by payload |

## §2.4.3.4 Adaptation field — Table 2-6 (adaptation_field)

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| `adaptation_field() {` | | |
| &nbsp;&nbsp;adaptation_field_length | 8 | uimsbf |
| &nbsp;&nbsp;`if (adaptation_field_length > 0) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;discontinuity_indicator | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;random_access_indicator | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;elementary_stream_priority_indicator | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;PCR_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;OPCR_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;splicing_point_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;transport_private_data_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;adaptation_field_extension_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`if (PCR_flag == '1') {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;program_clock_reference_base | 33 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 6 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;program_clock_reference_extension | 9 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`if (OPCR_flag == '1') {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;original_program_clock_reference_base | 33 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 6 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;original_program_clock_reference_extension | 9 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`if (splicing_point_flag == '1') {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;splice_countdown | 8 | tcimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`if (transport_private_data_flag == '1') {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;transport_private_data_length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for (i = 0; i < transport_private_data_length; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;private_data_byte | 8 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`if (adaptation_field_extension_flag == '1') {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;adaptation_field_extension_length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ltw_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;piecewise_rate_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;seamless_splice_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;af_descriptor_not_present_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 4 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`if (ltw_flag == '1') {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ltw_valid_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ltw_offset | 15 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`if (piecewise_rate_flag == '1') {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;piecewise_rate | 22 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`if (seamless_splice_flag == '1') {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;splice_type | 4 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;DTS_next_AU[32..30] | 3 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;marker_bit | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;DTS_next_AU[29..15] | 15 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;marker_bit | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;DTS_next_AU[14..0] | 15 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;marker_bit | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`if (af_descriptor_not_present_flag == '0') {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for (i = 0; i < N1; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;af_descriptor() | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`} else {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for (i = 0; i < N2; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 8 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i = 0; i < N3; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;stuffing_byte | 8 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

## §2.4.3.5 Semantics — adaptation field (framing-relevant fields)

- **adaptation_field_length** — 8-bit, number of bytes in the adaptation_field immediately following this field. Value `0` inserts a single stuffing byte. When `adaptation_field_control == '11'`: range **0..182**. When `'10'`: shall be **183**.
- **discontinuity_indicator** — 1-bit. `'1'` = discontinuity state true for this packet (a system time-base discontinuity on a PCR_PID, or a continuity_counter discontinuity). `'0'`/absent = false.
- **random_access_indicator** — 1-bit. `'1'` = the first byte of an elementary-stream access point starts in this packet (random access point hint).
- **elementary_stream_priority_indicator** — 1-bit elementary-stream priority within the adaptation field's PID.
- **PCR_flag / OPCR_flag** — 1-bit each; when `'1'`, the (Original) Program Clock Reference fields are present. PCR/OPCR = 33-bit base (90 kHz) + 6 reserved + 9-bit extension (27 MHz); full value = `base × 300 + extension` ticks of the 27 MHz clock.
- **splicing_point_flag** — `'1'` ⇒ `splice_countdown` (8-bit two's-complement, `tcimsbf`) present.
- **transport_private_data_flag** — `'1'` ⇒ `transport_private_data_length` + that many `private_data_byte`s.
- **adaptation_field_extension_flag** — `'1'` ⇒ the extension (ltw / piecewise_rate / seamless_splice / af_descriptor) per the table.

> Stuffing: for packets carrying PES, stuffing is by an over-long adaptation field (extra `stuffing_byte`s = 0xFF). For packets carrying sections, an alternative stuffing method applies (§2.4.4.1).

---

*Note: the source pages continue into §2.4.3.5's elementary-stream-access-point
definitions (AVC/SVC/MVC), which are codec-layer detail beyond this framing crate's
scope and are intentionally not transcribed here.*
