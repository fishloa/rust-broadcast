# RFC 8331 — RTP Payload for SMPTE ST 291 Ancillary Data (ST 2110-40)

_Source: RFC 8331 "RTP Payload for SMPTE ST 291-1 Ancillary Data" (T. Edwards,
February 2018), §2/§2.1 (pp. 4–10) and §3.1/§4 (pp. 11–13), transcribed line-for-line
from the vendored `specs/rfc8331_anc_rtp.txt`._

> Scope note: this file curates only the RTP-transport-specific material — the
> RTP header semantics for this payload type, the RFC 8331 §2.1 payload-header
> fields (`Extended Sequence Number`/`Length`/`ANC_Count`/`F`/`reserved`), and
> the §3.1/§4 media-type/clock-rate registration. The **per-ANC-packet fields**
> (`C`/`Line_Number`/`Horizontal_Offset`/`S`/`StreamNum`/`DID`/`SDID`/
> `Data_Count`/`User_Data_Words`/`Checksum_Word`/`word_align`), the 10-bit-word
> parity rule, and the `Checksum_Word` computation are **already fully
> curated** in [`anc_packet_291.md`](anc_packet_291.md) — reused unchanged
> here, not re-transcribed. This file is a sibling of
> [`st_2038.md`](st_2038.md): that file covers the ST 2038 MPEG-2 TS/PES
> transport of the same ST 291-1 content; this one covers the RFC 8331 / ST
> 2110-40 RTP transport.

---

## §2 Packet diagram (RFC 8331 Figure 1, specs/rfc8331_anc_rtp.txt:180–210)

```
    0                   1                   2                   3
    0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |V=2|P|X| CC    |M|    PT       |        sequence number        |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                           timestamp                           |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |           synchronization source (SSRC) identifier            |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |   Extended Sequence Number    |           Length=32           |   } RFC 8331
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+   } payload
    | ANC_Count=2   | F |                reserved                   |   } header
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |C|   Line_Number=9     |   Horizontal_Offset   |S| StreamNum=0 |   } per-ANC-packet
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+   } fields — see
    ...                                                                 } anc_packet_291.md
```

The RTP fixed header (`V`/`P`/`X`/`CC`/`M`/`PT`/sequence number/timestamp/SSRC)
is RFC 3550 §5.1/§5.3.1 verbatim, already implemented by the `rtp-packet` crate
(`rtp_packet::RtpPacket`) — this crate's `rtp` feature rides its payload, never
reimplementing the fixed header.

---

## RTP-header semantics for this payload type (specs/rfc8331_anc_rtp.txt:231–258)

> "RTP packet header fields SHALL be interpreted as per RFC 3550 [RFC3550],
> with the following specifics:"

### `timestamp` (32 bits)

Interpreted "in a similar fashion to RFC 4175":

- **Progressive scan video**: the timestamp denotes the sampling instant of
  the **frame** to which the ANC data in the RTP packet belongs. RTP packets
  **MUST NOT** include ANC data from multiple frames, and all RTP packets with
  ANC data belonging to the same frame **MUST** have the same timestamp.
- **Interlaced video**: the timestamp denotes the sampling instant of the
  **field** to which the ANC data in the RTP packet belongs. RTP packets
  **MUST NOT** include ANC data packets from multiple fields, and all RTP
  packets belonging to the same field **MUST** have the same timestamp.
- If the sampling instant does not correspond to an integer value of the
  clock, the value **SHALL** be truncated to the next lowest integer, with no
  ambiguity.

So: **one frame's (or one field's) worth of ANC data per RTP packet, never
mixed** — this crate does not enforce that invariant across a *stream* of RTP
packets (that is a session-layer concern outside a stateless per-packet
parser), but a single [`AncRtpPayload`](../src/rtp.rs) always represents
exactly one RTP packet's payload, consistent with the rule.

### Marker bit (`M`, 1 bit)

> "The marker bit set to '1' indicates the last ANC data RTP packet for a
> frame (for progressive scan video) or the last ANC data RTP packet for a
> field (for interlaced video)."

