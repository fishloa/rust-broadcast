# NORM — NACK-Oriented Reliable Multicast Protocol

_Source: RFC 5740 §4 (Figures 1-20), transcribed_

NORM provides reliable bulk-data / stream transfer over IP multicast using
NACK-based repair plus FEC. The NORM version number for this specification is **1**.
All integer fields are big-endian (network order). NORM uses its **own** common
message header (NOT the LCT header used by ALC/FLUTE), but borrows the same
HET/HEL header-extension convention and FEC Payload ID concept.

Reserved NormNodeId values (§): `0x00000000` = NORM_NODE_NONE (invalid),
`0xffffffff` = NORM_NODE_ANY (wildcard).

## Common Message Header (§4.1, Figure 1)

All NORM messages begin with this header:

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|version|  type |    hdr_len    |          sequence             |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                           source_id                           |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | No. of bits | Mnemonic | Meaning |
|-------|-------------|----------|---------|
| version | 4 | uimsbf | Protocol version = **1**. |
| type | 4 | uimsbf | Message type (see table). |
| hdr_len | 8 | uimsbf | Header length in 32-bit words. Greater than the message-type base ⇒ header extensions present. |
| sequence | 16 | uimsbf | Set by originator; sender loss-detection seq / receiver replay-protection seq. |
| source_id | 32 | uimsbf | NormNodeId of the message originator. |

Message `type` values:

| Message | Value |
|---------|-------|
| NORM_INFO | 1 |
| NORM_DATA | 2 |
| NORM_CMD | 3 |
| NORM_NACK | 4 |
| NORM_ACK | 5 |
| NORM_REPORT | 6 |

## Header Extensions (§4.1, Figures 2-3)

Same dual-format scheme as LCT, keyed on the 8-bit `het` range.

Variable-length (`het` 0..127):

```
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|   het <=127   |      hel      |                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+                               |
|                    Header Extension Content                   |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

Fixed-length (`het` 128..255, exactly one 32-bit word):

```
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|   het >=128   |    reserved   |    Header Extension Content   |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | bits | Meaning |
|-------|------|---------|
| het | 8 | Extension type. 0..127 = variable; 128..255 = fixed (4 bytes). |
| hel | 8 | Length of whole extension in 32-bit words. Present ONLY for `het` 0..127. |
| reserved | 8 | Present (in place of hel) for `het` 128..255. |
| Content | var / 16 | Extension-specific. |

## Shared sender fields

NORM_DATA, NORM_INFO and NORM_CMD share a second header word after the common
header:

```
|          instance_id          |     grtt      |backoff| gsize |
```

| Field | bits | Meaning |
|-------|------|---------|
| instance_id | 16 | Sender's current participation instance; lets receivers detect rejoin. |
| grtt | 8 | Quantized group RTT estimate (encode/decode per RFC 5401). |
| backoff | 4 | NACK backoff factor K_sender (0..15, × GRTT). |
| gsize | 4 | Quantized group-size estimate. MSB (0/1) = mantissa 1 or 5; low 3 bits +1 = base-10 exponent. |

---

## NORM_DATA (type = 2) (§4.2.1, Figure 4)

```
|version| type=2|    hdr_len    |          sequence             |
|                           source_id                           |
|          instance_id          |     grtt      |backoff| gsize |
|     flags     |    fec_id     |     object_transport_id       |
|                         fec_payload_id                        |
|                header_extensions (if applicable)              |
|          payload_len*         |       payload_msg_start*      |
|                        payload_offset*                        |
|                          payload_data*                        |
```

