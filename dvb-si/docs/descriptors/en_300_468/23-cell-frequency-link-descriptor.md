## Table 23 — Cell frequency link descriptor
_§6.2.7, PDF pp. 58-58_

| Syntax | Number of bits | Identifier |
|---|---|---|
| cell_frequency_link_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| cell_id | 16 | uimsbf |
| frequency | 32 | uimsbf |
| subcell_info_loop_length | 8 | uimsbf |
| for (j=0;j<N;j++) { |
| cell_id_extension | 8 | uimsbf |
| transposer_frequency | 32 | uimsbf |
| } |
| } |
| } |

