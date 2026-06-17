## Table 4.17 — carousel_identifier_descriptor (tag 0x13)
_§4.7.7.1, PDF p. 45 — inserted in the PMT 2nd (ES_info) descriptor loop of the DSI's elementary stream_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| `descriptor_tag` | 8 | `0x13` | |
| `descriptor_length` | 8 | * | |
| `carousel_id` | 32 | + | = the object carousel's carouselId |
| `FormatId` | 8 | + | identifies the FormatSpecifier — Table 4.17a |
| `FormatSpecifier()` | 8×N2 | + | length depends on `FormatId` (Table 4.17a) |
| `private_data_byte` × N1 | 8 each | + | remainder of the descriptor |

`FormatId` selects the FormatSpecifier layout (Table 4.17a): `0x00` = none (no
FormatSpecifier bytes); `0x01` = the aggregated ServiceGateway-location fields
(see Table 4.17a below); `0x02–0x7F` reserved; `0x80–0xFF` private.