| Field | bits | Meaning |
|-------|------|---------|
| (common header) | 64 | version=1, type=2, hdr_len, sequence, source_id |
| (shared word) | 32 | instance_id, grtt, backoff, gsize |
| flags | 8 | Object flags (see table). |
| fec_id | 8 | FEC Encoding ID; implies size/format of fec_payload_id. |
| object_transport_id | 16 | Monotonic NormTransportId of the object. |
| fec_payload_id | var | FEC coding-block + symbol identifier; size/format per fec_id. |
| header_extensions | var | e.g. EXT_FTI. |
| payload_len* | 16 | STREAM only: source content length, or 0 ⇒ stream_control_code. |
| payload_msg_start* | 16 | STREAM only: message-boundary offset+1, or stream_control_code when payload_len=0. |
| payload_offset* | 32 | STREAM only: byte offset of content from stream start. |
| payload_data* | var | Source or parity content (≤ NormSegmentSize). |

`hdr_len` base = 4 words **plus** the size of `fec_payload_id` (e.g. 6 when
`fec_id`=129). The `payload_len`/`payload_msg_start`/`payload_offset` fields are
present **only for NORM_OBJECT_STREAM objects** (indicated by NORM_FLAG_STREAM) and
do NOT contribute to `hdr_len`. The only defined stream_control_code is
NORM_STREAM_END = 0.

`flags` field values:

| Flag | Value | Purpose |
|------|-------|---------|
| NORM_FLAG_REPAIR | 0x01 | Message is a repair transmission. |
| NORM_FLAG_EXPLICIT | 0x02 | Repair segment meeting a specific erasure (vs general parity). |
| NORM_FLAG_INFO | 0x04 | NORM_INFO is available for this object. |
| NORM_FLAG_UNRELIABLE | 0x08 | No repair will be supplied (one-shot best-effort). |
| NORM_FLAG_FILE | 0x10 | Object is file-based (hint: use disk storage). |
| NORM_FLAG_STREAM | 0x20 | Object is NORM_OBJECT_STREAM (overrides FILE; enables payload_* fields). |

### FEC Payload ID — example for fec_id = 129 (§4.2.1, Figure 5)

⚠ The FEC Payload ID layout is **defined per FEC Scheme, not by RFC 5740 itself**;
RFC 5740 reproduces one example from RFC 5445 (Small Block Systematic, fec_id=129):

```
|                       source_block_number                     |
|        source_block_len       |      encoding_symbol_id       |
```

| Field | bits | Meaning |
|-------|------|---------|
| source_block_number | 32 | Coding-block position within the object. |
| source_block_len | 16 | Number of user-data segments in the block. |
| encoding_symbol_id | 16 | Symbol/segment index; `< source_block_len` ⇒ source, else parity. |

Other `fec_id` values give other layouts (out of scope here).

### EXT_FTI Header Extension — general part (§4.2.1, Figure 6)

Variable-length extension, `het` = **64**. MAY be applied to NORM_DATA and NORM_INFO.

```
|    het = 64   |    hel = 4    |       object_size (msb)       |
|                       object_size (lsb)                       |
|                  FEC scheme-specific content ...              |
```

| Field | bits | Meaning |
|-------|------|---------|
| het | 8 | = 64 |
| hel | 8 | Length in 32-bit words; depends on fec_id (= 4 for fec_id=129). |
| object_size | 48 | Total object length in bytes (or stream buffer size for STREAM). |
| FEC-scheme-specific content | var | Per fec_id. |

### EXT_FTI — example body for fec_id = 129 (§4.2.1, Figure 7)

```
|    het = 64   |    hel = 4    |       object_size (msb)       |
|                       object_size (lsb)                       |
|       fec_instance_id         |          segment_size         |
|       fec_max_block_len       |         fec_num_parity        |
```

| Field | bits | Meaning |
|-------|------|---------|
| object_size | 48 | Total object/stream-buffer size in bytes. |
| fec_instance_id | 16 | FEC Instance ID (RFC 5052) — the specific small-block systematic code. |
| segment_size | 16 | Max message payload content in bytes (NormSegmentSize). |
| fec_max_block_len | 16 | Max user-data segments per FEC coding block. |
| fec_num_parity | 16 | Max encoding symbols (parity) generable per source block. |

