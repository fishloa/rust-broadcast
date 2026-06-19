# Table 2-39 — Program stream pack header

_Source: ISO/IEC 13818-1:2021 (Rec. ITU-T H.222.0 06/2021) §2.5.3.3–2.5.3.4 (PDF p. 90)._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| pack_header() { |  |  |
| pack_start_code | 32 | bslbf |
| '01' | 2 | bslbf |
| system_clock_reference_base [32..30] | 3 | bslbf |
| marker_bit | 1 | bslbf |
| system_clock_reference_base [29..15] | 15 | bslbf |
| marker_bit | 1 | bslbf |
| system_clock_reference_base [14..0] | 15 | bslbf |
| marker_bit | 1 | bslbf |
| system_clock_reference_extension | 9 | uimsbf |
| marker_bit | 1 | bslbf |
| program_mux_rate | 22 | uimsbf |
| marker_bit | 1 | bslbf |
| marker_bit | 1 | bslbf |
| reserved | 5 | bslbf |
| pack_stuffing_length | 3 | uimsbf |
| for (i=0; i<pack_stuffing_length; i++) { stuffing_byte | 8 | bslbf |
| } |  |  |
| if (nextbits() == system_header_start_code) { system_header() |  |  |
| } |  |  |
| } |  |  |

Semantics:

- **pack_start_code**: `0x000001BA`. Identifies the beginning of a pack.
- **system_clock_reference_base / _extension**: the 42-bit SCR (33-bit base + 9-bit
  extension), intended time of arrival at the P-STD input.
- **marker_bit**: 1-bit field, always `'1'`.
- **program_mux_rate**: 22-bit integer, rate the P-STD receives the stream during this
  pack, in units of **50 bytes/second**. Value `0` is forbidden.
- **pack_stuffing_length**: 3-bit count of `stuffing_byte`s that follow (≤ 7).
- **stuffing_byte**: `0xFF`, discarded by the decoder.
