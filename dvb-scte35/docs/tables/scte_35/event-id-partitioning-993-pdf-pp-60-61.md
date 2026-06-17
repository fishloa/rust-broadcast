## Event id partitioning — §9.9.3, PDF pp. 60-61

Applies to `splice_event_id` in splice_insert()/splice_schedule() and to
`segmentation_event_id` carried in the segmentation_descriptor with a
time_signal() command. Only one occurrence of a given event id value shall be
active at any one time; once an event ends, the value becomes inactive and
eligible for reuse (but shall not be reused immediately if multiple messages
referencing the same Splice Point end some events and start new ones). The
32-bit event id value shall be partitioned:

| Syntax | Bits | Type |
|---|---|---|
| Event_source | 4 | uimsbf |
| Event_number | 28 | uimsbf |

- **Event_source** — a user assigned number for the source of the Cue
  Message.
- **Event_number** — a number chosen by the Event source to identify an
  instance of the Cue Message.

Assigned Event_source values:

| Source of Cue Message | Event_source value | Event id ranges |
|---|---|---|
| Cue embedded in original source material by network programming systems | 0 | 0x00000000, 0x00000001 … 0x0fffffff |
| Cue created by automation system switching | 4 | 0x40000000, 0x40000001 … 0x4fffffff |
| Cue created by manual event trigger system | 6 | 0x60000000, 0x60000001 … 0x6fffffff |
| Cue created by local content replacement system | 12 | 0xC0000000, 0xC0000001 … 0xCfffffff |

