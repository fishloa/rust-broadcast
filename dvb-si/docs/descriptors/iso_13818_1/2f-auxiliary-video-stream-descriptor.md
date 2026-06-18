## Table 2-98 — Auxiliary video stream descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.74, Table 2-98; PDF p.110._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| Auxiliary_video_stream_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;aux_video_codedstreamtype | 8 | uimsbf |
| &nbsp;&nbsp;si_rbsp(descriptor_length-1) |  |  |
| } |  |  |

`si_rbsp()` is the AVSI supplemental-information RBSP defined in ISO/IEC 23002-3 — carried here as opaque bytes.
