## Table 64 — Deferred_association_tags_descriptor
_§11.3.3, PDF pp. 73-73_

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| deferred_association_tags_descriptor() { |  |  |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| association_tags_loop_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |  |  |
| association_tag | 16 | uimsbf |
| } |  |  |
| transport_stream_id | 16 | uimsbf |
| program_number | 16 | uimsbf |
| for (i=0;i<N;i++) { |  |  |
| private_data_byte | 8 | uimsbf |
| } |  |  |
| } |  |  |

