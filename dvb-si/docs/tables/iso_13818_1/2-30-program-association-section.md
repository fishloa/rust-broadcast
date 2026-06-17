## Table 2-30 — Program association section
_§2.4.4.3–2.4.4.4, PDF pp. 55-57_

| Syntax | Bits | Mnemonic |
|---|---|---|
| program_association_section() { |  |  |
| table_id | 8 | uimsbf |
| section_syntax_indicator | 1 | bslbf |
| '0' | 1 | bslbf |
| reserved | 2 | bslbf |
| section_length | 12 | uimsbf |
| transport_stream_id | 16 | uimsbf |
| reserved | 2 | bslbf |
| version_number | 5 | uimsbf |
| current_next_indicator | 1 | bslbf |
| section_number | 8 | uimsbf |
| last_section_number | 8 | uimsbf |
| for (i = 0; i < N; i++) { |  |  |
| program_number | 16 | uimsbf |
| reserved | 3 | bslbf |
| if (program_number=='0') { |  |  |
| network_PID | 13 | uimsbf |
| } |  |  |
| else { |  |  |
| program_map_PID | 13 | uimsbf |
| } |  |  |
| } |  |  |
| CRC_32 | 32 | rpchof |
| } |  |  |

Cross-check notes (§2.4.4.5): table_id = 0x00; section_syntax_indicator =
`'1'`; section_length's first two bits `'00'`, value shall not exceed
**1021 (0x3FD)**; version_number increments by 1 modulo 32 whenever the PAT
definition changes; program_number 0x0000 ⇒ network_PID, otherwise
program_map_PID; CRC_32 gives zero output of the Annex A decoder registers
over the entire section.

table_id assignments (§2.4.4.4, Table 2-31): 0x00 PAT, 0x01 CA_section,
0x02 TS_program_map_section, 0x03 TS_description_section, 0x04
ISO_IEC_14496_scene_description_section, 0x05
ISO_IEC_14496_object_descriptor_section, 0x06 Metadata_section, 0x07
IPMP_Control_Information_section, 0x08–0x3F ISO/IEC 13818-1 reserved,
0x40–0xFE user private, 0xFF forbidden.

