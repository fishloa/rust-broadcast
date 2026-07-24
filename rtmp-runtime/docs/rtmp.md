# Adobe RTMP 1.0 — wire format spec grounding for `rtmp-runtime` (#738)

## Provenance

- **Primary spec:** *Adobe's Real-Time Messaging Protocol* (also filed as
  "Adobe's Real Time Messaging Protocol"), H. Parmar & M. Thornburgh (eds.),
  Adobe Systems Incorporated, **21 December 2012**. Fetched from
  <https://veovera.org/docs/legacy/rtmp-v1-0-spec.pdf> (a verbatim copy of
  Adobe's own PDF, hosted by the Veovera Software Organization, the group that
  now stewards the open RTMP spec after Adobe stopped hosting it directly;
  cover page/copyright confirmed via `pdftotext`: "Copyright Adobe Systems
  Incorporated", "H. Parmar, Ed. / M. Thornburgh, Ed. / Adobe / December 21,
  2012"). 52 pages. Section numbers below (`§5.2`, `§7.2.1.1`, …) are this
  document's own section numbers.
- **Companion spec (referenced normatively by the RTMP spec's own
  Definitions and References sections as `[AMF0]`):**
  *Action Message Format — AMF 0*, Adobe Systems Inc. ("Copyright (c) Adobe
  Systems Inc. (2002-2006). All Rights Reserved."; PDF metadata: created 21
  December 2007 — matching the "December 2007" date and the exact
  `amf0_spec_121207.pdf` filename the RTMP spec's own reference list cites
  for `[AMF0]`). Fetched from
  <https://ossrs.io/lts/en-us/assets/files/amf0_spec_121207-ac97fd4db9408706cd816b681ca3918c.pdf>
  (a mirror of that same file). 11 pages. Note: the fetched PDF's own header
  banner and one TOC line misname themselves "AMF 3 Specification" / "4.
  Usages of AMF 3" (apparently copy-paste artifacts in Adobe's original
  file — the body text at that same location correctly reads "4. Usages of
  AMF 0" / "4.1 AMF Packets and NetConnection", and the cover page, abstract,
  and all normative content are unambiguously the AMF0 spec); flagged here
  so this isn't mistaken for a transcription error.
- Both PDFs were extracted with `pdftotext -layout` and cross-read page by
  page; every field/table below is transcribed directly from that text, not
  from memory. AMF3 is referenced by both specs but its wire format is **out
  of scope** here (see the mapping section — this crate does not need to
  encode/decode it for the ingest path).
