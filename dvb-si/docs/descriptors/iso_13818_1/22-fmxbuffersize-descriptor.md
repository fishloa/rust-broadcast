## Table 2-80 — FmxBufferSize descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.50, Table 2-80; PDF p.97._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| FmxBufferSize_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;DefaultFlexMuxBufferDescriptor() |  |  |
| &nbsp;&nbsp;for (i = 0; i < descriptor_length; i += 4) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;FlexMuxBufferDescriptor() |  |  |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

The `*FlexMuxBufferDescriptor()` structures are defined in §11.2 of ISO/IEC 14496-1 — carried here as opaque bytes.
