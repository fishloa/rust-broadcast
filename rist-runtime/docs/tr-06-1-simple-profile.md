# VSF TR-06-1:2020 — RIST Protocol Specification — Simple Profile

Curated transcription for the `rist-runtime` crate, from
`private/specs/vsf_tr-06-1_2020_rist_protocol_simple_profile.pdf`
(31 pages, June 25 2020). This file is the field-semantics oracle for
the RIST Simple Profile — cite section numbers from here.

Normative references: IETF RFC 3550 (RTP), RFC 3551 (RTP/AVP),
RFC 4585 (RTP/AVPF), SMPTE ST 2022-1/-2/-7.

---

## §4 — RIST Profiles

RIST has multiple operational profiles with increasing complexity. Higher
profiles include all features of preceding profiles. This document defines
**Simple Profile**: basic interoperability and packet loss recovery. All
configuration is manual and done outside the protocol.

## §5 — Simple Profile

### §5.1 — Baseline Protocol

RTP SHALL be the baseline protocol for media transport. If an RTP standard
exists for a media type, that standard defines the RTP header fields (e.g.
SMPTE ST 2022-1/2 for MPEG-2 TS).

RTCP SHALL be used for feedback/control messages (RFC 3550).

#### §5.1.1 — Unicast Port Assignments

1. Sender transmits RTP media to receiver IP, even destination port **P**
   (2 ≤ P ≤ 65534). Receiver listens on port P (unidirectional).
   Sender MAY choose any source port M.

2. Sender periodically transmits compound RTCP (§5.2.1) to receiver
   port **P+1**. Sender MAY choose any source port R. Sender listens on
   port R for RTCP from receiver.

3. Receiver listens on port **P+1** for sender's RTCP. Source IP is S,
   source port is R'. Without NAT: R = R'. Receiver sends RTCP to
   IP S, port R', with source port P+1. Receiver uses S and R' from
   last valid RTCP received from sender.

4. Sender MAY allow manual configuration of source ports M and R.

5. Receiver MAY use UPnP for firewall configuration.

#### §5.1.2 — Multicast Port Assignments

1. Sender transmits RTP media to multicast destination port **P** (even,
   2 ≤ P ≤ 65534). Receiver listens on port P. Sender MAY choose any
   source port M.

2. Sender periodically transmits compound RTCP to port **P+1**, same
   multicast destination. Sender MAY choose any source port R. Sender
   listens on port P+1 for RTCP from receiver.

3. Receiver listens on port **P+1**, same multicast IP, for sender RTCP.

4. Receiver sends RTCP to same multicast address, port P+1. MAY choose
   any source port.

5. Sender MAY offer manual source port config. MAY use R=P+1.

### §5.2 — RTCP Support

Senders and receivers SHALL implement a minimal RTCP subset. For senders,
RTCP keeps state on NAT devices. For receivers, RTCP requests lost packet
retransmissions.

#### §5.2.1 — Compound RTCP Packets

Multiple RTCP packets concatenated without separators in one UDP payload
(RFC 3550 compound packet).

**Sender compound:**
- Sender Report (SR, §5.2.2) OR Empty RR (§5.2.3)
- SDES with CNAME (§5.2.5)
- RTT Echo Request or Response, if supported (§5.2.6)

**Receiver compound:**
- Receiver Report (RR, §5.2.4) or Empty RR (§5.2.3)
- SDES with CNAME (§5.2.5)
- NACK, if required (§5.3.2)
- RTT Echo Request or Response, if supported (§5.2.6)

**Timing requirements:**
1. Interval between successive RTCP packets SHALL be ≤100 ms.
2. Maximum RTCP data rate SHALL be ≤5% of average media rate. For very
   low bit-rate applications, requirement 1 takes precedence.

#### §5.2.2 — Sender Report (SR) RTCP Packets

