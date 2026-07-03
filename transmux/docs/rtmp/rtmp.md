# RTMP 1.0 — wire format transcription

Transcribed from **Adobe RTMP Specification 1.0** (Parmar & Thornburgh,
December 2012), the sections this crate implements. AMF0 value markers come from
the companion **AMF0 Specification** (Adobe, December 2007) referenced by
RTMP §7. All multi-byte integers are **big-endian** except the RTMP message
stream id (little-endian) and where AMF0 states otherwise.

---

## 5.2 Handshake

An RTMP connection begins with a handshake of three fixed chunks each way:
C0/C1/C2 from the client, S0/S1/S2 from the server (§5.2, §5.2.1).

### 5.2.2 C0 / S0 — version (1 byte)

| Field   | Bytes | Value                                          |
|---------|-------|------------------------------------------------|
| version | 1     | RTMP version. This crate uses **3** (plain RTMP)|

### 5.2.3 C1 / S1 — 1536 bytes

| Field  | Bytes | Description                                        |
|--------|-------|----------------------------------------------------|
| time   | 4     | Epoch timestamp (may be 0). Big-endian.            |
| zero   | 4     | MUST be all zeros.                                 |
| random | 1528  | Random data (used to distinguish the two peers).   |

Total = 4 + 4 + 1528 = **1536**.

### 5.2.4 C2 / S2 — 1536 bytes (echo)

| Field       | Bytes | Description                                             |
|-------------|-------|---------------------------------------------------------|
| time        | 4     | The `time` from the peer's S1 (for C2) / C1 (for S2).   |
| time2       | 4     | Timestamp at which the peer's S1/C1 was read.           |
| random echo | 1528  | The peer's S1/C1 `random` field, echoed verbatim.       |

Total = **1536**.

---

## 5.3 Chunking

After the handshake the connection multiplexes messages as a stream of chunks.
The maximum chunk size defaults to **128** bytes and is renegotiated with the
Set Chunk Size control message (§5.4.1). A message longer than the chunk size is
split across multiple chunks; the chunk size is maintained independently per
direction.

### 5.3.1 Chunk format (§5.3.1)

```
+--------------+----------------+--------------------+--------------+
| Basic Header | Message Header | Extended Timestamp |  Chunk Data  |
+--------------+----------------+--------------------+--------------+
```

- **Basic Header** — 1 to 3 bytes: chunk stream id (csid) + chunk type (`fmt`).
- **Message Header** — 0, 3, 7 or 11 bytes, selected by `fmt`.
- **Extended Timestamp** — 0 or 4 bytes (present iff the header timestamp field
  reads `0xFFFFFF`).
- **Chunk Data** — up to the current chunk size.

### 5.3.1.1 Chunk basic header

The 2 low bits of the first byte are `fmt`; the low 6 bits are the csid encoding.
csid 0, 1, 2 are reserved; csid 2 is used for low-level protocol control
messages. Implementations SHOULD use the smallest form that holds the id.

| csid value in byte 0 | Form   | Bytes | csid computed as                              |
|----------------------|--------|-------|-----------------------------------------------|
| 3–63                 | 1-byte | 1     | the 6-bit value itself                        |
| 0                    | 2-byte | 2     | second byte + 64  (range 64–319)              |
| 1                    | 3-byte | 3     | (third byte)·256 + second byte + 64 (64–65599)|

```
1-byte:  0 1 2 3 4 5 6 7        2-byte: |fmt|0 | cs id-64 (8) |
        |fmt| cs id (6) |       3-byte: |fmt|1 | cs id-64 (16, little-endian) |
```

For the 3-byte form the 16-bit `cs id - 64` is stored **little-endian**
(`(third byte)·256 + second byte`).

### 5.3.1.2 Chunk message header (four `fmt` types)

**Type 0 (fmt=0, 11 bytes)** — full header. MUST start a chunk stream or when
the timestamp goes backward.

| Field           | Bytes | Notes                                                  |
|-----------------|-------|--------------------------------------------------------|
| timestamp       | 3     | Absolute. `0xFFFFFF` ⇒ Extended Timestamp present.     |
| message length  | 3     | Total message payload length (big-endian).             |
| message type id | 1     | See §7.1 message type ids.                             |
| message stream id | 4   | **little-endian**.                                     |

**Type 1 (fmt=1, 7 bytes)** — no message stream id (inherits it).

| Field           | Bytes | Notes                              |
|-----------------|-------|------------------------------------|
| timestamp delta | 3     | `0xFFFFFF` ⇒ Extended Timestamp.   |
| message length  | 3     |                                    |
| message type id | 1     |                                    |

**Type 2 (fmt=2, 3 bytes)** — only a timestamp delta; length, type id and
stream id inherit from the preceding chunk on this csid.

| Field           | Bytes |
|-----------------|-------|
| timestamp delta | 3     |

**Type 3 (fmt=3, 0 bytes)** — no header. Length, type id, stream id and
timestamp delta all inherit from the preceding chunk on the same csid. Used for
every chunk of a message after the first.

