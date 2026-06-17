## Table 4.7 — Lite Options Profile Body (with ServiceLocation) syntax
_§4.7.3.3, PDF p. 34_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| `profileId_tag` | 32 | `0x49534F05` | TAG_LITE_OPTIONS |
| `profile_data_length` | 32 | * | |
| `profile_data_byte_order` | 8 | `0x00` | big-endian |
| `component_count` | 8 | N1 | |
| **DSM::ServiceLocation {** | | | (must be the first component) |
| `componentId_tag` | 32 | `0x49534F46` | TAG_ServiceLocation |
| `component_data_length` | 32 | * | |
| `serviceDomain_length` | 8 | `0x14` | 20 — length of the carousel NSAP address |
| `serviceDomain_data()` | 160 | + | DVBcarouselNSAPaddress (Table 4.8) |
| **CosNaming::Name() {** | | | pathName |
| `nameComponents_count` | 32 | N2 | |
| per component: `id_length` | 32 | N3 | |
| `id_data_byte` × N3 | 8 each | + | |
| per component: `kind_length` | 32 | N4 | |
| `kind_data_byte` × N4 | 8 each | + | as type_id (Table 4.4) |
| **}** | | | |
| `initialContext_length` | 32 | N5 | |
| `InitialContext_data_byte` × N5 | 8 each | | |
| **}** | | | |
| then (N6 = N1−1)× BIOP::LiteOptionComponent: `componentId_tag` | 32 | + | |
| `component_data_length` | 8 | N7 | |
| `component_data_byte` × N7 | 8 each | | |