- **Corroboration, not duplication:** this repo already carries a partial RTMP
  + AMF0 transcription at [`transmux/docs/rtmp/rtmp.md`](../../transmux/docs/rtmp/rtmp.md)
  (scoped to what `transmux`'s existing RTMP demux implements) and the FLV
  tag-body layout at [`transmux/docs/codec/flv.md`](../../transmux/docs/codec/flv.md)
  (from Adobe FLV v10.1 Annex E). Independently re-deriving the RTMP/AMF0
  sections from the primary PDFs above reproduces the same field layouts as
  that existing doc byte-for-byte, which cross-corroborates both. This file is
  the **broader** transcription (handshake diagram, full protocol-control set,
  full user-control event table, full shared-object event table, and the
  complete NetConnection/NetStream command set) needed to ground the
  sans-IO **session** state machine `rtmp-runtime` will implement, not just
  the message-type dispatch `transmux` needs.

All multi-byte integers are **big-endian** (network byte order) except the
Type-0 chunk message header's `message stream id`, which the spec states is
**little-endian** (§5.3.1.2.5), and the AMF0 `reference`/`ecma-array`/`strict-array`
counts, which are big-endian per `[AMF0]`.

---

## 1. Byte Order, Alignment, and Time Format (§4)

- **Byte order:** all integer fields are network byte order (big-endian);
  byte zero is the first byte shown, bit zero is the most significant bit of
  a word/field (§4, citing `[RFC0791]`). Numeric constants in the spec are
  decimal unless stated otherwise.
- **Alignment:** all data is byte-aligned except where otherwise specified —
  e.g. a 16-bit field may sit at an odd byte offset. Padding bytes, where
  present, SHOULD be zero.
- **Timestamps:** an integer number of **milliseconds relative to an
  unspecified epoch** (typically 0 at stream start, but not required — the
  two endpoints just need to agree). Cross-stream sync (especially across
  separate hosts) needs a mechanism outside RTMP itself. Being 32 bits, a
  timestamp **rolls over every 49 days, 17 hours, 2 minutes, 47.296
  seconds**; because streams can run for years, an application SHOULD use
  **serial number arithmetic (`[RFC1982]`)** and handle wraparound — e.g.
  treat adjacent timestamps as within `2^31 - 1` ms of each other, so `10000`
  is considered *after* `4000000000`, and `3000000000` *before* `4000000000`.
  Timestamp **deltas** are unsigned ms relative to the previous timestamp,
  24 or 32 bits depending on the chunk message header type (§3.1.2).

Relevant definitions (§3, abridged to what the wire layouts below use):
**message stream** / **message stream ID** — the logical channel a message
belongs to; **chunk** — a fragment of a message; **chunk stream** /
**chunk stream ID (csid)** — the logical channel chunks of one direction
flow through, ensuring timestamp-ordered delivery; **multiplexing** /
**demultiplexing** — interleaving/de-interleaving separate A/V streams into
one connection; **Action Message Format (AMF)** — the binary serialization
used by command/data messages, in two incompatible versions, AMF0 (`[AMF0]`)
and AMF3 (`[AMF3]`).

---

## 2. Handshake (§5.2)

An RTMP connection begins with a handshake consisting of three **static-sized**
chunks each way — C0, C1, C2 from the client; S0, S1, S2 from the server
(§5.2). This precedes and is structurally unlike the rest of the protocol
(no chunk headers).

### 2.1 Handshake sequence (§5.2.1)

- The client MUST wait for S1 before sending C2, and for S2 before sending
  anything else.
- The server MUST wait for C0 before sending S0/S1 (MAY wait for C1 too),
  MUST wait for C1 before sending S2, and MUST wait for C2 before sending
  anything else.

### 2.2 C0 / S0 format (§5.2.2) — 1 byte

| Field | Bits | Description |
|---|---|---|
| `version` | `[7:0]` | RTMP version. This spec defines **3**. `0`–`2` are deprecated (earlier proprietary products); `4`–`31` reserved; `32`–`255` **not allowed** (so RTMP is distinguishable from text protocols that start with a printable character). A server not recognizing the client's version SHOULD respond with 3; the client MAY degrade to 3 or abandon. |

### 2.3 C1 / S1 format (§5.2.3) — 1536 bytes

| Field | Bytes | Description |
|---|---|---|
| `time` | 4 | Timestamp, SHOULD be used as the epoch for this endpoint's future chunks. May be 0 or arbitrary. |
| `zero` | 4 | MUST be all zeros. |
| `random bytes` | 1528 | Arbitrary data distinguishing this handshake from the peer's; no cryptographic randomness required. |

Total: 4 + 4 + 1528 = **1536**.

### 2.4 C2 / S2 format (§5.2.4) — 1536 bytes (near-echo of the peer's C1/S1)

| Field | Bytes | Description |
|---|---|---|
| `time` | 4 | MUST equal the peer's S1 `time` (for C2) or C1 `time` (for S2). |
| `time2` | 4 | MUST be the timestamp at which the peer's previous packet (S1 or C1) was read. |
| `random echo` | 1528 | MUST equal the peer's S1/C1 `random bytes`, verbatim. |

Time/time2 together with the local clock give a rough bandwidth/latency
estimate ("unlikely to be useful", per the spec).

### 2.5 Handshake diagram (§5.2.5)

```
+-------------+                            +-------------+
|   Client    |       TCP/IP Network       |   Server    |
+-------------+             |              +-------------+
       |                    |                      |
Uninitialized               |                Uninitialized
       |          C0        |                      |
       |------------------->|         C0           |
       |                    |-------------------->|
       |          C1        |                      |
       |------------------->|         S0           |
       |                    |<--------------------|
       |                    |         S1           |
  Version sent              |<--------------------|
       |          S0        |                      |
       |<-------------------|                      |
       |          S1        |                      |
       |<-------------------|                 Version sent
       |                    |         C1           |
       |                    |-------------------->|
       |          C2        |                      |
       |------------------->|         S2           |
       |                    |<--------------------|
    Ack sent                |                   Ack Sent
       |          S2        |                      |
       |<-------------------|                      |
       |                    |         C2           |
       |                    |-------------------->|
  Handshake Done            |                Handshake Done
```

States: **Uninitialized** (version exchange, C0/S0) → **Version Sent**
(client awaits S1, server awaits C1; on receipt each side sends its
`C2`/`S2`) → **Ack Sent** (each side awaits the other's `C2`/`S2`) →
**Handshake Done** (normal chunk-stream message exchange begins).

---

## 3. RTMP Chunk Stream (§5.3)

After the handshake, the connection multiplexes one or more **chunk streams**,
each identified by a **chunk stream ID (csid)**. Each chunk stream carries
messages of one type from one message stream. Chunks are sent whole,
back-to-back; the receiver reassembles chunks into messages per csid.

Chunking lets large messages (e.g. video) be split so they don't block small
high-priority messages (e.g. audio/control), and lets small messages omit
redundant header fields via 4 compact header forms. **Max chunk size**
defaults to **128 bytes**, is renegotiated via Set Chunk Size (§5.4.1,
protocol control message 1), and is maintained **independently per direction**.

### 3.1 Chunk format (§5.3.1)

```
+--------------+----------------+---------------------+--------------+
| Basic Header | Message Header | Extended Timestamp   |  Chunk Data  |
+--------------+----------------+---------------------+--------------+
|                                                       |
|<-------------------- Chunk Header ------------------>|
```

- **Basic Header** — 1 to 3 bytes: encodes csid + chunk type (`fmt`); length
  depends only on the csid value.
- **Message Header** — 0, 3, 7, or 11 bytes, selected by `fmt`.
- **Extended Timestamp** — 0 or 4 bytes, present per the rule in §3.1.3 below.
- **Chunk Data** — up to the configured max chunk size.

#### 3.1.1 Chunk Basic Header (§5.3.1.1)

The protocol supports up to 65597 chunk streams, IDs 3–65599; IDs 0, 1, 2 are
reserved (0/1 select the 2-/3-byte header forms; **csid 2 is reserved for
low-level protocol control messages and commands**). Implementations SHOULD
use the smallest form that holds the id.

**1-byte form** (csid 3–63):

| Field | Bits |
|---|---|
| `fmt` | `[7:6]` |
| `cs id` | `[5:0]` |

**2-byte form** (csid 64–319; byte 0's low 6 bits = `0`):

| Field | Bits/Bytes |
|---|---|
| `fmt` | byte 0 `[7:6]` |
| `0` (marker) | byte 0 `[5:0]` |
| `cs id − 64` | byte 1 (8 bits) |

csid = byte 1 + 64.

**3-byte form** (csid 64–65599; byte 0's low 6 bits = `1`):

| Field | Bits/Bytes |
|---|---|
| `fmt` | byte 0 `[7:6]` |
| `1` (marker) | byte 0 `[5:0]` |
| `cs id − 64` | bytes 1–2 (16 bits) |

csid = (byte 2 × 256) + byte 1 + 64. Csids 64–319 are representable in
**either** the 2- or 3-byte form.

`fmt` (2 bits, all 3 forms): selects which of the 4 Chunk Message Header
formats follows (§3.1.2).

#### 3.1.2 Chunk Message Header — 4 formats selected by `fmt` (§5.3.1.2)

**Type 0 — `fmt`=0, 11 bytes** (§5.3.1.2.1). MUST be used at the start of a
chunk stream and whenever the stream timestamp goes backward (e.g. backward
seek).

| Field | Bytes | Notes |
|---|---|---|
| `timestamp` | 3 | Absolute timestamp of the message. If ≥ `0xFFFFFF`, this field MUST be `0xFFFFFF` and the full 32-bit value goes in Extended Timestamp. |
| `message length` | 3 | Length of the whole message (not the chunk payload — see §3.1.2 common fields). |
| `message type id` | 1 | See §6 message type ids. |
| `message stream id` | 4 | **Little-endian** (only field in the whole chunk stream that is). |

**Type 1 — `fmt`=1, 7 bytes** (§5.3.1.2.2). No message stream id — inherits
the preceding chunk's. SHOULD be used for the first chunk of each new message
after the first, for streams with **variable-sized** messages (e.g. video).

| Field | Bytes | Notes |
|---|---|---|
| `timestamp delta` | 3 | Delta from the previous chunk's timestamp. `0xFFFFFF` ⇒ Extended Timestamp carries the full 32-bit delta. |
| `message length` | 3 | |
| `message type id` | 1 | |

**Type 2 — `fmt`=2, 3 bytes** (§5.3.1.2.3). Neither stream id nor message
length included — both inherited. SHOULD be used for the first chunk of each
message after the first, for streams with **constant-sized** messages (e.g.
some audio/data formats).

| Field | Bytes | Notes |
|---|---|---|
| `timestamp delta` | 3 | Same `0xFFFFFF` extended-timestamp rule. |

**Type 3 — `fmt`=3, 0 bytes** (§5.3.1.2.4). No message header at all — stream
id, message length, message type id, and timestamp delta are all inherited
from the preceding chunk on the same csid. Used for every chunk of a message
after the first (continuation), **and** for a run of same-size/same-spacing
messages after an initial Type 2 chunk (the second use is "begin a new
message whose header is derived from existing state"). If a Type 3 chunk
immediately follows a Type 0 chunk (no intervening Type 2), its implied
timestamp delta equals the Type 0 chunk's absolute timestamp.

**Common header fields** (§5.3.1.2.5):

| Field | Bytes | Present in | Notes |
|---|---|---|---|
| `timestamp delta` | 3 | Type 1, 2 | Delta from previous chunk's timestamp on this csid. |
| `message length` | 3 | Type 0, 1 | Total message length, big-endian; generally **not** equal to the chunk payload length (see below). |
| `message type id` | 1 | Type 0, 1 | |
| `message stream id` | 4 | Type 0 only | Little-endian; typically constant per chunk stream, but a new Type 0 chunk MAY re-target a reused csid to a new message stream. |

Chunk payload length = max chunk size, for every chunk but the last of a
message; the last chunk carries the remainder (possibly the whole message,
for small ones).

#### 3.1.3 Extended Timestamp (§5.3.1.3) — 0 or 4 bytes

Encodes the full 32-bit timestamp/delta when it doesn't fit the 24-bit Type
0/1/2 field (i.e. ≥ `0xFFFFFF`). Presence is signalled by that 24-bit field
reading exactly `0xFFFFFF`. Also present on a **Type 3** chunk when the most
recent Type 0/1/2 chunk on the same csid indicated an extended timestamp.

#### 3.1.4 Worked examples (§5.3.2, informative)

- **Example 1** — 4 audio messages (msid 12345, type 8, 32 bytes, times
  1000/1020/1040/1060 ⇒ constant 20 ms delta): chunk 1 is Type 0 (11-byte
  header, full state); chunk 2 is Type 2 (3-byte header, delta 20 — length,
  type, stream id all repeat); chunks 3–4 are Type 3 (0-byte header — the
  20 ms delta and everything else is inherited from chunk 2's state).
- **Example 2** — one 307-byte video message (msid 12346, type 9, time 1000)
  over a 128-byte chunk size: chunk 1 is Type 0 (11-byte header + 128 bytes
  payload = 140 total); chunks 2–3 are Type 3 continuation chunks (0-byte
  header + 128, then + 51 bytes = the remaining 179 bytes).

---

## 4. Protocol Control Messages (§5.4) — message type ids 1, 2, 3, 5, 6

These carry information the **RTMP Chunk Stream** layer itself needs. They
**MUST** use message stream id **0** (the control stream) and be sent on
**chunk stream id 2**; they take effect immediately on receipt (timestamps
ignored).

| Type id | Name | Payload |
|---|---|---|
| 1 | Set Chunk Size (§5.4.1) | 1 reserved bit (MUST be 0) + `chunk size` (31 bits, `1`..`0x7FFFFFFF`; sizes > `0xFFFFFF` are equivalent since no chunk exceeds one message and no message exceeds `0xFFFFFF` bytes). Default max chunk size is 128; SHOULD be ≥ 128, MUST be ≥ 1. |
| 2 | Abort Message (§5.4.2) | `chunk stream id` (32 bits) — discard any partially-received message on that csid. Senders MAY send this when closing to signal further messages need not be processed. |
| 3 | Acknowledgement (§5.4.3) | `sequence number` (32 bits) = total bytes received so far. MUST be sent after receiving bytes equal to the peer's advertised window size. |
| 5 | Window Acknowledgement Size (§5.4.4) | `Acknowledgement Window size` (32 bits). Receiver MUST send an Acknowledgement (type 3) after receiving that many bytes since the last one (or session start). |
| 6 | Set Peer Bandwidth (§5.4.5) | `Acknowledgement Window size` (32 bits) + `Limit Type` (1 byte). Limits the peer's **output** bandwidth to the given window of unacknowledged data. Receiver SHOULD reply with Window Ack Size (type 5) if its window differs from the last one it advertised. |

**Set Peer Bandwidth Limit Type values** (§5.4.5):

| Value | Name | Meaning |
|---|---|---|
| 0 | Hard | Peer SHOULD limit output to exactly the indicated window. |
| 1 | Soft | Peer SHOULD limit output to the indicated window **or** its current limit, whichever is smaller. |
| 2 | Dynamic | If the previous Limit Type was Hard, treat as Hard; otherwise ignore this message. |

---

## 5. RTMP Message Format (§6) and User Control (§6.2, type 4)

§6 describes the message format independent of any particular lower-level
transport (RTMP is defined to work over the RTMP Chunk Stream, but could use
another transport).

### 5.1 Message Header (§6.1.1)

| Field | Bytes | Notes |
|---|---|---|
| Message Type | 1 | Ids `1`–`6` reserved for protocol control (this doc's §4). |
| Payload Length | 3 | Big-endian. |
| Timestamp | 4 | Big-endian. |
| Message Stream Id | 3 | Big-endian (per §6.1.1's own diagram — see note below). |

> **Note on stream-id endianness/width:** §6.1.1's abstract Message Header
> diagram shows a 3-byte, big-endian Message Stream Id, but the **concrete
> Type-0 chunk message header** that actually carries it over the wire
> (§5.3.1.2.1 / §5.3.1.2.5, this doc's §3.1.2 above) is 4 bytes and
> **little-endian**. The chunk-stream encoding (this doc's §3) is normative
> for wire bytes; treat §6.1.1's figure as the logical/abstract message
> header, not the wire encoding.

### 5.2 Message Payload (§6.1.2)

The payload's format/interpretation is "beyond the scope" of §6 — it's
defined per message type in §7.

### 5.3 User Control Messages (§6.2) — message type id 4

Carries information for the **RTMP streaming layer** (distinct from the
protocol-control types 1/2/3/5/6, which are Chunk Stream layer). SHOULD use
message stream id 0 and, over the chunk stream, csid 2. Effective on receipt;
timestamps ignored.

| Field | Bits |
|---|---|
| `Event Type` | 16 |
| `Event Data` | variable |

Event types + their event-data formats are in this doc's §6.7 User Control
Message Events table (§7.1.7 in the spec's own numbering).

---

## 6. RTMP Message Types (§7.1)

| Type id(s) | Name | §  |
|---|---|---|
| 20 (AMF0) / 17 (AMF3) | Command Message | §7.1.1 |
| 18 (AMF0) / 15 (AMF3) | Data Message | §7.1.2 |
| 19 (AMF0) / 16 (AMF3) | Shared Object Message | §7.1.3 |
| 8 | Audio Message | §7.1.4 |
| 9 | Video Message | §7.1.5 |
| 22 | Aggregate Message | §7.1.6 |

### 6.1 Command Message (20, 17) — §7.1.1

AMF-encoded commands (connect/createStream/publish/play/pause/…) between
client and server: **command name, transaction id, command object**
(name-value parameters), for RPC. Responses (`onStatus`, `result`, etc.)
inform the sender of the outcome.

### 6.2 Data Message (18, 15) — §7.1.2

Sends metadata (creation time, duration, theme, …) or arbitrary user data —
no RPC semantics.

### 6.3 Shared Object Message (19, 16) — §7.1.3

A Shared Object is a Flash object (name/value pairs) kept synchronized
across multiple clients + the server. Each message can carry multiple events.

```
+------+------+-------+-----+-----+------+-----+     +-----+------+-----+
|Header|Shared|Current|Flags|Event|Event |Event| ... |Event|Event |Event|
|      |Object|Version|     |Type |data  |data | ... |Type |data  |data |
|      |Name  |       |     |     |length|     |     |     |length|     |
+------+------+-------+-----+-----+------+-----+     +-----+------+-----+
       |<-------------------- AMF Shared Object Message body ---------->|
```

**Shared Object event types:**

| Event | Value | Direction / meaning |
|---|---|---|
| Use | 1 | Client → server: creation of a named shared object. |
| Release | 2 | Client → server: shared object deleted client-side. |
| Request Change | 3 | Client → server: request to change a named parameter's value. |
| Change | 4 | Server → all clients except originator: a parameter's value changed. |
| Success | 5 | Server → requesting client: RequestChange accepted. |
| SendMessage | 6 | Client → server: broadcast a message (server re-broadcasts to all, incl. sender). |
| Status | 7 | Server → clients: notify of an error condition. |
| Clear | 8 | Server → client: clear the shared object (also sent in response to `Use` on connect). |
| Remove | 9 | Server → client: delete a slot. |
| Request Remove | 10 | Client → server: request deletion of a slot. |
| Use Success | 11 | Server → client: successful connection. |

### 6.4 Audio Message (8) — §7.1.4

Carries audio data. **Body = the FLV `AudioTagHeader` + audio payload** —
see §9 (FLV mapping) below; this spec does not itself define the codec
layout.

### 6.5 Video Message (9) — §7.1.5

Carries video data. **Body = the FLV `VideoTagHeader` + video payload** —
see §9 below.

### 6.6 Aggregate Message (22) — §7.1.6

A single message containing a series of RTMP sub-messages using the RTMP
message format described in the spec's own §6.1 (this doc's §5.1):

```
+---------+-------------------------+
| Header  | Aggregate Message body  |
+---------+-------------------------+

+--------+-------+---------+--------+-------+---------+- - - -
|Header 0|Message|Back     |Header 1|Message|Back     |
|        |Data 0 |Pointer 0|        |Data 1 |Pointer 1|
+--------+-------+---------+--------+-------+---------+- - - -
```

- The aggregate message's own message stream id overrides every sub-message's
  stream id.
- Sub-message timestamps are renormalized: offset = (aggregate ts − first
  sub-message ts), added to each sub-message ts. The first sub-message's ts
  SHOULD equal the aggregate's, so offset SHOULD be 0.
- `Back Pointer` = size of the *previous* message **including its header**
  (matches the FLV `PreviousTagSize` convention — used for backward seek).
- Rationale: fewer chunks overall (one aggregate vs. many small messages),
  and sub-messages are contiguous in memory for cheaper `send()` syscalls.

### 6.7 User Control Message Events (§7.1.7) — event-data formats for type 4

| Event | Value | Sender | Event data | Meaning |
|---|---|---|---|---|
| Stream Begin | 0 | Server | 4 bytes: stream id | Stream is now functional/usable. By default sent on stream id 0 right after a successful `connect`. |
| Stream EOF | 1 | Server | 4 bytes: stream id | Playback requested on this stream has ended; no more data will be sent; client discards further messages for it. |
| StreamDry | 2 | Server | 4 bytes: stream id | No more data on the stream (server-detected idle). |
| SetBufferLength | 3 | Client | 4 bytes: stream id + 4 bytes: buffer length (ms) | Informs the server of the client's playback buffer size, before the server starts sending the stream. |
| StreamIsRecorded | 4 | Server | 4 bytes: stream id | The stream is a recorded (not live) stream. |
| PingRequest | 6 | Server | 4 bytes: server-local timestamp | Test reachability; client MUST reply with PingResponse. |
| PingResponse | 7 | Client | 4 bytes: echoed timestamp | Reply to PingRequest, echoing its timestamp. |

(Event value 5 is not defined by this spec.)

---

## 7. RTMP Command Messages (§7.2) — NetConnection / NetStream

Client and server exchange **AMF-encoded commands**: a command name, a
transaction id, and a command object of related parameters. A response's
command name is `_result`, `_error`, or a method name (e.g. `onStatus`); its
transaction id identifies which outstanding request it answers (cf. IMAP
tags).

### 7.1 NetConnection commands (§7.2.1) — csid 2 / message stream id 0

`NetConnection` is the two-way connection between client and a server
application instance; also supports async RPC. Commands: `connect`, `call`
(Call), `close` (not detailed further by the spec), `createStream`.

#### 7.1.1 `connect` (§7.2.1.1)

**Request (client → server):**

| Field | Type | Description |
|---|---|---|
| Command Name | String | `"connect"`. |
| Transaction ID | Number | Always `1`. |
| Command Object | Object | Name-value pairs (table below). |
| Optional User Arguments | Object | Any optional info. |

**Command Object name-value pairs:**

| Property | Type | Description | Example |
|---|---|---|---|
| `app` | String | Server application name the client connects to. | `testapp` |
| `flashVer` | String | Flash Player version string (as returned by `ApplicationScript.getversion()`). | `FMSc/1.0` |
| `swfUrl` | String | URL of the source SWF making the connection. | `file://C:/FlvPlayer.swf` |
| `tcUrl` | String | Server URL: `protocol://servername:port/appName/appInstance`. | `rtmp://localhost:1935/testapp/instance1` |
| `fpad` | Boolean | True if a proxy is used. | `true`/`false` |
| `audioCodecs` | Number | Bitmask of supported audio codecs (table below). | `SUPPORT_SND_MP3` |
| `videoCodecs` | Number | Bitmask of supported video codecs (table below). | `SUPPORT_VID_SORENSON` |
| `videoFunction` | Number | Bitmask of supported special video functions. | `SUPPORT_VID_CLIENT_SEEK` |
| `pageUrl` | String | URL of the web page that loaded the SWF. | `http://somehost/sample.html` |
| `objectEncoding` | Number | AMF encoding method (`0` = AMF0, `3` = AMF3). | `AMF3` |

**`audioCodecs` flag values:**

| Flag | Meaning | Value |
|---|---|---|
| `SUPPORT_SND_NONE` | Raw sound, no compression | `0x0001` |
| `SUPPORT_SND_ADPCM` | ADPCM compression | `0x0002` |
| `SUPPORT_SND_MP3` | mp3 compression | `0x0004` |
| `SUPPORT_SND_INTEL` | Not used | `0x0008` |
| `SUPPORT_SND_UNUSED` | Not used | `0x0010` |
| `SUPPORT_SND_NELLY8` | NellyMoser 8 kHz | `0x0020` |
| `SUPPORT_SND_NELLY` | NellyMoser (5/11/22/44 kHz) | `0x0040` |
| `SUPPORT_SND_G711A` | G711A (Flash Media Server only) | `0x0080` |
| `SUPPORT_SND_G711U` | G711U (Flash Media Server only) | `0x0100` |
| `SUPPORT_SND_NELLY16` | NellyMoser 16 kHz | `0x0200` |
| `SUPPORT_SND_AAC` | AAC codec | `0x0400` |
| `SUPPORT_SND_SPEEX` | Speex audio | `0x0800` |
| `SUPPORT_SND_ALL` | All RTMP-supported audio codecs | `0x0FFF` |

**`videoCodecs` flag values:**

| Flag | Meaning | Value |
|---|---|---|
| `SUPPORT_VID_UNUSED` | Obsolete | `0x0001` |
| `SUPPORT_VID_JPEG` | Obsolete | `0x0002` |
| `SUPPORT_VID_SORENSON` | Sorenson Flash video | `0x0004` |
| `SUPPORT_VID_HOMEBREW` | V1 screen sharing | `0x0008` |
| `SUPPORT_VID_VP6` | On2 video (Flash 8+) | `0x0010` |
| `SUPPORT_VID_VP6ALPHA` | On2 video with alpha channel | `0x0020` |
| `SUPPORT_VID_HOMEBREWV` | Screen sharing v2 (Flash 8+) | `0x0040` |
| `SUPPORT_VID_H264` | H.264 video | `0x0080` |
| `SUPPORT_VID_ALL` | All RTMP-supported video codecs | `0x00FF` |

**`videoFunction` flag values:**

| Flag | Meaning | Value |
|---|---|---|
| `SUPPORT_VID_CLIENT_SEEK` | Client can perform frame-accurate seeks | `1` |

**`objectEncoding` values:**

| Encoding | Meaning | Value |
|---|---|---|
| AMF0 | Supported by Flash 6+ | `0` |
| AMF3 | AS3 encoding from Flash 9 | `3` |

**Response (server → client):**

| Field | Type | Description |
|---|---|---|
| Command Name | String | `_result` or `_error`. |
| Transaction ID | Number | `1` for connect responses. |
| Properties | Object | Connection properties (e.g. `fmsver`). |
| Information | Object | Response info; commonly includes `level`, `code`, `description`, `objectencoding`. |

**Message flow (§7.2.1.1):**

```
Client                                        Server
  |----------- Command Message(connect) ------->|
  |<------- Window Acknowledgement Size --------|
  |<----------- Set Peer Bandwidth -------------|
  |-------- Window Acknowledgement Size ------->|
  |<------ User Control Message(StreamBegin) ---|
  |<------------ Command Message ---------------|
  |       (_result — connect response)          |
```

1. Client sends `connect`.
2. Server sends Window Acknowledgement Size (and connects the client to the
   named application).
3. Server sends Set Peer Bandwidth.
4. Client sends Window Acknowledgement Size (after processing Set Peer
   Bandwidth).
5. Server sends User Control (`StreamBegin`).
6. Server sends the `_result` command message with transaction id 1 and the
   Properties/Information described above.

#### 7.1.2 `call` (§7.2.1.2)

Runs an RPC at the receiving end.

**Request:**

| Field | Type | Description |
|---|---|---|
| Procedure Name | String | Name of the remote procedure. |
| Transaction ID | Number | Nonzero if a response is expected, else `0`. |
| Command Object | Object | Command info, or `Null`. |
| Optional Arguments | Object | Any optional arguments. |

**Response:**

| Field | Type | Description |
|---|---|---|
| Command Name | String | Name of the command. |
| Transaction ID | Number | ID of the request this responds to. |
| Command Object | Object | Command info, or `Null`. |
| Response | Object | The called method's response. |

#### 7.1.3 `createStream` (§7.2.1.3)

Creates a logical channel (message stream) for subsequent audio/video/data
messages. `NetConnection` itself is the default channel (message stream id
0); `createStream` and a few other command messages use it.

**Request:**

| Field | Type | Description |
|---|---|---|
| Command Name | String | `"createStream"`. |
| Transaction ID | Number | Transaction id. |
| Command Object | Object | Command info, or `Null`. |

**Response:**

| Field | Type | Description |
|---|---|---|
| Command Name | String | `_result` or `_error`. |
| Transaction ID | Number | ID of the request. |
| Command Object | Object | Command info, or `Null`. |
| Stream ID | Number | The newly created stream id (or error info on `_error`). |

### 7.2 NetStream commands (§7.2.2)

`NetStream` is the channel (over an existing `NetConnection`) through which
audio/video/data flow and playback is controlled. Client → server commands:
`play`, `play2`, `deleteStream`, `closeStream`, `receiveAudio`,
`receiveVideo`, `publish`, `seek`, `pause`. Server → client status updates
use `onStatus`.

**`onStatus` format (common to all NetStream status updates):**

| Field | Type | Description |
|---|---|---|
| Command Name | String | `"onStatus"`. |
| Transaction ID | Number | `0`. |
| Command Object | Null | None. |
| Info Object | Object | At least `level` (String: `"warning"`\|`"status"`\|`"error"`), `code` (String, e.g. `"NetStream.Play.Start"`), `description` (String). MAY carry more properties per code. |

#### 7.2.1 `play` (§7.2.2.1)

Plays a stream (repeatable calls build a playlist).

| Field | Type | Description |
|---|---|---|
| Command Name | String | `"play"`. |
| Transaction ID | Number | `0`. |
| Command Object | Null | None. |
| Stream Name | String | Stream to play. Bare name for FLV; `mp3:name` for MP3/ID3; `mp4:name.ext` for H.264/AAC. |
| Start | Number *(optional)* | Seconds. Default `-2`: try live stream of that name, else recorded, else wait for a live one. `-1`: only the live stream. `≥0`: recorded stream starting at that offset (falls through the playlist if not found). |
| Duration | Number *(optional)* | Seconds. Default `-1`: play until unavailable/end. `0`: single frame from Start. `>0`: play for that long (live) or that much of a recorded stream. Negative values other than `-1` are treated as `-1`. |
| Reset | Boolean *(optional)* | Whether to flush any existing playlist. |

**Message flow (§7.2.2.1, informative excerpt):**

```
Client                                             Server
  |------ Command Message(createStream) -------->|
  |<------------ Command Message -----------------|
  |      (_result — createStream response)        |
  |------ Command Message(play) ------------------>|
  |<------------- SetChunkSize --------------------|
  |<----- User Control (StreamIsRecorded) ---------|
  |<----- User Control (StreamBegin) --------------|
  |<----- Command Message(onStatus play.reset) ----|
  |<----- Command Message(onStatus play.start) ----|
  |<------------- Audio Message -------------------|
  |<------------- Video Message -------------------|
```

`NetStream.Play.Reset` is sent only if the client's `play` set the reset
flag. If the requested stream isn't found, the server sends
`NetStream.Play.StreamNotFound` instead.

#### 7.2.2 `play2` (§7.2.2.2)

Switches to a different-bitrate rendition of the same content without
resetting the timeline (server maintains multiple bitrate files).

| Field | Type | Description |
|---|---|---|
| Command Name | String | `"play2"`. |
| Transaction ID | Number | `0`. |
| Command Object | Null | None. |
| Parameters | Object | AMF object mirroring `flash.net.NetStreamPlayOptions`'s public properties (per the ActionScript 3 Language Reference — not reproduced by the RTMP spec itself). |

#### 7.2.3 `deleteStream` (§7.2.2.3)

Sent when the `NetStream` object is being destroyed.

| Field | Type | Description |
|---|---|---|
| Command Name | String | `"deleteStream"`. |
| Transaction ID | Number | `0`. |
| Command Object | Null | None. |
| Stream ID | Number | The stream id to destroy on the server. |

The server sends **no response**.

#### 7.2.4 `receiveAudio` (§7.2.2.4)

| Field | Type | Description |
|---|---|---|
| Command Name | String | `"receiveAudio"`. |
| Transaction ID | Number | `0`. |
| Command Object | Null | None. |
| Bool Flag | Boolean | Whether to receive audio. |

No response if `false`. If `true`, server responds with
`NetStream.Seek.Notify` and `NetStream.Play.Start`.

#### 7.2.5 `receiveVideo` (§7.2.2.5)

Same shape as `receiveAudio`, command name `"receiveVideo"`, same response
behavior.

#### 7.2.6 `publish` (§7.2.2.6) — the ingest-relevant command

Publishes a named stream to the server; other clients can play it.

| Field | Type | Description |
|---|---|---|
| Command Name | String | `"publish"`. |
| Transaction ID | Number | `0`. |
| Command Object | Null | None. |
| Publishing Name | String | Name to publish the stream under. |
| Publishing Type | String | `"live"` (no recording), `"record"` (new file, overwritten if it exists), or `"append"` (append to existing/new file). |

Server responds with `onStatus` marking the start of publishing.

#### 7.2.7 `seek` (§7.2.2.7)

| Field | Type | Description |
|---|---|---|
| Command Name | String | `"seek"`. |
| Transaction ID | Number | `0`. |
| Command Object | Null | None. |
| milliSeconds | Number | Offset to seek to. |

Server sends `NetStream.Seek.Notify` on success, `_error` on failure.

#### 7.2.8 `pause` (§7.2.2.8)

| Field | Type | Description |
|---|---|---|
| Command Name | String | `"pause"`. |
| Transaction ID | Number | `0` (no transaction id used). |
| Command Object | Null | None. |
| Pause/Unpause Flag | Boolean | `true` to pause, `false` to resume. |
| milliSeconds | Number | Client-side stream time at pause; on resume the server only sends messages timestamped later than this. |

Server sends `NetStream.Pause.Notify` (pause) / `NetStream.Unpause.Notify`
(resume) on success, `_error` on failure.

### 7.3 Message exchange examples (§7.3, informative)

- **§7.3.1 Publish a recorded/live stream:** `connect` → Window Ack Size /
  Set Peer Bandwidth exchange → Window Ack Size (client) → User Control
  (StreamBegin) → `_result` (connect) → `createStream` → `_result`
  (createStream) → `publish` → User Control (StreamBegin) → Data Message
  (metadata) → Audio Data → SetChunkSize → `_result` (publish) → Video Data
  → … until the stream completes.
- **§7.3.2 Broadcast a Shared Object message:** `Use` → `UseSuccess`+`Clear`
  → `RequestChange` → `Success` → `SendMessage` (client) →
  `SendMessage` (server broadcast).
- **§7.3.3 Publish metadata from a recorded stream:** `createStream` →
  `_result` → `publish` → User Control (StreamBegin) → Data Message
  (metadata).

---

## 8. AMF0 (companion spec `[AMF0]`)

RTMP Command/Data messages carry their command name, transaction id, and
argument values as a **sequence of AMF0-encoded values** written directly
into the message body — RTMP does not wrap them in AMF0's own `amf-packet`
framing (`[AMF0]` §4.1 "AMF Packets and NetConnection"; that framing, with
its `version`/`header-count`/`message-count` fields, is for the separate
"AMF over HTTP" NetConnection remoting scheme, not for command messages
carried over the RTMP chunk stream). AMF3 (`objectEncoding` 3, message
types 15/17) exists but is out of
scope here — see the mapping section.

### 8.1 Type markers (`[AMF0]` §2.1) — 1 byte, values in hex

There are 16 core type markers, plus one AMF0-extension marker for switching
to AMF3 mid-stream (`[AMF0]` §3.1):

| Marker | Value | Notes |
|---|---|---|
| `number-marker` | `0x00` | |
| `boolean-marker` | `0x01` | |
| `string-marker` | `0x02` | |
| `object-marker` | `0x03` | |
| `movieclip-marker` | `0x04` | Reserved, not supported. |
| `null-marker` | `0x05` | |
| `undefined-marker` | `0x06` | |
| `reference-marker` | `0x07` | |
| `ecma-array-marker` | `0x08` | |
| `object-end-marker` | `0x09` | |
| `strict-array-marker` | `0x0A` | |
| `date-marker` | `0x0B` | |
| `long-string-marker` | `0x0C` | |
| `unsupported-marker` | `0x0D` | |
| `recordset-marker` | `0x0E` | Reserved, not supported. |
| `xml-document-marker` | `0x0F` | |
| `typed-object-marker` | `0x10` | |
| `avmplus-object-marker` | `0x11` | Signals a switch to **AMF3** encoding for the following object (`[AMF0]` §3.1). |

### 8.2 Data types needed for command messages (`[AMF0]` §2.2–2.13, §2.18)

Basic rules (`[AMF0]` §1.3): `U16`/`U32` big-endian unsigned; `DOUBLE` = IEEE-754
8-byte big-endian; a "UTF-8" string is `U16` byte-length + UTF-8 bytes (a "long"
string uses a `U32` length instead).

| Type | Marker | Encoding |
|---|---|---|
| **Number** (§2.2) | `0x00` | `DOUBLE` (8 bytes, big-endian IEEE-754). |
| **Boolean** (§2.3) | `0x01` | `U8`: `0` = false, nonzero (typically `1`) = true. |
| **String** (§2.4) | `0x02` | `U16` length + UTF-8 bytes (≤ 65535 bytes; use Long String beyond that). |
| **Object** (anonymous, §2.5) | `0x03` | Repeated `(UTF-8 key, value-type)` pairs, terminated by an **Object End** (empty `U16`-length key + `0x09` marker). Sent by reference if the same instance recurs (§2.9). |
| **null** (§2.7) | `0x05` | No payload. |
| **undefined** (§2.8) | `0x06` | No payload. |
| **Reference** (§2.9) | `0x07` | `U16` index into a table of previously-serialized complex objects (indices from 0); avoids resending/duplicating cyclic object graphs. |
| **ECMA Array** (§2.10) | `0x08` | `U32` associative-count, then the same `(UTF-8 key, value-type)`* pairs as Object, terminated by Object End. (An ActionScript Array with non-ordinal indices, treated like an associative Object.) |
| **Object End** (§2.11) | `0x09` | Always preceded by an empty (`U16` = 0) key: the 3-byte sequence `0x00 0x00 0x09`. |
| **Strict Array** (§2.12) | `0x0A` | `U32` array-count, then that many `value-type` entries (ordinal only; sparse entries serialize as `undefined`). |
| **Date** (§2.13) | `0x0B` | `DOUBLE` (ms since Unix epoch, UTC) + `S16` time-zone (reserved, MUST be `0x0000`). |
| **Long String** (§2.14) | `0x0C` | `U32` length + UTF-8 bytes (for >65535-byte strings). |
| **Typed Object** (§2.18) | `0x10` | `UTF-8` class-name + the same terminated key/value pairs as Object. |

Every RTMP command/data message body is simply a sequence of these
`value-type` markers concatenated one after another — e.g. `connect`'s body
is: String (`"connect"`) + Number (`1`) + Object (the Command Object
name/value pairs) [+ optional trailing Object].

### 8.3 AMF3 (deferred — not needed for the ingest path)

`[AMF0]` §3.1 defines only the *marker* (`0x11`) that signals a value is
AMF3-encoded inside an otherwise-AMF0 stream; the AMF3 wire format itself is
a separate document (`[AMF3]`, referenced but not fetched here — see the
mapping section for why it's out of scope for now).

---

## 9. FLV tag payload mapping (Audio/Video message bodies)

The RTMP spec explicitly leaves message *payload* format out of scope for
Audio (8) and Video (9) messages (§6.1.2, §7.1.4, §7.1.5) — that layout comes
from the **Adobe Flash Video File Format Specification v10.1, Annex E**, a
different Adobe document. **The message body IS the FLV tag body**: an RTMP
Audio/Video message's payload is byte-identical to an FLV file's
`AudioTagHeader`/`VideoTagHeader` + payload for that tag (this is precisely
why FLV muxers/demuxers and RTMP publishers share the same sample-layer
code). This repo already carries that FLV transcription, spec-cited
independently, at
[`transmux/docs/codec/flv.md`](../../transmux/docs/codec/flv.md); summary:

- **Video** (`VideoTagHeader`): `FrameType` (4 bits: 1 keyframe, 2 inter, 3
  disposable inter, 5 info/command) + `CodecID` (4 bits: `7` = AVC/H.264).
  For AVC, an `AVCPacketType` byte follows (`0` = AVC sequence header =
  `avcC`/`AVCDecoderConfigurationRecord`, `1` = one or more length-prefixed
  NALUs, `2` = end of sequence) + a 24-bit signed `CompositionTime` (PTS−DTS,
  ms, meaningful only for type 1).
- **Audio** (`AudioTagHeader`): `SoundFormat` (4 bits: `10` = AAC) +
  `SoundRate`/`SoundSize`/`SoundType`. For AAC, an `AACPacketType` byte
  follows (`0` = sequence header = `AudioSpecificConfig`, `1` = one raw AAC
  access unit).
- **Data message** (type 18/15, `@setDataFrame`/`onMetaData`): an AMF0 String
  `"@setDataFrame"` + String `"onMetaData"` + an ECMA Array of metadata
  (`duration`, `width`, `height`, `framerate`, `audiocodecid`,
  `videocodecid`, …) — see `flv.md`'s "Script data" section.

`transmux` already demuxes RTMP → FLV-tag-shaped Audio/Video/Data messages
into its neutral `Media`/`Track` IR (`transmux/src/rtmp.rs`, `transmux/src/flv.rs`);
`rtmp-runtime` does **not** need to re-parse codec config headers or samples,
only to deliver the message body bytes intact.

---

## 10. `rtmp-runtime` mapping — session engine vs. delegated-to-transmux

Mirroring `rtsp-runtime`'s sans-IO design (driveable state machine, no
socket I/O baked in), `rtmp-runtime`'s job is the **session/transport**
layer; it hands opaque payload bytes onward rather than decoding them:

**`rtmp-runtime`'s job (the sans-IO session):**
- **Handshake** (§2): drive C0/C1/C2 (client) or S0/S1/S2 (server) through
  the Uninitialized → Version Sent → Ack Sent → Handshake Done states.
- **Chunk (de)assembly** (§3): basic header (1/2/3-byte csid+fmt) + message
  header (types 0/1/2/3) + extended timestamp, maintaining per-csid state
  (last timestamp, message length, type id, stream id) and per-direction max
  chunk size; reassembling chunked messages back into whole `message type
  id` + payload; and the reverse (chunking outbound messages). Timestamps
  are 32-bit and wrap every ~49.7 days — apply the serial-number-arithmetic
  handling the spec calls for (§1, `[RFC1982]`) when comparing/ordering them.
- **Protocol control messages** (§4): Set Chunk Size, Abort, Acknowledgement,
  Window Ack Size, Set Peer Bandwidth — updating/emitting session state
  (chunk size, ack windows) as a side effect of receipt.
  - **Note a genuine spec gap**: spec §5.4.3/§5.4.4 state *that* an
    Acknowledgement MUST follow receipt of a window's worth of bytes, but —
    unlike the timestamp field, where the spec's own §4 (this doc's §1)
    explicitly calls for `[RFC1982]` serial-number arithmetic on wraparound —
    the spec is **silent** on wraparound behavior for the Acknowledgement
    `sequence number` (spec §5.4.3) once cumulative bytes-received exceeds
    2^32. Do not assume the same RFC1982 treatment applies here just because
    it's used for timestamps elsewhere in this spec; treat the 32-bit
    counter's wraparound as an implementation decision to make explicitly
    (e.g. plain modular wraparound, since sequence numbers here are
    monotonic byte counts, not the reorderable values RFC1982 was designed
    for) and document it in `rtmp-runtime`'s own code, rather than inferring
    a specific behavior from this spec.
- **User Control messages** (§5.3, §6.7): Stream Begin/EOF/Dry,
  SetBufferLength, StreamIsRecorded, PingRequest/Response.
- **Command routing + the publish state machine** (§7): `connect` →
  Window Ack Size / Set Peer Bandwidth / StreamBegin / `_result` handshake;
  `createStream` → stream id allocation; `publish` (ingest) →
  `onStatus`-driven publish lifecycle; `deleteStream`; `play` (for
  completeness, playback direction). AMF0 encode/decode (§8) of command
  names, transaction ids, and command objects to drive this state machine —
  the session needs the *values*, not just opaque bytes, to route
  `connect`/`createStream`/`publish`/`deleteStream` correctly and to
  generate the right `onStatus` responses.

**Delegated to `transmux` (already implemented, see §9):**
- Interpreting **Audio (8)** / **Video (9)** message bodies as FLV
  `AudioTagHeader`/`VideoTagHeader` + codec config/samples.
- Interpreting **Data (18)** message bodies (`onMetaData`) as stream
  metadata.
- Producing the neutral `Media`/`Track` IR from the above, and muxing it
  onward to any other supported output container.

In short: `rtmp-runtime` owns everything **through** getting a clean,
reassembled `(message type id, timestamp, message stream id, payload)`
tuple out of the wire and driving the NetConnection/NetStream command
exchange that authorizes/positions a publish or play session; it treats
Audio/Video/Data payload bytes as opaque and hands them to `transmux`'s
existing RTMP/FLV spoke for actual codec-level interpretation.