### 5.3.1.3 Extended Timestamp (4 bytes)

Present when a Type 0 timestamp, or a Type 1/2 timestamp delta, equals
`0xFFFFFF`. It carries the full 32-bit timestamp / delta (big-endian). It is
also present on a Type 3 chunk when the most recent Type 0/1/2 chunk for that
csid indicated an extended timestamp.

---

## 5.4 Protocol control messages

Protocol control messages use message type ids **1, 2, 3, 5, 6**, MUST be sent
on chunk stream id **2** with message stream id **0**, and take effect on
receipt (timestamps ignored).

| Id | Name                          | Payload                                             |
|----|-------------------------------|-----------------------------------------------------|
| 1  | Set Chunk Size (§5.4.1)       | bit0=0 + chunk size (31 bits). 1..=0x7FFFFFFF.       |
| 2  | Abort Message (§5.4.2)        | chunk stream id (32 bits) to discard.               |
| 3  | Acknowledgement (§5.4.3)      | sequence number = bytes received so far (32 bits).  |
| 5  | Window Acknowledgement Size (§5.4.4) | window size (32 bits).                       |
| 6  | Set Peer Bandwidth (§5.4.5)   | window size (32 bits) + limit type (1 byte).        |

Set Peer Bandwidth limit type: `0` Hard, `1` Soft, `2` Dynamic (§5.4.5).

---

## 6 Message header (§6.1.1) and message types

An RTMP message carried over the chunk stream has: Message Type (1 byte),
payload Length (3 bytes, big-endian), Timestamp (4 bytes, big-endian) and
Message Stream Id (3 bytes, big-endian). Over the chunk stream these are carried
by the chunk headers above.

### 7.1 Message type ids

| Type id | Message                                        |
|---------|------------------------------------------------|
| 1–6     | Protocol control / user control (§5.4, §6.2)   |
| 4       | User Control (§6.2)                            |
| 8       | Audio (§7.1)  — body = FLV `AudioTagHeader` + data |
| 9       | Video (§7.1)  — body = FLV `VideoTagHeader` + data |
| 15 / 18 | Data message: AMF3 / AMF0 (`@setDataFrame`/`onMetaData`, §7.1.2) |
| 16 / 19 | Shared object: AMF3 / AMF0 (§7.1.3)            |
| 17 / 20 | Command message: AMF3 / AMF0 (§7.1.1)          |

Audio (8) and Video (9) message **bodies are FLV tag bodies** (the same
`AudioTagHeader`+data / `VideoTagHeader`+data layout as an FLV tag payload —
Adobe FLV v10.1 Annex E §E.4.2 / §E.4.3), which is why this crate routes them
through the FLV spoke to reach the IR.

---

## 7 Command messages / AMF0

Command (type 20) and data (type 18) message bodies are **AMF0**-encoded
(RTMP §7.1.1 / §7.1.2). A command body is: command name (String), transaction id
(Number), command object (Object or Null), then optional arguments.

### AMF0 value markers (AMF0 Specification §2)

| Marker | Value | Encoding                                                        |
|--------|-------|----------------------------------------------------------------|
| Number (§2.2)      | 0x00 | 8-byte IEEE-754 double, big-endian.                 |
| Boolean (§2.3)     | 0x01 | 1 byte (0 = false, non-zero = true).                |
| String (§2.4)      | 0x02 | U16 length (big-endian) + UTF-8 bytes.              |
| Object (§2.5)      | 0x03 | repeated (U16-string key + value); ends with object-end. |
| Null (§2.7)        | 0x05 | no payload.                                         |
| ECMA array (§2.10) | 0x08 | U32 associative-count + (key + value)*; ends with object-end. |
| Object End (§2.11) | 0x09 | preceded by an empty (U16 length 0) key.            |

**Commands** (§7.2):

- `connect` (§7.2.1.1): name `"connect"`, transaction id `1` (Number), a command
  Object of name-value pairs (`app`, `flashVer`, `tcUrl`, `objectEncoding`, …).
- `createStream` (§7.2.1.3): name `"createStream"`, transaction id, command
  object (usually Null).
- `play` (§7.2.2.1): name `"play"`, transaction id, Null, Stream Name (String),
  optional Start/Duration/Reset.
- `publish` (§7.2.2.6): name `"publish"`, transaction id, Null, Publishing Name
  (String), Publishing Type (String: `"live"` / `"record"` / `"append"`).
- `@setDataFrame` / `onMetaData` (§7.1.2, data message type 18): AMF0 String
  `"@setDataFrame"`, String `"onMetaData"`, then an ECMA array of stream
  metadata.

**AMF3** (command type 17, data type 15, object encoding 3) is **not
implemented** here — AMF0 is the mainstream encoding used by RTMP live
publishing (`connect` `objectEncoding` 0) and FLV. AMF3 command/data bodies are
surfaced as an unparsed `Amf3` command variant carrying the raw body.
