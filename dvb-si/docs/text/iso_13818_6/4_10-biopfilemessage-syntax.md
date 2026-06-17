## Table 4.10 — BIOP::FileMessage syntax
_§4.7.4.2, PDF p. 38_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| `magic` | 4×8 | `0x42494F50` | "BIOP" |
| `biop_version.major` | 8 | `0x01` | |
| `biop_version.minor` | 8 | `0x00` | |
| `byte_order` | 8 | `0x00` | big-endian |
| `message_type` | 8 | `0x00` | |
| `message_size` | 32 | * | |
| `objectKey_length` | 8 | N1 | ≤ 0x04 |
| `objectKey_data_byte` × N1 | 8 each | + | |
| `objectKind_length` | 32 | `0x00000004` | |
| `objectKind_data` | 4×8 | `0x66696C00` | "fil" |
| `objectInfo_length` | 16 | N2 | |
| `DSM::File::ContentSize` | 64 | + | first 8 bytes of objectInfo |
| `objectInfo_data_byte` × (N2−8) | 8 each | + | |
| `serviceContextList_count` | 8 | N3 | |
| per context: `context_id` | 32 | | |
| per context: `context_data_length` | 16 | N9 | |
| per context: `context_data_byte` × N9 | 8 each | + | |
| `messageBody_length` | 32 | * | |
| `content_length` | 32 | N4 | |
| `content_data_byte` × N4 | 8 each | + | actual file content |

Note: `objectInfo_length` (N2) is ≥ 8 because `DSM::File::ContentSize` (8 bytes)
is the leading part of objectInfo.

