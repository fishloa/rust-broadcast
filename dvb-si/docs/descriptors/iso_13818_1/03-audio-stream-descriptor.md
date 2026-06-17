## Table 2-48 — Audio stream descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.4, Table 2-48; PDF p.79._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| audio_stream_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;free_format_flag | 1 | bslbf |
| &nbsp;&nbsp;ID | 1 | bslbf |
| &nbsp;&nbsp;layer | 2 | bslbf |
| &nbsp;&nbsp;variable_rate_audio_indicator | 1 | bslbf |
| &nbsp;&nbsp;reserved | 3 | bslbf |
| } |  |  |