`M` lives in the RTP fixed header (`rtp_packet::RtpPacket::marker`), not in the
ANC-specific payload header — this crate does not duplicate it; callers set
`RtpPacket::marker` per the rule above when assembling packets.

---

## §2.1 Payload Header Definitions (specs/rfc8331_anc_rtp.txt:259–325)

> "The ANC data RTP payload header fields are defined as:"

### `Extended Sequence Number` (16 bits)

> "The high-order bits of the extended 32-bit sequence number, in network byte
> order. This is the same as the Extended Sequence Number field in
> RFC 4175 [RFC4175]."

**Out of scope note**: RFC 4175 reconstructs the full 32-bit extended sequence
number by combining this 16-bit field with the RTP fixed header's 16-bit
`sequence number`, using a **stateful sliding-window algorithm** (tracking the
most-recently-seen extended value per SSRC to resolve which 16-bit "epoch" a
new low 16 bits belongs to, across wraparound and packet loss/reordering).
That reconstruction is a **session/depacketiser-layer** concern — it requires
per-stream state this crate's stateless, per-packet wire parser does not (and
should not) hold. [`AncRtpPayload::extended_sequence_number`](../src/rtp.rs)
exposes the raw 16-bit field only; reconstructing the full 32-bit counter is
the caller's responsibility, exactly as RFC 8331 defers it to RFC 4175.

### `Length` (16 bits)

> "Number of octets of the ANC data RTP payload, beginning with the 'C' bit of
> the first ANC packet data header, as an unsigned integer in network byte
> order. Note that all word_align fields contribute to the calculation of the
> Length field."

So `Length` excludes the 8-byte payload header itself (`Extended Sequence
Number` + `Length` + `ANC_Count` + `F` + `reserved`) and counts every
per-ANC-packet record **including its trailing `word_align` padding**. This
crate never stores `Length` as an independent field: it is always recomputed
from the actual `anc_packets` on serialize, and on parse the declared value is
validated by requiring the per-ANC-packet loop (bounded by `ANC_Count`) to
consume *exactly* `Length` bytes — any mismatch (too few bytes for the
declared count, or leftover unconsumed bytes) is rejected rather than
silently trusted.

### `ANC_Count` (8 bits)

> "This field is the count of the total number of ANC data packets carried in
> the RTP payload, as an unsigned integer. A single ANC data RTP packet
> payload cannot carry more than 255 ANC data packets.
>
> If more than 255 ANC data packets need to be carried in a field or frame,
> additional RTP packets carrying ANC data MAY be sent with the same RTP
> timestamp but with different sequence numbers. ANC_Count of 0 indicates that
> there are no ANC data packets in the payload (for example, an RTP packet
> that carries no actual ANC data packets even though its marker bit indicates
> the last ANC data RTP packet in a field/frame). If the ANC_Count is 0, the
> Length will also be 0."

Like `Length`, `ANC_Count` is never an independent stored field — it is always
`anc_packets.len()` on serialize (rejected if it would exceed 255), and on
parse it is the wire value used as the per-ANC-packet-loop bound, cross-checked
against `Length` by the same "consumed bytes must match declared `Length`"
rule above.

### `F` (2 bits)

> "These two bits relate to signaling the field specified by the RTP timestamp
> in an interlaced SDI raster. A value of 0b00 indicates that either the video
> format is progressive or that no field is specified. A value of 0b10
> indicates that the timestamp refers to the first field of an interlaced
> video signal. A value of 0b11 indicates that the timestamp refers to the
> second field of an interlaced video signal. The value 0b01 is not valid.
> Receivers SHOULD ignore an ANC data packet with an F field value of 0b01 and
> SHOULD process any other ANC data packets with valid F field values that are
> present in the RTP payload."

