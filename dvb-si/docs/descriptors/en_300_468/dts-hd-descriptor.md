# DTS-HD descriptor — ETSI EN 300 468 V1.19.1 Annex G.3 (extension descriptor, tag_extension 0x0E)

_Source: ETSI EN 300 468 V1.19.1 (2025-02), Annex G (normative), §G.3 (PDF pp. 187–189).
Verified against the PDF render. Carried as an extended descriptor (descriptor_tag
0x7F) per §6.2.16; appears in the PMT ES_info loop._

## Table G.6 — DTS-HD descriptor

| Syntax | No. of bits | Identifier |
|---|---|---|
| DTS-HD_descriptor() { |  |  |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| substream_core_flag | 1 | bslbf |
| substream_0_flag | 1 | bslbf |
| substream_1_flag | 1 | bslbf |
| substream_2_flag | 1 | bslbf |
| substream_3_flag | 1 | bslbf |
| reserved_future_use | 3 | bslbf |
| if (substream_core_flag == '1') { substream_info() } |  |  |
| if (substream_0_flag == '1') { substream_info() } |  |  |
| if (substream_1_flag == '1') { substream_info() } |  |  |
| if (substream_2_flag == '1') { substream_info() } |  |  |
| if (substream_3_flag == '1') { substream_info() } |  |  |
| for (i=0; i<N; i++) { additional_info_byte | 8 | uimsbf |
| } |  |  |
| } |  |  |

- **substream_core_flag / substream_0..3_flag**: a substream_info() block follows for each set flag (core present; ext substream nuExtSSIndex 0..3 present).
- **additional_info_byte**: optional reserved tail.

## Table G.7 — substream_info()

| Syntax | No. of bits | Identifier |
|---|---|---|
| substream_info() { |  |  |
| substream_length | 8 | uimsbf |
| num_assets | 3 | uimsbf |
| channel_count | 5 | uimsbf |
| lfe_flag | 1 | bslbf |
| sampling_frequency | 4 | uimsbf |
| sample_resolution | 1 | bslbf |
| reserved_future_use | 2 | bslbf |
| for (i=0; i<N; i++) { asset_info() } |  |  |
| } |  |  |

- **substream_length**: bytes following this field (incl. the embedded asset_info()).
- **num_assets**: number of audio assets = `num_assets + 1` (0 for a core substream).
- **sampling_frequency**: coded per Table G.8.
- **sample_resolution**: '1' if decoded resolution > 16 bit.
- **asset_info()**: appears `num_assets + 1` times.

## Table G.8 — Sampling frequency coding

| sampling_frequency | Description |
|---|---|
| 0 | 8 kHz | 1 | 16 kHz | 2 | 32 kHz | 3 | 64 kHz | 4 | 128 kHz (not for core) |
| 5 | 22,05 kHz | 6 | 44,1 kHz | 7 | 88,2 kHz | 8 | 176,4 kHz (not for core) | 9 | 352,8 kHz (not for core) |
| 10 | 12 kHz | 11 | 24 kHz | 12 | 48 kHz | 13 | 96 kHz | 14 | 192 kHz (not for core) | 15 | 348 kHz (not for core) |

(Stored as the raw 4-bit value with this mapping as the label table.)

## Table G.9 — asset_info()

| Syntax | No. of bits | Identifier |
|---|---|---|
| asset_info() { |  |  |
| asset_construction | 5 | uimsbf |
| vbr_flag | 1 | bslbf |
| post_encode_br_scaling_flag | 1 | bslbf |
| component_type_flag | 1 | bslbf |
| language_code_flag | 1 | bslbf |
| if (post_encode_br_scaling_flag == '1') { bit_rate_scaled | 13 | bslbf |
| } else { bit_rate | 13 | uimsbf |
| } |  |  |
| reserved_future_use | 2 | bslbf |
| if (component_type_flag == '1') { component_type | 8 | bslbf |
| } |  |  |
| if (language_code_flag == '1') { ISO_639_language_code | 24 | bslbf |
| } |  |  |
| } |  |  |

- **asset_construction**: 5-bit, interpreted per Table G.10 (core/extension substream mix; 21 defined values). Stored as the raw value.
- **bit_rate_scaled / bit_rate**: 13-bit, selected by post_encode_br_scaling_flag.
- **component_type / ISO_639_language_code**: present per their flags.
