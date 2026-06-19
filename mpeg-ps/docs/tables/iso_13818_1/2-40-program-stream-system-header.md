# Table 2-40 — Program stream system header

_Source: ISO/IEC 13818-1:2021 (Rec. ITU-T H.222.0 06/2021) §2.5.3.5–2.5.3.6 (PDF p. 91).
Verified against the PDF render._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| system_header() { |  |  |
| system_header_start_code | 32 | bslbf |
| header_length | 16 | uimsbf |
| marker_bit | 1 | bslbf |
| rate_bound | 22 | uimsbf |
| marker_bit | 1 | bslbf |
| audio_bound | 6 | uimsbf |
| fixed_flag | 1 | bslbf |
| CSPS_flag | 1 | bslbf |
| system_audio_lock_flag | 1 | bslbf |
| system_video_lock_flag | 1 | bslbf |
| marker_bit | 1 | bslbf |
| video_bound | 5 | uimsbf |
| packet_rate_restriction_flag | 1 | bslbf |
| reserved_bits | 7 | bslbf |
| while (nextbits() == '1') { |  |  |
| stream_id | 8 | uimsbf |
| if (stream_id == '1011 0111') { |  |  |
| '11' | 2 | bslbf |
| '000 0000' | 7 | bslbf |
| stream_id_extension | 7 | uimsbf |
| '1011 0110' | 8 | bslbf |
| '11' | 2 | bslbf |
| P-STD_buffer_bound_scale | 1 | bslbf |
| P-STD_buffer_size_bound | 13 | uimsbf |
| } else { |  |  |
| '11' | 2 | bslbf |
| P-STD_buffer_bound_scale | 1 | bslbf |
| P-STD_buffer_size_bound | 13 | uimsbf |
| } } |  |  |
| } |  |  |

Semantics:

- **system_header_start_code**: `0x000001BB`.
- **rate_bound**: 22-bit, ≥ the max `program_mux_rate` of any pack.
- **audio_bound** (6) / **video_bound** (5): upper bounds on simultaneously-active
  audio / video streams.
- **fixed_flag**, **CSPS_flag**, **system_audio_lock_flag**, **system_video_lock_flag**,
  **packet_rate_restriction_flag**: 1-bit constraint indicators.
- The `stream_id` loop carries a P-STD buffer bound per stream; the
  `stream_id == 0xB7` branch is the extended (stream_id_extension) form.