**Design decision**: `0b01` is a **per-ANC-packet-payload** SHOULD-ignore
recommendation, not a whole-payload parse failure — and RFC 8331 states it
right alongside "process any other ANC data packets ... present", i.e. the
guidance is about what a *receiver* does with the data, not a wire-validity
rule the *parser* enforces. Per this project's decode-completeness principle
(never silently drop data the wire actually carried), `FieldSense`
(../src/rtp.rs) is a `#[non_exhaustive]` labelled enum with **four** variants
— `ProgressiveOrUnspecified` (`0b00`), `Invalid` (`0b01`), `Field1` (`0b10`),
`Field2` (`0b11`) — and parsing an `F` value of `0b01` **succeeds**, producing
`FieldSense::Invalid`. Any SHOULD-ignore behavior (e.g. discarding the ANC
packets in a payload whose `F` is `Invalid`) is left entirely to the caller;
this crate does not silently drop or reject the packet.

### `reserved` (22 bits)

> "The 22 reserved bits of value '0' follow the F field to ensure that the
> first ANC data packet header field in the payload begins 32-bit
> word-aligned with the start of the RTP header to ease implementation."

Always written as `0` on serialize; **validated as `0` on parse** — a nonzero
value is rejected (`Error::ReservedNotZero`). Unlike ST 2038's leading
`'000000'` bits (`st_2038.md`/`anc_packet_291.md`, tolerated but not enforced
zero on parse — a deliberate, differently-scoped design choice made for that
transport), RFC 8331's `reserved` here has an explicit **normative purpose**
(32-bit word alignment) stated in the same sentence as the "value '0'"
requirement, so this crate enforces it strictly. This crate additionally
validates that each per-ANC-packet `word_align` padding (`anc_packet_291.md`)
is all-zero on parse, for the same reason — RFC 8331 states its value is "0"
bits, and there is no stated tolerance for a non-conformant sender here as
there is for ST 2038's differently-purposed leading bits.

---

## §3.1 / §4 — Media type and clock rate

### §3 Payload Format Parameters / §3.1 Media Type Definition (specs/rfc8331_anc_rtp.txt:588–612)

> "This RTP payload format is identified using the 'video/smpte291' media
> type... Type name: video ... Subtype name: smpte291 ... Rate: RTP timestamp
> clock rate. When an ANC data RTP stream is to be associated with an RTP
> video stream, the RTP timestamp rates SHOULD be the same to ensure that ANC
> data packets can be associated with the appropriate frame or field.
> Otherwise, a 90 kHz rate SHOULD be used."

So the media type is `video/smpte291`, and the RTP timestamp clock rate is
**90 kHz** by default (matching video's own conventional rate), unless the
ANC stream is grouped with a specific video stream at a different rate — this
crate exposes this as `ANC_RTP_MEDIA_TYPE` (`"video/smpte291"`) and
`ANC_RTP_DEFAULT_CLOCK_RATE` (`90_000`) constants for callers building
SDP/session-description material; the crate itself has no SDP layer.

### §4 SDP Considerations (specs/rfc8331_anc_rtp.txt:718–772)

RFC 8331's own worked SDP example:

```
a=rtpmap:112 smpte291/90000
```

— `112` is an example dynamic payload type (negotiated per session, not fixed
by the spec); `smpte291` is the encoding name; `90000` is the clock rate in Hz.
This crate does not assign or validate a payload type value itself (RTP `PT`
is a `rtp_packet::RtpPacket` field, dynamically negotiated out-of-band via SDP
per RFC 3550/RFC 4855) — the worked value `112` is used only as an example
payload type in this crate's runnable examples, not a hardcoded requirement.

---

## Summary: what's new here vs. `anc_packet_291.md`

| Concern | Curated in |
|---|---|
| RTP fixed header (`V`/`P`/`X`/`CC`/`M`/`PT`/seq/timestamp/SSRC) wire format | `rtp-packet` crate (RFC 3550), not re-curated here |
| `timestamp`/`M` *semantics* for this payload type | **this file** |
| `Extended Sequence Number`/`Length`/`ANC_Count`/`F`/`reserved` (RFC 8331 §2.1 payload header) | **this file** |
| Per-ANC-packet fields (`C`.."word_align") + parity/checksum math | `anc_packet_291.md` (unchanged) |
| Media type / clock rate (§3.1/§4) | **this file** |
