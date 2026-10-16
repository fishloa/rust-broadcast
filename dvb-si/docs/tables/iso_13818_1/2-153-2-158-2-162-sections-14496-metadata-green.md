## Table 2-153 — Section syntax for transport of ISO/IEC 14496 stream (table_id 0x04, 0x05, 0x08)
_Rec. ITU-T H.222.0 (08/2023) §2.11.3.8; printed p.177 (PDF p.199). Verified against the rendered page._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| ISO_IEC_14496_section() { |  |  |
| &nbsp;&nbsp;table_id | 8 | uimsbf |
| &nbsp;&nbsp;section_syntax_indicator | 1 | bslbf |
| &nbsp;&nbsp;private_indicator | 1 | bslbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;ISO_IEC_14496_section_length | 12 | uimsbf |
| &nbsp;&nbsp;table_id_extension | 16 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;version_number | 5 | uimsbf |
| &nbsp;&nbsp;current_next_indicator | 1 | bslbf |
| &nbsp;&nbsp;section_number | 8 | uimsbf |
| &nbsp;&nbsp;last_section_number | 8 | uimsbf |
| &nbsp;&nbsp;if (PMT_has_SL_descriptor(current_PID)) { SL_Packet() } |  |  |
| &nbsp;&nbsp;else if (PMT_has_FMC_descriptor(current_PID)) { for (i = 1; i < N1; i++) M4MuxPacket() } |  |  |
| &nbsp;&nbsp;else { for (i = 1; i < N2; i++) { reserved | 8 | bslbf } } |  |  |
| &nbsp;&nbsp;CRC_32 | 32 | rpchof |
| } |  |  |

table_id 0x04 = scene description stream, 0x05 = object descriptor stream, 0x08 = other ISO/IEC 14496 streams. The payload alternative depends on the PMT of the carrying PID (out of band) and on ISO/IEC 14496-1 (not vendored): carried as one opaque byte run.

## Table 2-158 — Section syntax for transport of metadata (table_id 0x06)
_§2.12.6; printed p.188 (PDF p.210). Verified against the rendered page._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| Metadata_section() { |  |  |
| &nbsp;&nbsp;table_id | 8 | uimsbf |
| &nbsp;&nbsp;section_syntax_indicator | 1 | bslbf |
| &nbsp;&nbsp;private_indicator | 1 | bslbf |
| &nbsp;&nbsp;random_access_indicator | 1 | bslbf |
| &nbsp;&nbsp;decoder_config_flag | 1 | bslbf |
| &nbsp;&nbsp;metadata_section_length | 12 | uimsbf |
| &nbsp;&nbsp;metadata_service_id | 8 | uimsbf |
| &nbsp;&nbsp;reserved | 8 | bslbf |
| &nbsp;&nbsp;section_fragment_indication | 2 | bslbf |
| &nbsp;&nbsp;version_number | 5 | uimsbf |
| &nbsp;&nbsp;current_next_indicator | 1 | bslbf |
| &nbsp;&nbsp;section_number | 8 | uimsbf |
| &nbsp;&nbsp;last_section_number | 8 | uimsbf |
| &nbsp;&nbsp;for (i = 1; i < N; i++) { metadata_byte | 8 | bslbf } |  |  |
| &nbsp;&nbsp;CRC_32 | 32 | rpchof |
| } |  |  |

Table 2-159 `section_fragment_indication` (printed p.189): `11` single section carrying a complete AU; `10` first; `01` last; `00` neither first nor last.

## Table 2-162 — Green access unit section syntax (table_id 0x09)
_§2.18.1–2.18.3; printed p.225 (PDF p.247). Verified against the rendered page._

| Syntax | Bits | Mnemonic |
|---|---|---|
| Green_access_unit_section_message(){ |  |  |
| &nbsp;&nbsp;table_ID | 8 | uimsbf |
| &nbsp;&nbsp;section_syntax_indicator | 1 | bslbf |
| &nbsp;&nbsp;private_indicator | 1 | bslbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;private_section_length | 12 | uimsbf |
| &nbsp;&nbsp;'0010' | 4 | bslbf |
| &nbsp;&nbsp;Display_in_PTS [32..30] | 3 | bslbf |
| &nbsp;&nbsp;marker_bit | 1 | bslbf |
| &nbsp;&nbsp;Display_in_PTS [29..15] | 15 | bslbf |
| &nbsp;&nbsp;marker_bit | 1 | bslbf |
| &nbsp;&nbsp;Display_in_PTS [14..0] | 15 | bslbf |
| &nbsp;&nbsp;marker_bit | 1 | bslbf |
| &nbsp;&nbsp;Green_AU() |  |  |
| &nbsp;&nbsp;CRC_32 | 32 | rpchof |
| } |  |  |

`section_syntax_indicator` shall be `'0'`. `Green_AU()` (Table 2-163: `num_quality_levels`(4) `reserved`(4), then loops bounded by `num_constant_backlight_voltage_time_intervals` and `num_max_variations` of the Green extension descriptor, which are out of band) cannot be parsed from the section alone: carried as one opaque byte run.

## Not typed as sections (no section syntax in the primary text)
table_id 0x07 (IPMP, ISO/IEC 13818-11), 0x0B (media orchestration AU) and 0x38..0x39 (ISO/IEC 13818-6): Table 2-31 only assigns them.

table_id 0x0A (quality access unit): the *body* is defined (Table 2-164, §2.20.1) and is typed as the standalone
`quality_access_unit::QualityAccessUnit` (see `2-164-quality-access-unit.md`); what the vendored text does not define is the
**section wrapper** (header bits, `private_section_length`, `CRC_32`) that carries it "in MPEG sections", so no
`table_id` 0x0A section type exists and the section is still dispatched as `AnyTableSection::Unknown`.
