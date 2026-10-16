## Table 82 — Syntax of the pre-fetch descriptor
_§10.8.3.2, PDF pp. 169-169_

| Syntax | Bits | Mnemonic |
|---|---|---|
| prefetch_descriptor () { |  |  |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| transport_protocol_label | 8 | uimsbf |
| for(i=0; i<N; i++ ) { |  |  |
| label_length | (blank in the table; "8-bit field" per prose, p.170) |  |
| for(j=0; j<label_length; j++ ) { |  |  |
| label_byte | 8 | uimsbf |
| } |  |  |
| prefetch_priority | 8 | uimsbf |
| } |  |  |
| } |  |  |

descriptor_tag = 0x0C. prefetch_priority: a value between 1 and 100 inclusive (100 highest).