---

## NORM_INFO (type = 1) (§4.2.2, Figure 8)

Conveys OPTIONAL out-of-band context for an object (one per NormObject, atomic,
≤ NormSegmentSize).

```
|version| type=1|    hdr_len    |          sequence             |
|                           source_id                           |
|          instance_id          |     grtt      |backoff| gsize |
|     flags     |     fec_id    |     object_transport_id       |
|                header_extensions (if applicable)              |
|                         payload_data                          |
```

| Field | bits | Meaning |
|-------|------|---------|
| (common header) | 64 | type=1. `hdr_len` = 4 when no extensions. |
| (shared word) | 32 | instance_id, grtt, backoff, gsize. |
| flags | 8 | Same as NORM_DATA. |
| fec_id | 8 | FEC Encoding ID. |
| object_transport_id | 16 | Same object the INFO is associated with. |
| header_extensions | var | EXT_FTI MAY be applied. |
| payload_data | var | Application-defined content (≤ NormSegmentSize). |

---

## NORM_CMD (type = 3) (§4.2.3, Figure 9)

Common NORM_CMD prefix (all sub-types share this; `sub-type` selects the body):

```
|version| type=3|    hdr_len    |          sequence             |
|                           source_id                           |
|          instance_id          |     grtt      |backoff| gsize |
|    sub-type   |                                               |
+-+-+-+-+-+-+-+-+        NORM_CMD Content                       +
|                              ...                              |
```

`sub-type` is an 8-bit field selecting the command:

| Command | Sub-type | Purpose |
|---------|----------|---------|
| NORM_CMD(FLUSH) | 1 | Temporary end-of-transmission; excite receivers for outstanding repairs; optional positive-ack collection. |
| NORM_CMD(EOT) | 2 | Permanent end-of-transmission. |
| NORM_CMD(SQUELCH) | 3 | Advertise current repair window in response to out-of-range NACKs. |
| NORM_CMD(CC) | 4 | GRTT measurement + congestion-control feedback collection. |
| NORM_CMD(REPAIR_ADV) | 5 | Advertise aggregated repair/feedback state for unicast-feedback suppression. |
| NORM_CMD(ACK_REQ) | 6 | Request application-defined positive ack from a receiver list. |
| NORM_CMD(APPLICATION) | 7 | Application-defined command content. |

### NORM_CMD(FLUSH) sub-type=1 (Figure 10)

```
|  sub-type = 1 |    fec_id     |      object_transport_id      |
|                         fec_payload_id                        |
|                acking_node_list (if applicable)               |
```

| Field | bits | Meaning |
|-------|------|---------|
| sub-type | 8 | = 1 |
| fec_id | 8 | Implies fec_payload_id size/format. |
| object_transport_id | 16 | Current logical transmit position (object). |
| fec_payload_id | var | Current logical transmit position (block/symbol). |
| acking_node_list | var | OPTIONAL list of NormNodeIds requested to positively ack. Length inferred from message length. |

`hdr_len` (no ext) = 4 + size of fec_payload_id.

### NORM_CMD(EOT) sub-type=2 (Figure 11)

```
|  sub-type = 2 |                    reserved                   |
```

| Field | bits | Meaning |
|-------|------|---------|
| sub-type | 8 | = 2 |
| reserved | 24 | MUST be 0; ignored on reception. |

`hdr_len` (no ext) = 4.

### NORM_CMD(SQUELCH) sub-type=3 (Figure 12)

```
| sub-type = 3  |     fec_id    |      object_transport_id      |
|                         fec_payload_id                        |
|                        invalid_object_list                    |
```

| Field | bits | Meaning |
|-------|------|---------|
| sub-type | 8 | = 3 |
| fec_id | 8 | Implies fec_payload_id size/format. |
| object_transport_id | 16 | Start (earliest) of sender's current repair window. |
| fec_payload_id | var | Repair-window start (encoding_symbol_id SHOULD be 0). |
| invalid_object_list | var | List of 16-bit NormTransportIds in-window but no longer repairable. Length from packet length, ≤ NormSegmentSize. |

