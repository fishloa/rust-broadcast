## Table 4.11 — BIOP::StreamMessage syntax
_§4.7.4.3, PDF p. 39 — `objectKind = "str"`_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| common header (magic..message_size) | | | as Table 4.9 |
| `objectKey_length` | 8 | N1 | |
| `objectKey_data_byte` × N1 | 8 each | + | |
| `objectKind_length` | 32 | `0x00000004` | |
| `objectKind_data` | 4×8 | `0x73747200` | "str\0" |
| `objectInfo_length` | 16 | N6 | |
| **DSM::Stream::Info_T {** | | | objectInfo head |
| `aDescription_length` | 8 | N2 | |
| `aDescription_bytes` × N2 | 8 each | + | |
| `duration.aSeconds` | 32 | + | AppNPT seconds (**signed**, `simsbf`) |
| `duration.aMicroSeconds` | 16 | + | AppNPT microseconds |
| `audio` | 8 | + | |
| `video` | 8 | + | |
| `data` | 8 | + | |
| **}** | | | (Info_T = N2 + 10 bytes) |
| `objectInfo_byte` × (N6 − (N2+10)) | 8 each | + | trailing objectInfo |
| `serviceContextList_count` | 8 | N3 | + loop (context_id 32, context_data_length 16, data) |
| `messageBody_length` | 32 | * | |
| `taps_count` | 8 | N4 | |
| per tap: `id` | 16 | `0x0000` | |
| per tap: `use` | 16 | + | Table 4.12 (ES_USE / PROGRAM_USE / STR_*_USE) |
| per tap: `association_tag` | 16 | + | |
| per tap: `selector_length` | 8 | `0x00` | no selector |

