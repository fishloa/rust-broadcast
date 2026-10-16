## Table 2-96 — MPEG-4 text descriptor (tag 0x2D)
_Rec. ITU-T H.222.0 (08/2023) | ISO/IEC 13818-1:2023 §2.6.70–2.6.71, Table 2-96; printed p.110 (PDF p.132). Verified against the rendered page._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| MPEG-4_text_descriptor () { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;textConfig() |  |  |
| } |  |  |

`textConfig()` is the `TextConfig()` of ISO/IEC 14496-17 (not vendored): it is carried as an opaque body.

## Table 2-97 — MPEG-4 audio extension descriptor (tag 0x2E)
_Rec. ITU-T H.222.0 (08/2023) §2.6.72–2.6.73, Table 2-97; printed p.110 (PDF p.132). Verified against the rendered page._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| MPEG-4_audio_extension_descriptor () { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;ASC_flag | 1 | bslbf |
| &nbsp;&nbsp;reserved | 3 | bslbf |
| &nbsp;&nbsp;num_of_loops | 4 | uimsbf |
| &nbsp;&nbsp;for (i=0; i<num_of_loops; i++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;audioProfileLevelIndication | 8 | uimsbf |
| &nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;if (ASC_flag == '1') { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;ASC_size | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;audioSpecificConfig() |  |  |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

`audioSpecificConfig()` is ISO/IEC 14496-3 §1.6.2.1 (not vendored); `ASC_size` is its length in bytes. Carried opaque.
