## Table 54 — DVB-TVA-init
_§9.4.2.1, PDF pp. 60-60_

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| DVB-TVA-init { | | |
| EncodingVersion | 8 | uimsbf |
| IndexingFlag | 1 | bslbf |
| reserved | 7 | |
| DecoderInitptr | 8 | bslbf |
| if( EncodingVersion == '0x01' \|\| | | |
| EncodingVersion == '0xF0') { | | |
| BufferSizeFlag | 1 | bslbf |
| PositionCodeFlag | 1 | bslbf |
| reserved | 6 | |
| CharacterEncoding | 8 | uimsbf |
| if (BufferSizeFlag == '1') { | | |
| BufferSize | 24 | uimsbf |
| } | | |
| } | | |
| if(IndexingFlag) { | | |
| IndexingVersion | 8 | uimsbf |
| } | | |
| Reserved | 0 or 8+ | |
| DecoderInit( ) | | bslbf |
| } | | |

