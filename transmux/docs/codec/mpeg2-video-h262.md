# MPEG-2 video (ITU-T H.262 | ISO/IEC 13818-2) — config for the transmux IR

Source: **ITU-T H.262 (1995)** = ISO/IEC 13818-2, vendored at
`specs/itu_t_h262_mpeg2_video.pdf`. Fields below are the subset transmux needs to
recover a `CodecConfig::Mpeg2Video` (coded dimensions + profile/level); coded
samples pass through opaque.

Carriage:
- **MPEG-2 TS:** `stream_type` **0x02** (ISO/IEC 13818-1 Table 2-34); the ES is the
  H.262 bitstream (sequence header + pictures).
- **ISO-BMFF / mp4:** `mp4v` visual sample entry + `esds` with ObjectTypeIndication
  **0x60–0x65** (0x61 = MPEG-2 Main Visual, ISO/IEC 14496-1); the `esds`
  DecoderSpecificInfo carries the sequence header bytes.

Start codes are `00 00 01 xx`; `sequence_header_code` = `0x000001B3`.

## sequence_header() — H.262 §6.2.2.1

| field | bits | mnemonic |
|---|---|---|
| `sequence_header_code` | 32 | bslbf (`0x000001B3`) |
| `horizontal_size_value` | 12 | uimsbf |
| `vertical_size_value` | 12 | uimsbf |
| `aspect_ratio_information` | 4 | uimsbf |
| `frame_rate_code` | 4 | uimsbf |
| `bit_rate_value` | 18 | uimsbf |
| `marker_bit` | 1 | bslbf |
| `vbv_buffer_size_value` | 10 | uimsbf |
| `constrained_parameters_flag` | 1 | bslbf |
| `load_intra_quantiser_matrix` | 1 | uimsbf |
| `if (load_intra_quantiser_matrix)` `intra_quantiser_matrix[64]` | 8×64 | uimsbf |
| `load_non_intra_quantiser_matrix` | 1 | uimsbf |
| `if (load_non_intra_quantiser_matrix)` `non_intra_quantiser_matrix[64]` | 8×64 | uimsbf |
| `next_start_code()` | | |

## sequence_extension() — H.262 §6.2.2.3 (follows sequence_header; extension_start_code `0x000001B5`)

| field | bits | mnemonic |
|---|---|---|
| `extension_start_code` | 32 | bslbf (`0x000001B5`) |
| `extension_start_code_identifier` | 4 | uimsbf (`0001` = Sequence Extension ID) |
| `profile_and_level_indication` | 8 | uimsbf |
| `progressive_sequence` | 1 | uimsbf |
| `chroma_format` | 2 | uimsbf (1=4:2:0, 2=4:2:2, 3=4:4:4) |
| `horizontal_size_extension` | 2 | uimsbf |
| `vertical_size_extension` | 2 | uimsbf |
| `bit_rate_extension` | 12 | uimsbf |
| `marker_bit` | 1 | bslbf |
| `vbv_buffer_size_extension` | 8 | uimsbf |
| `low_delay` | 1 | uimsbf |
| `frame_rate_extension_n` | 2 | uimsbf |
| `frame_rate_extension_d` | 5 | uimsbf |

## Derived config

- **coded width**  = `(horizontal_size_extension << 12) | horizontal_size_value`
- **coded height** = `(vertical_size_extension  << 12) | vertical_size_value`
- **chroma**: from `chroma_format` (defaults to 4:2:0 for MPEG-1-style streams with
  no `sequence_extension`).
- **profile/level**: `profile_and_level_indication` (present only with
  `sequence_extension`; MPEG-1 streams have none).

A pure MPEG-1 video stream (ISO/IEC 11172-2) has `sequence_header()` but **no**
`sequence_extension()` — then the `_extension` fields are absent and
width/height are the 12-bit `_value` fields directly.
