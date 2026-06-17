## Table 4.9 — BIOP::DirectoryMessage syntax
_§4.7.4.1, PDF p. 36 — ServiceGateway is identical except `objectKind = "srg"` (§4.7.4.4)_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| `magic` | 4×8 | `0x42494F50` | "BIOP" |
| `biop_version.major` | 8 | `0x01` | |
| `biop_version.minor` | 8 | `0x00` | |
| `byte_order` | 8 | `0x00` | big-endian |
| `message_type` | 8 | `0x00` | |
| `message_size` | 32 | * | bytes following this field |
| `objectKey_length` | 8 | N1 | ≤ 0x04 |
| `objectKey_data_byte` × N1 | 8 each | + | |
| `objectKind_length` | 32 | `0x00000004` | |
| `objectKind_data` | 4×8 | `0x64697200` | "dir" (or `0x73726700` "srg" for ServiceGateway) |
| `objectInfo_length` | 16 | N2 | |
| `objectInfo_data_byte` × N2 | 8 each | + | |
| `serviceContextList_count` | 8 | N3 | |
| per context: `context_id` | 32 | | |
| per context: `context_data_length` | 16 | N9 | |
| per context: `context_data_byte` × N9 | 8 each | + | |
| `messageBody_length` | 32 | * | |
| `bindings_count` | 16 | N4 | |
| **per binding: BIOP::Name {** | | | |
| `nameComponents_count` | 8 | N5 | DVB: = 1 |
| per name-comp: `id_length` | 8 | N6 | |
| `id_data_byte` × N6 | 8 each | + | |
| per name-comp: `kind_length` | 8 | N7 | |
| `kind_data_byte` × N7 | 8 each | + | as type_id (Table 4.4) |
| **}** | | | |
| `bindingType` | 8 | + | `0x01` nobject / `0x02` ncontext |
| `IOP::IOR()` | | + | objectRef (Table 4.3) |
| `objectInfo_length` | 16 | N8 | |
| `objectInfo_data_byte` × N8 | 8 each | + | |

Strings are NUL-terminated (`0x00`). DVB: `nameComponents_count = 1`; receivers
must skip over `serviceContextList` and `objectInfo`.

