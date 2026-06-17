## Table 2-60 — Video window descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.14, Table 2-60; PDF p.85._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| video_window_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;horizontal_offset | 14 | uimsbf |
| &nbsp;&nbsp;vertical_offset | 14 | uimsbf |
| &nbsp;&nbsp;window_priority | 4 | uimsbf |
| } |  |  |