RIST SR: no reception report blocks (RC=0). Sender MAY use SR or empty RR.

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|V=2|P|  RC=0  |  PT=SR=200    |         length=6              |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     SSRC of sender                            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|          NTP timestamp, most significant word                 |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|          NTP timestamp, least significant word                |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     RTP timestamp                             |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                  sender's packet count                        |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                  sender's octet count                         |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | Bits | RIST constraint |
|-------|------|-----------------|
| version (V) | 2 | SHALL be 2 |
| padding (P) | 1 | SHALL be 0 |
| reception report count (RC) | 5 | SHALL be 0 |
| packet type (PT) | 8 | 200 (SR) |
| length | 16 | SHALL be 6 |
| SSRC | 32 | sender's sync source identifier |
| NTP timestamp | 64 | wallclock time (seconds since 0h UTC Jan 1 1900, MSW=seconds, LSW=fraction). MAY set to 0 if no wallclock |
| RTP timestamp | 32 | same instant as NTP ts, in RTP clock units with same random offset as data packets |
| sender's packet count | 32 | total RTP data packets since start (reset on SSRC change) |
| sender's octet count | 32 | total payload octets (excl. header/padding) since start (reset on SSRC change) |

#### §5.2.3 — Empty Receiver Report (RR) RTCP Packets

Sender MAY use empty RR instead of SR to establish NAT state.

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|V=2|P|  RC=0  |  PT=RR=201    |         length=1              |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     SSRC of sender                            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | Bits | RIST constraint |
|-------|------|-----------------|
| V | 2 | 2 |
| P | 1 | 0 |
| RC | 5 | 0 |
| PT | 8 | 201 (RR) |
| length | 16 | 1 |
| SSRC | 32 | sender's sync source identifier |

#### §5.2.4 — Receiver Report (RR) RTCP Packets

RIST RR: exactly one report block (RC=1), for the sender's stream.

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|V=2|P|  RC=1  |  PT=RR=201    |         length=7              |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                   SSRC of packet sender                       |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                   Received stream SSRC                        |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
| fraction lost |     cumulative number of packets lost         |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|           extended highest sequence number received           |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                      interarrival jitter                      |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                       last SR (LSR)                           |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                  delay since last SR (DLSR)                   |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | Bits | RIST constraint |
|-------|------|-----------------|
| V | 2 | 2 |
| P | 1 | 0 |
| RC | 5 | 1 |
| PT | 8 | 201 |
| length | 16 | 7 |
| SSRC of packet sender | 32 | receiver's own SSRC |
| Received stream SSRC | 32 | sender's SSRC |
| fraction lost | 8 | per RFC 3550 §6.4.1 |
| cumulative packets lost | 24 | per RFC 3550 §6.4.1 |
| extended highest seq received | 32 | per RFC 3550 §6.4.1 |
| interarrival jitter | 32 | per RFC 3550 §6.4.1 |
| last SR (LSR) | 32 | per RFC 3550 §6.4.1 |
| delay since last SR (DLSR) | 32 | per RFC 3550 §6.4.1 |

#### §5.2.5 — SDES RTCP Packets

One chunk (SC=1), one item: CNAME.

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|V=2|P|  SC=1  |  PT=SDES=202  |         length                |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                   SSRC of packet sender                       |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|  CNAME=1      | name length   | user and domain name      ...
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
| user and domain name (cont).  |0 to 3 bytes=0 |       0      |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

- CNAME is an ASCII string, NOT null-terminated (length-prefixed).
- RFC 3550 recommends `user@host` form; RIST implementations MAY use
  the device IP in ASCII (e.g. `"192.168.129.10"`).
- SDES packet terminated with 1–4 zero bytes to reach 32-bit alignment.

#### §5.2.6 — RTCP RTT Echo Request/Response Packets (Optional)

