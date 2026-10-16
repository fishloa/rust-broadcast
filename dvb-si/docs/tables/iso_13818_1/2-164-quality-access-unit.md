## Table 2-164 — Quality Access Unit
_Rec. ITU-T H.222.0 (08/2023) §2.20.1; printed pp.228-229 (PDF pp.250-251). Verified on the rendered pages (PDF pp.250-251, printed pp.228-229)._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| Quality_Access_Unit() { |  |  |
| field_size_bytes | 8 | uimsbf |
| metric_count | 8 | uimsbf |
| for (i = 0; i < metric_count; i++) { |  |  |
| metric_code | 32 | uimsbf |
| sample_count | 8 | uimsbf |
| for (j = 0; j < sample_count; j++) { |  |  |
| '0010' | 4 | bslbf |
| media_DTS [32..30] | 3 | uimsbf |
| marker_bit | 1 | bslbf |
| media_DTS [29..15] | 15 | uimsbf |
| marker_bit | 1 | bslbf |
| media_DTS [14..0] | 15 | uimsbf |
| marker_bit | 1 | bslbf |
| quality_metric_sample | N | uimsbf |
| } |  |  |
| } |  |  |
| } |  |  |

field_size_bytes: "Size of the quality_metric_sample field. As such, N=8*field_size_bytes". metric_count:
number of metrics, "as defined in 4.2 of ISO/IEC 23001-10". metric_code: the code name of the metric
(ISO/IEC 23001-10 §4.2, not vendored). sample_count: "Number of quality metric samples per metric".
media_DTS: "DTS of the media access unit described by the quality metric sample". Each value is "padded by
preceding zero bytes, as needed, to the number of bytes indicated by field_size_bytes" (§2.20.1).

> **Spec note:** the Quality Access Unit is "carried in MPEG sections" identified by stream_type 0x2F, and
> Table 2-31 assigns table_id 0x0A to "ISO/IEC 23001-10 (Quality access unit) section", but the vendored
> primary text gives **only this access-unit body**: there is no section-header syntax for table_id 0x0A
> (short or long form, `private_section_length`, `CRC_32`), unlike Tables 2-153, 2-158, 2-162 for the
> sibling sections. §2.20.2 refers to the sections being removed "based on section length", which implies
> a section length but does not define the header. No wrapper is implemented (it would be a guess);
> only the body is typed (`quality_access_unit::QualityAccessUnit`). The `'0010'` prefix + 33-bit
> `media_DTS` with three marker bits is the same 5-byte marker-coded timestamp as a PES PTS-only field
> (`broadcast_common::clock33::unpack_marked`/`pack_marked`); the prefix and markers are literals in the
> table. The table defines no bytes after the last metric.
