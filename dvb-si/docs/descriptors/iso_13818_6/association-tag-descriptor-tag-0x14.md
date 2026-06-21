## Table 4.18 — association_tag_descriptor (tag 0x14)
_ISO/IEC 13818-6 §11.4.1, as reproduced in ETSI TR 101 202 V1.2.1 §4.7.7.2, Table 4.18 (PDF p. 47)._
_ISO/IEC 13818-6 itself cannot be vendored; TR 101 202 (free ETSI) is the authoritative free reproduction of this DSM-CC descriptor used by DVB._

Inserted in the PMT 2nd (ES_info) descriptor loop to label an Elementary Stream
with a 16-bit `association_tag`, associating all Taps carrying that tag with the
stream. The DVB analogue of EN 300 468's `stream_identifier_descriptor` (which
uses an 8-bit `component_tag`); the wider 16-bit tag identifies the PID carrying
the object carousel's ServiceGateway so receivers can bootstrap efficiently.

| Syntax | bits | Type | Value | Comment |
|---|---|---|---|---|
| `association_tag_descriptor() {` | | | | |
| &nbsp;&nbsp;`descriptor_tag` | 8 | uimsbf | `0x14` | |
| &nbsp;&nbsp;`descriptor_length` | 8 | uimsbf | * | |
| &nbsp;&nbsp;`association_tag` | 16 | uimsbf | + | |
| &nbsp;&nbsp;`use` | 16 | uimsbf | `0x0000` | DSI with IOR of SGW |
| | | | `0x0100`–`0x1FFF` | DVB reserved |
| | | | `0x2000`–`0xFFFF` | user private |
| &nbsp;&nbsp;`if (use == 0x0000) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`selector_length` | 8 | uimsbf | `0x08` | |
| &nbsp;&nbsp;&nbsp;&nbsp;`transaction_id` | 32 | uimsbf | + | transaction_id of DSI |
| &nbsp;&nbsp;&nbsp;&nbsp;`timeout` | 32 | uimsbf | + | timeout for DSI |
| &nbsp;&nbsp;`} else if (use == 0x0001) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`selector_length` | 8 | uimsbf | `0x00` | |
| &nbsp;&nbsp;`} else {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`selector_length` | 8 | uimsbf | N1 | |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<N1; i++) { selector_byte` | 8 | uimsbf | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`} }` | | | | |
| &nbsp;&nbsp;`for (i=0; i<N2; i++) { private_data_byte` | 8 | uimsbf | + | private data |
| &nbsp;&nbsp;`} }` | | | | |

### Semantics (TR 101 202 §4.7.7.2)

- **`use`** selects the syntax and semantics of the selector field:
  - `0x0000` — the `DownloadServerInitiate()` (DSI) message carrying the IOR of
    the Service Gateway is broadcast on this PID; the selector bytes then carry
    `transaction_id` + `timeout` (so `selector_length` is `0x08`).
  - `0x0001` — `selector_length` is `0x00` (no selector bytes).
  - otherwise — `selector_length` = N1 explicit `selector_byte`s.
- **`transaction_id`** (when `use == 0x0000`) corresponds to the `transaction_id`
  of the DSI message conveying the IOR of the U-U object carousel's Service
  Gateway. `0xFFFFFFFF` means the DSI `transaction_id` is not known at this point
  but all DSI messages on the PID are valid (permits a 'static' PMT whose DSI may
  change without a PMT update).
- **`timeout`** (when `use == 0x0000`) is the time-out period in **microseconds**
  for acquiring the DSI message. `0xFFFFFFFF` means no timeout value is known.
- **DVB Guideline:** the default `use` value is `0x0100` (the PID may or may not
  broadcast a DSI). DVB reserves `0x0101`–`0x01FF` for future use.

### Parse/serialize notes

- The body after `descriptor_length` is `2 (association_tag) + 2 (use) +
  1 (selector_length) + selector_length + private_data`. The three `use` branches
  differ only in the conventional `selector_length` value — structurally every
  branch is `selector_length` followed by `selector_length` selector bytes, so a
  single field-driven layout (`association_tag`, `use`, `selector` slice,
  `private_data` slice) round-trips all three byte-exactly. `transaction_id`/
  `timeout` are the two big-endian u32s inside the 8-byte selector when
  `use == 0x0000`; expose them as typed accessors, not a separate wire shape.
