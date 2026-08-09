# VSF TR-06-1:2020 — RIST Simple Profile

Source: VSF Technical Recommendation TR-06-1:2020, "Reliable Internet Stream
Transport (RIST) Protocol Specification — Simple Profile", 31 pages, June 25
2020. CC BY-ND 4.0.

## 1. Overview

RIST Simple Profile = RTP baseline + RTCP feedback + NACK-based ARQ
retransmission. Configuration is manual (no in-band negotiation).

## 2. Baseline Protocol (§5.1)

- RTP (RFC 3550) for media transport
- RTCP for feedback/control
- If an RTP profile exists for the media type, use it (e.g. SMPTE 2022-1/2 for
  MPEG-2 TS)

### 2.1 Unicast Port Assignments (§5.1.1)

1. Sender → receiver: RTP to even port P (2–65534), source port M (arbitrary)
2. Sender → receiver: RTCP to port P+1, source port R (arbitrary)
3. Receiver → sender: RTCP to IP=S, port=R', source port P+1
   - S and R' from last valid RTCP packet received from sender

### 2.2 Multicast Port Assignments (§5.1.2)

Same even-port rule. RTP on P, RTCP on P+1, same multicast address.

## 3. RTCP Support (§5.2)

Compound RTCP packets per RFC 3550.

**Sender compound:** SR or empty-RR + SDES(CNAME) + [RTT Echo if supported]
**Receiver compound:** RR or empty-RR + SDES(CNAME) + [NACK if needed] + [RTT Echo if supported]

Timing: interval ≤ 100 ms; bandwidth ≤ 5% of average media rate.

### 3.1 Sender Report — SR (§5.2.2, PT=200)

Standard RFC 3550 SR with RIST constraints:
- V=2, P=0, RC=0, PT=200, length=6
- No reception report blocks
- Fields: SSRC, NTP timestamp (64-bit), RTP timestamp (32-bit),
  sender's packet count (32-bit), sender's octet count (32-bit)

### 3.2 Empty Receiver Report — RR (§5.2.3, PT=201)

- V=2, P=0, RC=0, PT=201, length=1
- SSRC only; no report blocks

### 3.3 Receiver Report — RR (§5.2.4, PT=201)

Standard RFC 3550 RR with exactly 1 report block:
- V=2, P=0, RC=1, PT=201, length=7
- Fields: SSRC of sender, received-stream SSRC, fraction lost (8),
  cumulative lost (24), extended highest seq (32), jitter (32),
  last SR (32), delay since last SR (32)

### 3.4 SDES (§5.2.5, PT=202)

- V=2, P=0, SC=1, PT=202
- One chunk: SSRC + CNAME item (type=1, length, ASCII string)
- Padded to 32-bit boundary with 1–4 zero bytes
- CNAME: RFC 3550 recommends `user@host`; RIST allows IP address

### 3.5 RTT Echo Request/Response (§5.2.6, PT=APP=204)

Stated purpose (§5.2.6, informative framing): "to allow RIST endpoints to
measure the Round Trip Time (RTT) to the remote endpoint. The RTT information
can be used by receivers to optimize their retransmission requests." Support
is optional. **This is the entirety of what TR-06-1 says about RTT's role in
retransmission timing** — it does not state a formula relating measured RTT to
the retransmission-request interval or to buffer sizing; Appendix B's
132 ms/1000 ms/70 ms/7-retries figures (below) are stated as flat defaults, not
as a function of measured RTT. A concrete RTT-driven timing algorithm, if one
exists, is not in this document — check whether TR-06-2 (Main Profile timing,
2024; also vendored at `private/specs/vsf_tr-06-2_2024_rist_timing_main_profile.pdf`,
out of scope for this transcription) states one before assuming Simple-Profile
behaviour extends there.

RTCP APP message with name `"RIST"` (0x52495354).

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|V=2|0| Subtype |  PT=APP=204   |           Length              |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                   SSRC of media source                        |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                   name (ASCII) = "RIST"                       |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|              Timestamp, most significant word                 |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|              Timestamp, least significant word                |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|              Processing Delay (microseconds)                  |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                        Padding bytes                          |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | Bits | Notes |
|---|---|---|
| Subtype | 5 | `2` = RTT Echo Request, `3` = RTT Echo Response |
| PT | 8 | `204` (APP) |
| Length | 16 | `5 + X/4` where X = padding bytes (multiple of 4) |
| SSRC | 32 | Media source SSRC |
| name | 32 | `0x52495354` = ASCII "RIST" |
| Timestamp | 64 | Arbitrary (may be NTP format); request sets, response echoes |
| Processing Delay | 32 | Request: 0. Response: microseconds between receiving request and sending response |
| Padding | n×32 | Optional, arbitrary content, multiple of 4 bytes |

