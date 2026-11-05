# dvb-si 11.1.0

_Released 2026-10-13._

### Fixed

- Presence flags stored next to an `Option` no longer drive the serializer, so a mismatch cannot write a misframed descriptor (r03-W5, #1138): `metadata` (`dsmcc_flag`), `content_labeling` (`content_reference_id_record_flag`, and an indicator/Option mismatch now returns `ValueOutOfRange`), `j2k_video` (`extended_capability_flag`, the stripe/block/mdm flags, and a missing `color_specification` now returns `ValueOutOfRange` instead of leaving a stale byte), `supplementary_audio` (`language_code_present`). The flag fields are kept (informational, set by parse); serialize derives the wire bit from the `Option`.
- **Reserved-bit emission (r03-W7) — serialized bytes change for non-conformant reserved bits; parse unaffected; lockstep minor.** `reserved` bits are now emitted as 1 per ISO/IEC 13818-1 / H.222.0 (and TS 102 006 Table 4) in `video_stream`, `system_clock`, `maximum_bitrate`, `smoothing_buffer`, `STD`, `AVC_timing_and_HRD`, `SVC_extension`, `MVC_extension` (confirmed by the TSDuck-compiled oracle PMT; `video_stream` also by the real `shared/real-pmt.bin`), plus `multiplex_buffer_utilization`, `metadata_pointer`, `metadata` and the SSU `data_broadcast_id` selector (spec convention only, no fixture carries them). Parsers still accept any reserved value and re-serialize normalizes it; `tests/reserved_bits_real_streams.rs` asserts the committed real/oracle descriptors re-serialize byte-exactly. `j2k_video` stays 0 (its table says "all bits to be set to '0'"). The remaining EN 300 468 / TS 102 809 descriptors and tables were swept and already follow their tables (`reserved_future_use`=1, `reserved_zero_future_use`=0).
- `video_depth_range`: a `production_disparity_hint` with `range_length > 3` or a multi-region-SEI entry with a body is preserved verbatim as `DepthRangeBody::Other` instead of dropping its tail; `protection_message` ignores (does not reject) bytes after the component-tag loop (r03-W9).

---

Published from tag `dvb-si-v11.1.0`.
