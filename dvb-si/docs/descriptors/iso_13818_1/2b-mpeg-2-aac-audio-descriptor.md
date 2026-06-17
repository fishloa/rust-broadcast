## Table 2-94 — MPEG-2 AAC audio descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.68, Table 2-94; PDF p.108. additional_information values per Table 2-95._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| MPEG-2_AAC_audio_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;MPEG-2_AAC_profile | 8 | uimsbf |
| &nbsp;&nbsp;MPEG-2_AAC_channel_configuration | 8 | uimsbf |
| &nbsp;&nbsp;MPEG-2_AAC_additional_information | 8 | uimsbf |
| } |  |  |