## 4. NACK-Based Recovery (§5.3)

### 4.1 Protocol Overview (§5.3.1, informative)

General operation:
- Once loss is detected, the receiver requests retransmission of the lost packet(s).
- The receiver implements a buffer to accommodate one or more network round-trip
  delays and packet re-ordering.
- A lost packet may be requested multiple times (the spec does not bound this
  other than via the Appendix B retry-count default — see Appendix B, below;
  it does not itself state a suppression rule for "don't re-request a packet
  that's already been requested/received").

Receiver data flow:
- Packets arrive into a **Reorder Section**, which absorbs out-of-order packets
  (this is also the mechanism that supports bonding of multiple channels).
- After the Reorder Section, packets cross into a **Retransmission Reassembly
  Section**. Packet loss is detected **at the boundary between these two
  sections**, by looking for discontinuities in the RTP sequence number.
- The combined buffer behaves as a FIFO: an arriving in-order packet pushes
  existing packets forward one or more positions. Out-of-order packets whose
  sequence number falls between the newest packet in the reorder section and the
  oldest packet in the retransmission-reassembly section are placed by sequence
  number.
- **Where exactly loss is detected is explicitly left to the implementation**:
  - a minimum-delay implementation detects loss at the input of the buffer —
    packets that arrive out of order then cause *extra* (spurious) retransmission
    requests;
  - a bonding-capable implementation sizes the reorder section large enough to
    absorb the worst-case delay differential between the bonded paths.
- Recommendation (non-quantified): provision for short network outages; buffer
  size is "a function of the round-trip time, packet jitter, and these outages,
  if present" — the spec does not give a formula for this, only Appendix B's
  flat suggested defaults (§4.4/Appendix B).
- **Simple Profile specifically**: buffer size is manually configured at both
  the sending and receiving ends — there is no in-band buffer-size negotiation.

### 4.2 Retransmission Requests (§5.3.2)

RIST Simple Profile defines two retransmission-request wire formats:
- **Bitmask-based** (§5.3.2.1) — suited to individual losses and short loss bursts.
- **Range-based** (§5.3.2.2) — suited to block losses.

Conformance: **RIST senders shall support both** request types. **RIST receivers
may implement either one, or both** — a receiver is not required to implement
both.

#### 4.2.1 Bitmask-Based Retransmission — Generic NACK (§5.3.2.1)

RFC 4585 §6.2 Generic NACK:

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|V=2|P|  FMT=1  |   PT=205      |           length              |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                   SSRC of packet sender                       |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                   SSRC of media source                        |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
:              Feedback Control Information (FCI)               :
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

- P=0, FMT=1, PT=205 (Transport-Layer FB)
- Length = n+2 where n = number of Generic NACK FCI fields
- SSRC of packet sender: ignored by RIST sender
- SSRC of media source: identifies the flow

Each FCI (32 bits):

| Field | Bits | Notes |
|---|---|---|
| PID | 16 | RTP sequence number of lost packet |
| BLP | 16 | Bitmask: bit i=1 means PID+i+1 is also lost (LSB=bit 1) |

Each FCI can request up to 17 packets. Multiple FCI fields allowed.

#### 4.2.2 Range-Based Retransmission (§5.3.2.2)

RTCP APP message with name `"RIST"` (0x52495354):

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|V=2|0| Subtype=0|  PT=APP=204  |           Length              |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                   SSRC of media source                        |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                   name (ASCII) = "RIST"                       |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|   Missing Pkt Sequence Start  | Number of addtl missing Pkts  |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

- Subtype=0, PT=204 (APP)
- Length = n+2 where n = number of range request fields
- Max 16 range requests per packet
- SSRC: media source (LSB distinguishes original vs retransmit)

Each Range Request (32 bits):

| Field | Bits | Notes |
|---|---|---|
| Missing Pkt Sequence Start | 16 | First lost RTP sequence number |
| Number of Additional Missing Pkts | 16 | Count of consecutive additional packets; 0 = only the start packet |

#### 4.2.3 RTCP Packet Size Considerations (§5.3.2.3, informative)

- Both NACK wire formats can carry multiple requests in one RTCP packet.
- Rationale given: under congestion, smaller packets have a higher probability
  of delivery.
- **The spec places no hard limit** on how many requests may be included in a
  single RTCP NACK packet in general. It only *recommends* implementers
  restrict the count, suggesting "no more than 16 requests per packet."
- This is an **informative recommendation**, not a "shall" — do not confuse it
  with the range-based format's own normative cap. §5.3.2.2 (Range-Based
  Retransmission Requests) separately states the packet **shall** have a
  maximum of 16 range requests (that limit is on the wire format itself, not
  this general congestion-avoidance guidance). The bitmask format (§5.3.2.1)
  has **no stated per-packet limit on the number of Generic NACK FCI fields**
  at all — the 16-ish figure here is the only guidance that applies to it, and
  it is advisory only.