`hdr_len` (no ext) = 4 + size of fec_payload_id.

### NORM_CMD(CC) sub-type=4 (Figure 13)

```
|  sub-type = 4 |    reserved   |          cc_sequence          |
|                         send_time_sec                         |
|                        send_time_usec                         |
|               header extensions (if applicable)               |
|                  cc_node_list (if applicable)                 |
```

| Field | bits | Meaning |
|-------|------|---------|
| sub-type | 8 | = 4 |
| reserved | 8 | MUST be 0. |
| cc_sequence | 16 | Sender CC feedback round number. |
| send_time_sec | 32 | Timestamp seconds (since sender reference, usually 1970-01-01). |
| send_time_usec | 32 | Timestamp microseconds. |
| header extensions | var | e.g. EXT_RATE. |
| cc_node_list | var | OPTIONAL list of per-receiver CC items (see below). |

`hdr_len` (no ext) = 6.

**EXT_RATE Header Extension** (fixed-length, `het` = 128):

```
|    het = 128  |    reserved   |           send_rate           |
```

| Field | bits | Meaning |
|-------|------|---------|
| het | 8 | = 128 |
| reserved | 8 | — |
| send_rate | 16 | Sender tx rate, bytes/s. 12-bit mantissa (high) + 4-bit base-10 exponent (low): `send_rate = (((int)(M*4096/10 + 0.5)) << 4) | E`. |

**cc_node_list item** format:

```
|                          cc_node_id                           |
|    cc_flags   |     cc_rtt    |            cc_rate            |
```

| Field | bits | Meaning |
|-------|------|---------|
| cc_node_id | 32 | NormNodeId of the receiver. |
| cc_flags | 8 | CC status flags (see table). |
| cc_rtt | 8 | Quantized RTT (valid only if NORM_FLAG_CC_RTT set). |
| cc_rate | 16 | Receiver CC rate (same encoding as send_rate). |

`cc_flags` values:

| Flag | Value | Purpose |
|------|-------|---------|
| NORM_FLAG_CC_CLR | 0x01 | Current limiting receiver. |
| NORM_FLAG_CC_PLR | 0x02 | Potential limiting receiver. |
| NORM_FLAG_CC_RTT | 0x04 | Receiver has measured RTT to sender. |
| NORM_FLAG_CC_START | 0x08 | Slow-start phase (cc_rate is measured rate). |
| NORM_FLAG_CC_LEAVE | 0x10 | Receiver leaving; ignore its feedback. |

### NORM_CMD(REPAIR_ADV) sub-type=5 (Figure 14)

```
| sub-type = 5  |     flags     |            reserved           |
|               header extensions (if applicable)               |
|                       repair_adv_payload                      |
```

| Field | bits | Meaning |
|-------|------|---------|
| sub-type | 8 | = 5 |
| flags | 8 | NORM_REPAIR_ADV_FLAG_LIMIT = 0x01 (sender's full repair state didn't fit one segment). |
| reserved | 16 | — |
| header extensions | var | EXT_CC SHOULD be applied under CC operation. |
| repair_adv_payload | var | Same form as NORM_NACK `nack_content`. |

`hdr_len` (no ext) = 4.

**EXT_CC Header Extension** (variable-length, `het` = 3, `hel` = 3) — used in
NORM_NACK, NORM_ACK and NORM_CMD(REPAIR_ADV):

```
|     het = 3   |    hel = 3    |          cc_sequence          |
|    cc_flags   |     cc_rtt    |            cc_loss            |
|            cc_rate            |          cc_reserved          |
```

