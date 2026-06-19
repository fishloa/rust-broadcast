# Table 2-41 — Program stream map

_Source: ISO/IEC 13818-1:2021 (Rec. ITU-T H.222.0 06/2021) §2.5.4.1–2.5.4.2 (PDF p. 94)._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| program_stream_map() { |  |  |
| packet_start_code_prefix | 24 | bslbf |
| map_stream_id | 8 | uimsbf |
| program_stream_map_length | 16 | uimsbf |
| current_next_indicator | 1 | bslbf |
| single_extension_stream_flag | 1 | bslbf |
| reserved | 1 | bslbf |
| program_stream_map_version | 5 | uimsbf |
| reserved | 7 | bslbf |
| marker_bit | 1 | bslbf |
| program_stream_info_length | 16 | uimsbf |
| for (i=0; i<N; i++) { descriptor() } |  |  |
| elementary_stream_map_length | 16 | uimsbf |
| for (i=0; i<N1; i++) { |  |  |
| stream_type | 8 | uimsbf |
| elementary_stream_id | 8 | uimsbf |
| elementary_stream_info_length | 16 | uimsbf |
| if (elementary_stream_id == 0xFD && single_extension_stream_flag == 0) { |  |  |
| pseudo_descriptor_tag | 8 | uimsbf |
| pseudo_descriptor_length | 8 | uimsbf |
| marker_bit | 1 | bslbf |
| elementary_stream_id_extension | 7 | uimsbf |
| for (i=3; i<N2; i++) { descriptor() } |  |  |
| } else { |  |  |
| for (i=0; i<N2; i++) { descriptor() } |  |  |
| } |  |  |
| } |  |  |
| CRC_32 | 32 | rpchof |
| } |  |  |

Semantics:

- **packet_start_code_prefix**: `0x000001`; with **map_stream_id** (`0xBC`) forms the PSM
  packet start code.
- **program_stream_map_length**: bytes following this field; max 1018 (`0x3FA`).
- **single_extension_stream_flag**: when `1`, the stream has at most one `stream_id == 0xFD`.
- **current_next_indicator**: `1` = currently applicable.
- **program_stream_map_version**: 5-bit version (mod 32).
- **CRC_32**: CRC over the PSM (MPEG-2 CRC-32, `rpchof`).
