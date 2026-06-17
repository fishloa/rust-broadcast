## Table 4.5 — BIOP Profile Body syntax
_§4.7.3.2, PDF p. 32_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| `profileId_tag` | 32 | `0x49534F06` | TAG_BIOP |
| `profile_data_length` | 32 | * | |
| `profile_data_byte_order` | 8 | `0x00` | big-endian |
| `liteComponents_count` | 8 | N1 | |
| **BIOP::ObjectLocation {** | | | (1st component, mandatory) |
| `componentId_tag` | 32 | `0x49534F50` | TAG_ObjectLocation |
| `component_data_length` | 8 | * | |
| `carouselId` | 32 | + | |
| `moduleId` | 16 | + | |
| `version.major` | 8 | `0x01` | |
| `version.minor` | 8 | `0x00` | |
| `objectKey_length` | 8 | N2 | ≤ `0x04` (DVB) |
| `objectKey_data_byte` × N2 | 8 each | + | |
| **}** | | | |
| **DSM::ConnBinder {** | | | (2nd component, mandatory) |
| `componentId_tag` | 32 | `0x49534F40` | TAG_ConnBinder |
| `component_data_length` | 8 | * | |
| `taps_count` | 8 | N3 | |
| first BIOP::Tap: `id` | 16 | `0x0000` | user private |
| first BIOP::Tap: `use` | 16 | `0x0016` | BIOP_DELIVERY_PARA_USE |
| first BIOP::Tap: `association_tag` | 16 | + | |
| first BIOP::Tap: `selector_length` | 8 | `0x0A` | |
| first BIOP::Tap: `selector_type` | 16 | `0x0001` | MESSAGE |
| first BIOP::Tap: `transactionId` | 32 | * | transactionId of the DII carrying the module |
| first BIOP::Tap: `timeout` | 32 | * | µs |
| then (N3−1)× BIOP::Tap: `id` | 16 | `0x0000` | |
| `use` | 16 | + | |
| `association_tag` | 16 | + | |
| `selector_length` | 8 | N4 | |
| `selector_data_byte` × N4 | 8 each | | |
| **}** | | | |
| then (N5 = N1−2)× BIOP::LiteComponent: `componentId_tag` | 32 | + | |
| `component_data_length` | 8 | N6 | |
| `component_data_byte` × N6 | 8 each | | |

DVB guidelines: `byte_order = 0x00`; the first two components are exactly one
ObjectLocation then one ConnBinder, in that order; `objectKey_length ≤ 0x04`;
the BIOP Profile Body refers only to objects in the same carousel (its
`carouselId` equals the carousel's); if a BIOP_DELIVERY_PARA_USE tap is present
it is the first tap in the ConnBinder; the `id` field is 0 if unused.