| Field | bits | Meaning |
|-------|------|---------|
| het | 8 | = 3 |
| hel | 8 | = 3 |
| cc_sequence | 16 | Greatest cc_sequence the receiver got from NORM_CMD(CC). |
| cc_flags | 8 | Same values as cc_node_list item flags. |
| cc_rtt | 8 | Quantized RTT (default max if no RTT info). |
| cc_loss | 16 | Loss fraction × 65535 (`floor(loss * 65535)`). |
| cc_rate | 16 | Receiver CC rate (send_rate encoding). |
| cc_reserved | 16 | MUST be 0; ignored. |

### NORM_CMD(ACK_REQ) sub-type=6 (Figure 15)

```
| sub-type = 6  |    reserved   |    ack_type   |    ack_id     |
|                       acking_node_list                        |
```

| Field | bits | Meaning |
|-------|------|---------|
| sub-type | 8 | = 6 |
| reserved | 8 | MUST be 0; ignored. |
| ack_type | 8 | Type of ack requested (see table). |
| ack_id | 8 | Sequenced id echoed back in NORM_ACK. |
| acking_node_list | var | NormNodeIds expected to ack; length from payload length, ≤ NormSegmentSize. |

`hdr_len` (no ext) = 4.

`ack_type` values (shared with NORM_ACK):

| ACK Type | Value | Purpose |
|----------|-------|---------|
| NORM_ACK(CC) | 1 | Ack in response to NORM_CMD(CC). |
| NORM_ACK(FLUSH) | 2 | Ack in response to NORM_CMD(FLUSH). |
| NORM_ACK(RESERVED) | 3-15 | Reserved for future NORM use. |
| NORM_ACK(APPLICATION) | 16-255 | Application discretion. |

(NORM_ACK(CC) and NORM_ACK(FLUSH) ack_types SHALL NOT be generated as
NORM_CMD(ACK_REQ); they are used only for the auto-generated NORM_ACK responses.)

### NORM_CMD(APPLICATION) sub-type=7 (Figure 16)

```
| sub-type = 7  |                    reserved                   |
|                   Application-Defined Content                 |
```

| Field | bits | Meaning |
|-------|------|---------|
| sub-type | 8 | = 7 |
| reserved | 24 | — |
| Application-Defined Content | var | App-format; ≤ NormSegmentSize. |

`hdr_len` (no ext) = 4.

---

## NORM_NACK (type = 4) (§4.3.1, Figure 17)

Receiver → sender repair request + CC/RTT feedback.

```
|version| type=4|    hdr_len    |            sequence           |
|                           source_id                           |
|                           server_id                           |
|           instance_id         |            reserved           |
|                       grtt_response_sec                       |
|                       grtt_response_usec                      |
|               header extensions (if applicable)               |
|                          nack_payload                         |
```

| Field | bits | Meaning |
|-------|------|---------|
| (common header) | 64 | type=4. `hdr_len` = 6 when no extensions. |
| server_id | 32 | NormNodeId of the destination sender. |
| instance_id | 16 | Sender's current instance_id (sender ignores feedback with wrong value). |
| reserved | 16 | MUST be 0. |
| grtt_response_sec | 32 | Adjusted NORM_CMD(CC) send_time seconds (0 = none received yet). |
| grtt_response_usec | 32 | Adjusted send_time microseconds. |
| header extensions | var | EXT_CC for CC feedback. |
| nack_payload | var | One or more NORM Repair Requests; ≤ NormSegmentSize. |

### NORM Repair Request (§4.3.1, Figure 18)

```
|      form     |     flags     |             length            |
|                      repair_request_items                     |
```

| Field | bits | Meaning |
|-------|------|---------|
| form | 8 | Item form (see table). |
| flags | 8 | Repair level flags (see table). |
| length | 16 | Length in bytes of repair_request_items. |
| repair_request_items | var | List of Repair Request Items (Figure 19). |

`form` values:

| Form | Value | Meaning |
|------|-------|---------|
| NORM_NACK_ITEMS | 1 | Each item is an individual request. |
| NORM_NACK_RANGES | 2 | Items are inclusive-range pairs. |
| NORM_NACK_ERASURES | 3 | Items individual; encoding_symbol_id interpreted as an erasure count for the block. |

