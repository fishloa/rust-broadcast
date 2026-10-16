Bodies selected by `extension_descriptor_tag` (Table 2-110, tag 0x3F); the spec omits the `descriptor_tag`/`descriptor_length`/`extension_descriptor_tag` header from these tables. The MPEG-H 3D audio bodies 0x08..0x0E are in `3f-extension-descriptor-mpegh-3d-audio.md`.

## Table 2-128 — Quality extension descriptor (ext 0x0F)
_Rec. ITU-T H.222.0 (08/2023) §2.6.119; printed p.144 (PDF p.166). Verified against the rendered page. Semantics: ISO/IEC 23001-10 §4.2 (not vendored)._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| Quality_extension_descriptor() { |  |  |
| &nbsp;&nbsp;field_size_bytes | 8 | uimsbf |
| &nbsp;&nbsp;metric_count | 8 | uimsbf |
| &nbsp;&nbsp;for (i=0; i < metric_count; i++) { metric_code[i] | 32 | uimsbf } |  |  |
| } |  |  |

## Table 2-129 — Virtual segmentation descriptor (ext 0x10)
_§2.6.120–2.6.121; printed p.145 (PDF p.167). Verified against the rendered page (byte alignment checked: each partition is 8 + 8 + MDL×8 bits or 8 + 8 + 16 bits)._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| Virtual_segmentation_descriptor(){ |  |  |
| &nbsp;&nbsp;if (descriptor_length > 1) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;num_partitions | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;timescale_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 4 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;if (timescale_flag == 1) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ticks_per_second | 21 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;maximum_duration_length_minus_1 (MDL) | 2 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;for (i = 0; i < num_partitions; i++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;explicit_boundary_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;partition_id | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 4 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;SAP_type_max | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;if (explicit_boundary_flag == 0) { reserved (5); boundary_PID (13); reserved (3) } |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;else { maximum_duration (MDL*8 + 5) } |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

> **Deferred (ambiguous text), not implemented.** §2.6.121 (printed p.145) says, verbatim: "timescale_flag: If set to '1', timescale information is present. If set to '0', ticks_per_second is inferred to be 1, and MDL is inferred to be 0 (i.e., maximum_duration_length_minus_1=−1)." and "maximum_duration_length_minus_1: Length, in bytes (minus one), of the maximum_duration field variable byte length. This provides additional bytes in addition to the 5 bits pre-allocated to the maximum_duration field." while Table 2-129 labels the 2-bit field itself "(MDL)" and sizes `maximum_duration` as `MDL*8 + 5` bits. Two readings of the extra byte count are possible (`MDL` = field value, or field value + 1); the text does not settle it. TSDuck 3.44 (`tstabcomp`) encodes the first reading (a 13-bit `maximum_duration` is written with the 2-bit field = 1, and its model limits `maximum_duration` to 29 bits = field 3), but a single implementation is not the spec. The descriptor therefore stays `Mpeg2ExtensionBody::Other` (verbatim bytes) until the text is clarified. `descriptor_length` here is the size of the body after `extension_descriptor_tag` (cf. Table 2-130 "N1 = descriptor_length minus 1").

## Table 2-130 — HEVC tile substream descriptor (ext 0x12)
_§2.6.122–2.6.123; printed p.146 (PDF p.168). Verified against the rendered page._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| HEVC_tile_substream_descriptor() { |  |  |
| &nbsp;&nbsp;ReferenceFlag | 1 | bslbf |
| &nbsp;&nbsp;SubstreamID | 7 | uimsbf |
| &nbsp;&nbsp;if (descriptor_length > 1) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;if (ReferenceFlag == '1') { PreambleFlag (1, bslbf); PatternReference (7, uimsbf) } |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;else { for (i=0; i<N1; i++) { Flag[i] (1, bslbf); AdditionalSubstreamID[i] (7, uimsbf) } } |  |  |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

N1 = descriptor_length − 1 (body size minus the first byte). When the body is a single byte, `ReferenceFlag` is reserved.

## Table 2-131 — HEVC subregion descriptor (ext 0x13)
_§2.6.125–2.6.126; printed p.147 (PDF p.169). Verified against the rendered page._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| HEVC_subregion_descriptor() { |  |  |
| &nbsp;&nbsp;SubstreamMarkingFlag | 1 | bslbf |
| &nbsp;&nbsp;SubstreamIDsPerLine | 7 | uimsbf |
| &nbsp;&nbsp;TotalSubstreamIDs | 8 | uimsbf |
| &nbsp;&nbsp;LevelFullPanorama | 8 | uimsbf |
| &nbsp;&nbsp;for (i=0; i<N1; i++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;if (SubstreamMarkingFlag == 1) { reserved (1, bslbf); PreambleSubstreamID[i] (7, uimsbf) } |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;SubstreamCountMinus1[i] | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;Level[i] | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;PictureSizeHor[i] | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;PictureSizeVert[i] | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;PatternCount[i] | 7 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;for (j=0; j<PatternCount[i]; j++) { for (k=0; k<SubstreamCountMinus1[i]; k++) { SubstreamOffset[k][j][i] | 8 | tcimsbf } } |  |  |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

N1 is implied by the descriptor size.

> **Deferred (loop bound in doubt), not implemented.** The table prints the inner loop as `for (k=0; k<SubstreamCountMinus1[i]; k++)`, i.e. `SubstreamCountMinus1` offsets per pattern, while §2.6.126 describes `SubstreamCountMinus1[i]` (range 0..TotalSubstreamIDs−1) as "the number of HEVC tile substreams to be appended". TSDuck 3.44 (`tstabcomp`) disagrees with the printed bound: for two `<Substream offset>` entries it writes `SubstreamCountMinus1 = 1` followed by **two** offset bytes (see `dvb-si/tests/fixtures/tsduck-h222-descriptors-pmt.bin`, ext 0x13). Because the printed syntax and the only independent implementation disagree, the descriptor stays `Mpeg2ExtensionBody::Other` (verbatim bytes) until the text is clarified.

## Table 2-132 — JPEG XS video descriptor (ext 0x14)
_§2.6.127–2.6.128; printed p.149 (PDF p.171). Verified against the rendered page._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| JXS_video_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_version | 8 | uimsbf |
| &nbsp;&nbsp;horizontal_size | 16 | uimsbf |
| &nbsp;&nbsp;vertical_size | 16 | uimsbf |
| &nbsp;&nbsp;brat | 32 | uimsbf |
| &nbsp;&nbsp;frat | 32 | bslbf |
| &nbsp;&nbsp;schar | 16 | bslbf |
| &nbsp;&nbsp;Ppih | 16 | bslbf |
| &nbsp;&nbsp;Plev | 16 | bslbf |
| &nbsp;&nbsp;max_buffer_size | 32 | uimsbf |
| &nbsp;&nbsp;buffer_model_type | 8 | uimsbf |
| &nbsp;&nbsp;colour_primaries | 8 | uimsbf |
| &nbsp;&nbsp;transfer_characteristics | 8 | uimsbf |
| &nbsp;&nbsp;matrix_coefficients | 8 | uimsbf |
| &nbsp;&nbsp;video_full_range_flag | 1 | bslbf |
| &nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;still_mode | 1 | bslbf |
| &nbsp;&nbsp;mdm_flag | 1 | bslbf |
| &nbsp;&nbsp;zero_bits | 6 | bslbf |
| &nbsp;&nbsp;if (mdm_flag == '1') { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;X_c0, Y_c0, X_c1, Y_c1, X_c2, Y_c2 | 16x6 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;X_wp | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;Y_wp | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;L_max | 32 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;L_min | 32 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;MaxCLL | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;MaxFALL | 16 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;for (i=0; i<N; i++) { private_data_byte | 8 | bslbf } |  |  |
| } |  |  |

## Table 2-139 — VVC timing and HRD descriptor (ext 0x15)
_§2.6.131–2.6.132; printed p.154 (PDF p.176). Verified against the rendered page._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| VVC_timing_and_HRD_descriptor() { |  |  |
| &nbsp;&nbsp;hrd_management_valid_flag | 1 | bslbf |
| &nbsp;&nbsp;reserved | 6 | bslbf |
| &nbsp;&nbsp;picture_and_timing_info_present_flag | 1 | bslbf |
| &nbsp;&nbsp;if (picture_and_timing_info_present_flag == '1') { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;90kHz_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;if (90kHz_flag == '0') { N (32, uimsbf); K (32, uimsbf) } |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;num_units_in_tick | 32 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

## Table 2-141 — EVC timing and HRD descriptor (ext 0x16)
_§2.6.135–2.6.136; printed p.157 (PDF p.179). Verified against the rendered page; identical layout to Table 2-139._

## Table 2-142 — LCEVC video descriptor (ext 0x17)
_§2.6.137–2.6.138; printed p.158 (PDF p.180). Verified against the rendered page._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| LCEVC_video_descriptor() { |  |  |
| &nbsp;&nbsp;lcevc_stream_tag | 8 | uimsbf |
| &nbsp;&nbsp;profile_idc | 4 | uimsbf |
| &nbsp;&nbsp;level_idc | 4 | uimsbf |
| &nbsp;&nbsp;sublevel_idc | 2 | uimsbf |
| &nbsp;&nbsp;processed_planes_type_flag | 1 | bslbf |
| &nbsp;&nbsp;picture_type_bit_flag | 1 | bslbf |
| &nbsp;&nbsp;field_type_bit_flag | 1 | bslbf |
| &nbsp;&nbsp;reserved | 3 | bslbf |
| &nbsp;&nbsp;HDR_WCG_idc | 2 | uimsbf |
| &nbsp;&nbsp;reserved_zero_2bit | 2 | bslbf |
| &nbsp;&nbsp;video_properties_tag | 4 | uimsbf |
| } |  |  |

## Table 2-143 — LCEVC linkage descriptor (ext 0x18)
_§2.6.139–2.6.140; printed p.159 (PDF p.181). Verified against the rendered page._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| LCEVC_linkage_descriptor() { |  |  |
| &nbsp;&nbsp;num_lcevc_stream_tags | 8 | uimsbf |
| &nbsp;&nbsp;for (i=0; i<num_lcevc_stream_tags; i++) { lcevc_stream_tag | 8 | uimsbf } |  |  |
| } |  |  |

## Table 2-144 — Media service kind descriptor (ext 0x19)
_§2.6.141–2.6.142, Tables 2-145..2-151; printed p.160–162 (PDF p.182–184). Verified against the rendered pages._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| Media_service_kind_descriptor() { |  |  |
| &nbsp;&nbsp;for (i=0; i<N; i++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;media_description_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;identifier_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;lang_pairs | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;media_type_idc | 2 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;if (identifier_flag) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ID_length_code | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ID_type | 13 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;if (ID_length_code == '7') { ID_len | 8 | uimsbf } |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;media_ID_field | ID_len*8 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;for (j=0; j<lang_pairs; j++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;configuration_type | 2 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;lang_purpose_cnt | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;lang_len_idc | 2 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;if (lang_len_idc == 0) { lang_len | 8 | uimsbf } |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;IETF_BCP_47_language_code | lang_len*8 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;for (k=0; k<lang_purpose_cnt; k++) { media_service_type | 8 | uimsbf } |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

Table 2-146 `media_type_idc`: 0 unknown, 1 video, 2 audio, 3 text/data. Table 2-147 `ID_length_code`: 0→1, 1→2, 2→4, 3→8, 4→12, 5→16, 6→20 bytes, 7→explicit `ID_len`. Table 2-149 `configuration_type`: 0 complete, 1 partial, 2 complete combination, 3 reserved. Table 2-150 `lang_len_idc`: 0 explicit `lang_len`, 1→2, 2→3, 3 reserved. `lang_purpose_cnt` 7 is reserved. Table 2-151 `media_service_type` is 8-bit and its meaning depends on `media_type_idc` (Tables 2-151..2-153); carried as `u8`.

## ext 0x11 — timed_metadata_extension_descriptor
_Table 2-109 / 2-110 only name it; the structure is defined in clause 8.2.3 of ISO/IEC 23001-13 (not vendored): carried opaque._
