## Table 4.13 — BIOP::StreamEventMessage syntax
_§4.7.4.5, PDF p. 41 — `objectKind = "ste"`_

Identical to StreamMessage through the `DSM::Stream::Info_T` block, then adds an
event-name list, and ends with an `eventId` list after the tap loop:

| Syntax | bits | Value | Comment |
|---|---|---|---|
| common header + objectKey + objectKind | | `0x73746500` | "ste\0" |
| `objectInfo_length` | 16 | N6 | |
| `DSM::Stream::Info_T { … }` | | | exactly as Table 4.11 (N2 + 10 bytes) |
| **DSM::Event::EventList_T {** | | | |
| `eventNames_count` | 16 | N3 | |
| per name: `eventName_length` | 8 | N4 | |
| `eventName_data_byte` × N4 | 8 each | + | NUL-terminated |
| **}** | | | |
| `objectInfo_byte` × (N6 − (N2+10) − (2 + ΣeventName)) | 8 each | + | trailing objectInfo |
| `serviceContextList_count` | 8 | N | + loop |
| `messageBody_length` | 32 | * | |
| `taps_count` | 8 | N5 | tap = id(16)/use(16, Table 4.12)/association_tag(16)/selector_length(8)=0 |
| `eventIds_count` | 8 | N3 | = `eventNames_count` |
| `eventId` × N3 | 16 each | + | correlates to the event names |

DVB note: the eventId sequence count equals the eventNames count. DSM-CC events
are **not** DVB-SI events.