### 4.3 Retransmitted Packets (§5.3.3)

- Sender retransmits with same sequence number and timestamp
- SSRC LSB distinguishes: `0` = original, `1` = retransmission
- Upper 31 bits of SSRC identical between original and retransmit
- The retransmitted packet **shall use the same transmission method** as the
  rest of that flow: same destination IP if unicast; if the flow is bonded
  (split across multiple destinations), the same path-selection algorithm used
  for original packets picks the retransmit's path; if the flow is multicast,
  other receivers on the group may also pick up the retransmitted copy.
- Sender-side responsibility on receiving a NACK, spelled out here: identify
  the flow via the SSRC field, locate the originally-sent packet, and resend an
  exact copy (same seq/timestamp) with the SSRC LSB flipped to 1. The spec does
  not prescribe *how* the sender looks the packet up (e.g. a send-side ring
  buffer) — that storage/lookup mechanism is left to the implementation.

### 4.4 Burst Control (§5.3.4, informative)

Two burst scenarios the spec flags as recommendations (not requirements):

1. **NACK packet bursts** (both Bitmask and Range modes): if a receiver needs
   to send a large number of back-to-back NACK packets, it should take care not
   to create too large a burst. Which request mode is most efficient (fewest
   NACK packets for a given loss pattern) depends on the loss pattern itself —
   no selection algorithm is given.
2. **Retransmitted-packet bursts**: a sender can receive a NACK requesting a
   large number of retransmissions in one shot. It is recommended that
   implementations **throttle** the retransmitted packets so as not to overload
   the network.
   - Called out explicitly: a range-based request can, in the extreme, request
     retransmission of every possible RTP sequence number by setting "Number of
     addtl missing Pkts" to 65535 for an arbitrary start — i.e. a single 32-bit
     range-request field can nominally demand 65536 retransmissions. An
     implementation must be prepared to throttle/reject this rather than
     attempt it literally.