`flags` values:

| Flag | Value | Meaning |
|------|-------|---------|
| NORM_NACK_SEGMENT | 0x01 | Listed segment(s)/range needed. |
| NORM_NACK_BLOCK | 0x02 | Listed block(s)/range needed in entirety. |
| NORM_NACK_INFO | 0x04 | NORM_INFO needed for listed object(s). |
| NORM_NACK_OBJECT | 0x08 | Listed object(s)/range needed in entirety (fec_payload_id ignored). |

### NORM Repair Request Item (§4.3.1, Figure 19)

```
|     fec_id    |   reserved    |      object_transport_id      |
|                        fec_payload_id                         |
```

| Field | bits | Meaning |
|-------|------|---------|
| fec_id | 8 | FEC type ⇒ fec_payload_id format. |
| reserved | 8 | MUST be 0; ignored. |
| object_transport_id | 16 | Object being requested. |
| fec_payload_id | var | Coding block / segment; size per fec_id. Ignored if NORM_NACK_OBJECT; only block portion used if NORM_NACK_BLOCK. |

---

## NORM_ACK (type = 5) (§4.3.2, Figure 20)

Receiver → sender, primarily for CC and positive-ack.

```
|version| type=5|    hdr_len    |          sequence             |
|                           source_id                           |
|                           server_id                           |
|           instance_id         |    ack_type  |     ack_id     |
|                       grtt_response_sec                       |
|                       grtt_response_usec                      |
|               header extensions (if applicable)               |
|                   ack_payload (if applicable)                 |
```

| Field | bits | Meaning |
|-------|------|---------|
| (common header) | 64 | type=5. `hdr_len` = 6 when no extensions. |
| server_id | 32 | Destination sender NormNodeId. |
| instance_id | 16 | Sender's current instance_id. |
| ack_type | 8 | Corresponds to the NORM_CMD(ACK_REQ) ack_type (table above). |
| ack_id | 8 | Echoed ack_id (unused for NORM_ACK(CC)/NORM_ACK(FLUSH)). |
| grtt_response_sec | 32 | Adjusted send_time seconds. |
| grtt_response_usec | 32 | Adjusted send_time microseconds. |
| header extensions | var | EXT_CC etc. |
| ack_payload | var | Function of ack_type (see below). |

`ack_payload`:
- **NORM_ACK(CC)**: no attached content (header only).
- **NORM_ACK(FLUSH)**: the following payload (same shape as a Repair Request Item):

```
|     fec_id    |   reserved    |      object_transport_id      |
|                        fec_payload_id                         |
```

  | Field | bits | Meaning |
  |-------|------|---------|
  | fec_id | 8 | FEC type ⇒ fec_payload_id format. |
  | reserved | 8 | — |
  | object_transport_id | 16 | Object acknowledged through. |
  | fec_payload_id | var | Transmit position acknowledged. |

- **Application-defined ack_types**: application-specific, ≤ NormSegmentSize.

---

## NORM_REPORT (type = 6) (§4.4.1)

⚠ OPTIONAL message. **Format is currently UNDEFINED** by RFC 5740 — experimental
implementations may define their own NORM_REPORT formats. SHOULD be disabled for
interoperability testing. No wire layout to transcribe.

---

## IANA — NORM Header Extension Types registry (§8.1.1)

8-bit namespace; 0..127 = variable-length, 128..255 = fixed (4-byte) length.

| Value | Name | Reference |
|-------|------|-----------|
| 1 | EXT_AUTH | RFC 5740 |
| 3 | EXT_CC | RFC 5740 |
| 64 | EXT_FTI | RFC 5740 |
| 128 | EXT_RATE | RFC 5740 |

(EXT_AUTH content/processing is out of scope of RFC 5740, communicated out-of-band.)