Allows measuring Round Trip Time (RTT). Implemented as RTCP APP messages
(PT=204) with name "RIST" (0x52495354).

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|V=2|0| Subtype |  PT=APP=204   |         Length                |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                   SSRC of media source                        |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                      name (ASCII)                             |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|          Timestamp, most significant word                     |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|          Timestamp, least significant word                    |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|              Processing Delay (microseconds)                  |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                      Padding bytes                            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
                          ...
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | Bits | Semantics |
|-------|------|-----------|
| V | 2 | 2 |
| P | 1 | 0 |
| Subtype | 5 | **2** = RTT Echo Request, **3** = RTT Echo Response |
| PT | 8 | 204 (APP) |
| Length | 16 | 5 + X/4 where X = padding bytes (multiple of 4) |
| SSRC of media source | 32 | identifies the media flow |
| Name | 32 | 0x52495354 ("RIST") |
| Timestamp | 64 | arbitrary value in Request; echoed verbatim in Response. MAY be NTP format |
| Processing Delay | 32 | Request: SHALL be 0. Response: processing time in microseconds (interval between receiving Request and sending Response) |
| Padding | n×32 | optional, to match stream packet size. Multiple of 4 bytes. Response SHALL echo ≥same padding count |

**Multicast RTT rules:**
- Request: sent unicast to the participant whose RTT is desired (source IP
  of multicast packets on port P), to port P+1.
- Response: sent unicast to source of Request, port P+1.
- Devices SHALL accept both multicast and unicast RTCP on P+1.

### §5.3 — NACK-Based Recovery Protocol

#### §5.3.1 — Protocol Overview (Informative)

RIST uses NACK-based Selective Retransmission:
- Receiver detects packet loss → requests retransmission.
- Receiver buffer: Reorder Section + Retransmission Reassembly Section.
- Buffer is FIFO; arriving in-order packets advance the boundary.
- Packet loss detected at boundary between sections (sequence number gap).
- Lost packet MAY be requested multiple times.
- Buffer size manually configured (Simple Profile).

#### §5.3.2 — Retransmission Requests

Two types:
1. **Bitmask-based** (§5.3.2.1) — individual packet losses + short bursts.
2. **Range-based** (§5.3.2.2) — block losses.

Senders SHALL support both. Receivers MAY implement either or both.

##### §5.3.2.1 — Bitmask-Based Retransmission Requests

Uses Generic NACK (RFC 4585 §6.2): PT=205 (RTPFB), FMT=1.

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|V=2|P|  FMT=1 |    PT=205     |         length                |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                   SSRC of packet sender                       |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                   SSRC of media source                        |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
:              Feedback Control Information (FCI)               :
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | Bits | RIST constraint |
|-------|------|-----------------|
| V | 2 | 2 |
| P | 1 | 0 |
| FMT | 5 | 1 (Generic NACK) |
| PT | 8 | 205 (Transport-Layer FB) |
| length | 16 | n+2 (n = number of Generic NACK FCI fields) |
| SSRC of packet sender | 32 | receiver's SSRC (ignored by RIST sender) |
| SSRC of media source | 32 | identifies the flow; LSB distinguishes original (0) vs retransmitted (1). Receiver MAY use either value |

**FCI (Generic NACK, per FCI field = 32 bits):**

```
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|            PID                |              BLP              |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | Bits | Semantics |
|-------|------|-----------|
| PID (Packet ID) | 16 | RTP sequence number of first lost packet |
| BLP (Bitmask of following Lost Packets) | 16 | bit i (1=LSB, 16=MSB): set if packet PID+i is lost. Each FCI covers up to 17 packets. Multiple FCI fields SHOULD NOT overlap |

##### §5.3.2.2 — Range-Based Retransmission Requests

Uses RTCP APP (PT=204), name "RIST" (0x52495354), Subtype=0.

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|V=2|P| Sub=0   |  PT=APP=204   |         length                |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                   SSRC of media source                        |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                      name (ASCII)                             |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|              Packet Range Requests                            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | Bits | RIST constraint |
|-------|------|-----------------|
| V | 2 | 2 |
| P | 1 | 0 |
| Subtype | 5 | 0 (range request) |
| PT | 8 | 204 (APP) |
| length | 16 | n+2 (n = number of range request fields, max 16) |
| SSRC of media source | 32 | identifies the flow (LSB: 0=orig, 1=retransmit) |
| Name | 32 | 0x52495354 ("RIST") |

**Packet Range Request (per entry = 32 bits):**

```
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|    Missing Pkt Sequence Start | Number of addtl missing Pkts  |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | Bits | Semantics |
|-------|------|-----------|
| Missing Packet Sequence Start | 16 | RTP seq of first lost packet |
| Number of Additional Missing Packets | 16 | consecutive packets after start (0 = single packet; N+A inclusive) |

