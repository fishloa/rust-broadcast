## Table 83 — Syntax of the DII location descriptor
_§10.8.3.3, PDF pp. 170-170_

| Syntax | Bits | Mnemonic |
|---|---|---|
| DII_location_descriptor () { |  |  |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| transport_protocol_label | 8 | uimsbf |
| for(i=0; i<N; i++ ) { |  |  |
| reserved_future_use | 1 | bslbf |
| DII_identification | 15 | uimsbf |
| association_tag | 16 | uimsbf |
| } |  |  |
| } |  |  |

descriptor_tag = 0x0D.