**For RIST Simple Profile, the spec explicitly states these techniques' details
are left to the discretion of the implementer.** There is no stated back-off
algorithm, no stated suppression rule (e.g. "don't re-NACK a sequence number
already NACKed within window T"), and no stated retransmission-throttle rate.
Anything an ARQ engine does here beyond Appendix B's flat retry-count/interval
figures (see Appendix B below for the numeric defaults) is implementation
policy, not spec-mandated behaviour.

### 4.5 SSRC Filtering (§5.3.5, informative)

Both NACK types carry SSRC of media source. Senders match requests to streams
via SSRC (+ IP/port when multiple streams share an address).

## 5. Bonding Support (§5.4)

Optional. Split stream across multiple paths (bandwidth bonding) or replicate
for reliability. Network connection = distinct (dest IP, dest port) pair.

Requirements if implemented:
- Receiver: aggregate RTP from multiple connections into one buffer
- Receiver: send RTCP to all connections
- Sender: replicated packets same seq+timestamp; retransmit on any connection
- SMPTE 2022-7 Class-C compatible for replicated bonding

## Appendix A — Retransmission Request Examples (informative)

Worked example straight from the spec (§Appendix A), useful as a hand-checkable
wire-format test vector (not a network capture — see the fixture assessment in
the issue-#741 investigation notes for that distinction).

Setup: RIST device receiving a stream from `192.168.1.10:3000`, `SSRC =
0xAABBCC00`. Sequence 99 received; 100 lost; 101, 102 received; 103–122 lost
(20 consecutive packets); 123+ received. Retransmission-request packets are
sent to `192.168.1.10:3001`.

**Bitmask-Based request** (`length=4` → n+2=4 → 2 Generic NACK FCI fields):

```
|V=2|0| FMT=1  |   PT=205      |  length=4     |
|         SSRC of packet sender (ignored)       |
| 0xAA | 0xBB | 0xCC | 0x00 or 0x01              |   (SSRC of media source)
| PID=100       | BLP=1111111111110000          |
| PID=117       | BLP=0000000000011111          |
```

- FCI 1: PID=100 signals packet 100 itself lost. BLP bit1(=101)=0, bit2(=102)=0
  (both received); bits 3–16 (=103–116) = 1 (lost).
- FCI 2: PID=117 signals packet 117 itself lost. BLP bits 1–5 (=118–122) = 1
  (lost); bits 6–16 = 0 (not being requested here).
- Together: 100, 103–116, 117, 118–122 = 21 lost packets signalled (1 + 14 + 1
  + 5), matching the 1 (pkt 100) + 20 (103–122) loss pattern in the setup.

**Range-Based request** (`length=4` → n+2=4 → 2 range-request fields):

```
|V=2|0|Subtype=0|  PT=APP=204   |  length=4     |
| 0xAA | 0xBB | 0xCC | 0x00 or 0x01              |   (SSRC of media source)
| 0x52 (R) | 0x49 (I) | 0x53 (S) | 0x54 (T)       |   (name = "RIST")
| Start=100     | Additional=0                   |
| Start=103     | Additional=19                   |
```

- Range 1: Start=100, Additional=0 → packet 100 only.
- Range 2: Start=103, Additional=19 → packets 103 through 103+19=122 inclusive
  (20 packets).

Spec's own footnote: "the contents of the fields in red are fixed by this
standard and never change" — i.e. `PT`, `FMT`/`Subtype`, and the `"RIST"` name
field are wire constants; only `length`, `SSRC`, `PID`/`BLP`, and
`Start`/`Additional` vary per-request.

## Appendix B — Suggested Default Values (informative)

> **Correction (this transcription):** an earlier draft of this file listed
> "NACK processing start: When packet enters retransmission reassembly",
> "Max retries: 10", "Reorder buffer size: 25 ms (adjustable)", and
> "Retransmission buffer size: Function of RTT + jitter" for this appendix.
> None of those four entries match the source PDF. They have been replaced
> below with the actual Appendix B text, re-verified against
> `private/specs/vsf_tr-06-1_2020_rist_protocol_simple_profile.pdf` via
> `pdf2md --engine textlayer` (page 31).

Stated verbatim framing: "RIST implementations complying with this
specification are manually configured by the user. In the absence of user
input, the following default parameters are suggested":

| Parameter | Suggested value |
|---|---|
| Receiver Buffer | 1000 ms |
| Sender Buffer | ≥ Receiver Buffer |
| Reorder Section | 70 ms |
| Number of Retransmission Requests per Packet | 7 |

The interval between retransmission requests is **derived**, not independently
stated: "the receiver buffer minus the reorder section divided by the number of
retransmission requests." For the values above that is
`(1000 - 70) / 7 = 132.86`, and the spec states the rounded outcome directly:
**132 ms**.

Notes for an ARQ engine building against these:
- "Receiver Buffer" is the *total* buffer depth (Reorder Section +
  Retransmission Reassembly Section combined) — it is not a separate pool from
  the 70 ms reorder figure; the reassembly section's time budget is the
  remainder, `1000 - 70 = 930 ms`, which is what the 132 ms interval is derived
  from (`930 / 7 ≈ 132.86`).
- "Number of Retransmission Requests per Packet" is, in effect, the maximum
  number of times a given lost packet is re-requested — 7. The spec does not
  name it "max retries," but that is its
  effect: after 7 requests spaced ~132 ms apart (≈924 ms, just inside the
  930 ms reassembly-section budget), the packet falls off the end of the
  buffer and further requesting it is moot.
- "Sender Buffer ≥ Receiver Buffer" is the only stated sender-side buffer-depth
  constraint — no absolute sender buffer figure is given, only the
  relationship.
- All of the above are **suggested defaults for the manually-configured Simple
  Profile**, not protocol minimums/maximums — nothing in §5 makes these
  numbers normative ("shall"); Appendix B is explicitly informative.