##### §5.3.2.3 — RTCP Packet Size Considerations (Informative)

Recommended: ≤16 requests per NACK packet (smaller packets survive congestion better).

#### §5.3.3 — Retransmitted Packets

Sender resends the packet with exact same sequence number and timestamp.
The **SSRC LSB** differentiates:
- **SSRC LSB = 0**: original packet
- **SSRC LSB = 1**: retransmission packet

Remaining 31 bits of SSRC are the same, allowing receiver to match
retransmissions to original flows.

Retransmission uses same transmission method as the original flow (same
IP dest, same bonding algorithm).

#### §5.3.4 — Burst Control (Informative)

Implementations SHOULD manage:
- **NACK bursts**: throttle back-to-back NACKs.
- **Retransmission bursts**: throttle retransmitted packets to avoid
  overloading the network.

#### §5.3.5 — SSRC Filtering (Informative)

Both Bitmask and Range NACK contain "SSRC of Media Source". Senders use
this to match requests with streams. Streams usually identified by
(src IP, dst IP, dst port), except when a source sends multiple streams
(different SSRCs) on the same address/port.

### §5.4 — Bonding Support (Optional)

Split or replicate an RTP stream across multiple network connections
(WiFi, LTE, etc.) for bandwidth aggregation or increased reliability.

A network connection = distinct (destination IP, destination UDP port).

**Operating rules:**
- Receiver listens to multiple RTP and RTCP making up a RIST stream over
  one or more connections. Re-aggregates into one buffer (§5.3).
- Receiver sends RTCP to ALL connections associated with a RIST stream.
- Receiver MAY send NACK on any or multiple connections.
- Replicated packets: same RTP sequence number and timestamp across all
  copies.
- Sender listens to NACK on ALL connections.
- Sender MAY retransmit on any or multiple connections.
- Sender MAY mix unicast and multicast destinations.
- A SMPTE ST 2022-7 Class-C receiver can receive a replicated RIST bonding
  stream (without retransmission) if Path Differential is within limits.

---

## Appendix B — Suggested Default Values (Informative)

| Parameter | Suggested Default |
|-----------|-------------------|
| NACK window (buffer size) | 1000 ms |
| Max retries per packet | 10 |
| Reorder section | 70 ms |

---

## Implementation scope for `rist-runtime`

The crate targets a **sans-IO** RIST Simple Profile implementation:

1. **Wire types** (Parse/Serialize, `no_std`):
   - RIST-specific RTCP APP messages: Range-Based NACK (Subtype=0, name="RIST"),
     RTT Echo Request (Subtype=2), RTT Echo Response (Subtype=3).
   - Generic NACK (RFC 4585 FMT=1 / PT=205) is already in `rtcp-packet`.
   - RIST compound RTCP packet builder/parser (sender + receiver compounds).

2. **Sans-IO state machines** (`no_std` + `alloc`):
   - `RistSender`: sequence tracking, retransmission buffer, SSRC LSB
     flipping, compound RTCP (SR or empty RR + SDES) generation,
     RTT Echo Response generation, NACK processing → retransmit queue.
   - `RistReceiver`: reorder + retransmission reassembly buffer, gap
     detection → NACK generation (bitmask + range), compound RTCP
     (RR + SDES + NACK) generation, RTT Echo Request generation +
     RTT measurement, optional bonding aggregation.

3. **Tokio adapter** (feature `tokio`):
   - UDP socket pair (port P + P+1), compound RTCP timer, NACK pacing.
