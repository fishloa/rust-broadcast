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

Receiver buffer = Reorder Section + Retransmission Reassembly Section.
Loss detected at boundary via RTP sequence number gaps. FIFO; out-of-order
packets placed by sequence number.

### 4.2 Bitmask-Based Retransmission — Generic NACK (§5.3.2.1)

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

### 4.3 Range-Based Retransmission (§5.3.2.2)

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

### 4.4 Retransmitted Packets (§5.3.3)

- Sender retransmits with same sequence number and timestamp
- SSRC LSB distinguishes: `0` = original, `1` = retransmission
- Upper 31 bits of SSRC identical between original and retransmit

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

## Appendix B — Suggested Default Values

| Parameter | Suggested Value |
|---|---|
| NACK processing start | When packet enters retransmission reassembly |
| Max retries | 10 |
| Reorder buffer size | 25 ms (adjustable) |
| Retransmission buffer size | Function of RTT + jitter |
