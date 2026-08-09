# SMPTE ST 2110-30:2025 — PCM Digital Audio

Source: SMPTE ST 2110-30:2025, "Professional Media over Managed IP Networks —
PCM Digital Audio" (revision of ST 2110-30:2017), 8 pages, approved
October 1, 2025. Transcribed from the vendored PDF
(`private/specs/smpte_st2110-30_2025_pcm_audio_rtp.pdf`) via `pdf2md`
(verified, exit 0); Tables 1–3 additionally cross-checked against a
page-rendered image of the PDF and found byte-for-byte identical to the
converter's text-layer extraction (no correction needed for this document).

## 1. Scope (§1)

Real-time, RTP-based transport of PCM digital audio streams over IP
networks, **by reference to AES67** — this document does not define its own
RTP payload format, it constrains AES67's. An SDP-based signaling method is
defined for the metadata needed to receive/interpret the stream.
Non-PCM digital audio, including compressed audio, is explicitly out of
scope.

**What this means for an implementer**: unlike ST 2110-20 (which defines its
own RTP payload header layout), ST 2110-30 defines no RTP payload header,
bit-field diagram, or packing structure of its own. The wire format is
AES67's — which itself follows IETF RFC 3190 ("RTP Payload Format for
12-bit DAT Audio and 20- and 24-bit Linear Sampled Audio") for linear PCM —
constrained by the clauses below. A conformant implementation needs the
AES67-2023 and RFC 3190 texts for the actual RTP payload byte layout; they
are out of scope for this transcription (AES67 is not in this repo's
vendored `private/specs/`).

## 2. Conformance notation (§2)

Same keyword conventions as ST 2110-20/-21: `shall`/`shall not` = mandatory;
`should`/`should not` = recommended, not mandatory; `may`/`need not` =
permitted; `reserved` = undefined now, shall not be used, may be defined
later; `forbidden` = `reserved` and will never be defined. Precedence:
normative prose > tables > formal languages > figures > other forms.

## 3. Normative references (§3)

- AES67-2023 — "AES standard for audio applications of networks –
  High-performance streaming audio-over-IP interoperability"
- IETF RFC 3190 — "RTP Payload Format for 12-bit DAT Audio and 20- and
  24-bit Linear Sampled Audio"
- IETF RFC 8866 — "SDP: Session Description Protocol"
- SMPTE ST 2110-10:2022 — "System Timing and Definitions"
  (`https://doi.org/10.5594/SMPTE.ST2110-10.2022`)
- SMPTE ST 2036-2:2008 — "Ultra High Definition Television – Audio
  Characteristics and Audio Channel Mapping for Program Production"
  (`https://doi.org/10.5594/SMPTE.ST2036-2.2008`)

## 4. Media Clock, RTP Clock, RTP Timestamps (§6.1)

- Media Clock and RTP Clock comply with ST 2110-10 §7.3 ("RTP Clock Offset")
  and §7.4 ("RTP Clock and Media Clock") respectively.
- **Both clocks run at the digital audio sampling rate** (not a fixed rate
  like ST 2110-20's 90 kHz).
- RTP timestamp complies with ST 2110-10 §7.5 ("RTP Timestamps – General
  Provisions") and §7.7 ("RTP Timestamps for Audio Streams").
- **48 kHz sampling is mandatory** for all conformant senders/receivers.
  44.1 kHz and/or 96 kHz **should** also be supported. No other rate is in
  scope.
- Senders/receivers conforming to any conformance level above Level A (§7,
  Tables 2/3) must support that level's stated sampling rates, packet
  times, and channel counts.

Notes (§6.1, informative):

1. To interoperate with other RTP implementations (e.g. AES67 itself),
   implementers should mind those standards' clock-offset provisions and
   the possibility the RTP Clock is offset from the Media Clock.
2. The IETF RFCs cited in §3 are Standards-Track but at varying maturity
   phases (not all "Internet Standard") — see RFC 6410 for the IETF's
   phase definitions.

## 5. AES67 constraints (§6.2)

### 5.1 General provisions (§6.2.1)

- Digital audio streams **shall conform to AES67**, including its SDP usage
  per RFC 8866, subject to this document's constraints.
- Notwithstanding AES67's SIP provisions, receivers **need not** support SIP
  or any other AES67-mentioned connection-management method.
- Senders/receivers **shall comply** with AES67 §7.1 ("Payload format and
  sampling rate").
- Senders/receivers **shall observe** the timing provisions of AES67 §7.5
  ("Sender timing and receiver buffering").
- **The Standard UDP Datagram Size Limit (ST 2110-10) shall be used** — i.e.
  ST 2110-30 streams do not use the Extended UDP Size Limit that ST
  2110-20/-21 make available to video.

### 5.2 Channel Order Convention (§6.2.2)

If channel order is signaled in SDP, it uses IETF RFC 3190's
`channel-order` parameter syntax. The `<convention>` **should** be
`SMPTE2110`.

Under the `SMPTE2110` convention, `<order>` is a parenthesized,
comma-separated list of Channel Grouping Symbols (Table 1). Example syntax:

```
a=fmtp:101 channel-order=SMPTE2110.(51,ST)
```
— first 6 channels = 5.1 surround group, next 2 = stereo group.

```
a=fmtp:101 channel-order=SMPTE2110.(M,M,M,M,ST,U02)
```
— 4 mono channels, then a stereo group, then a 2-channel Undefined group.

Rules:

- Any channels not matching a Table 1 grouping's quantity/order **shall** be
  grouped as **Undefined**.
- Undefined signals a mix that's unknown, unrepresentable, or otherwise
  non-conformant to the convention's other groupings.
- Undefined symbol = `U` + two-digit channel count (zero-padded), e.g. `U05`
  = a 5-channel Undefined group.
- If `channel-order` is absent, **all** channels are treated as Undefined.
- If the `channel-order`-declared channel count mismatches the stream's
  actual channel count, the undeclared channels are treated as Undefined.

**Table 1 — Channel Order Convention Grouping Symbols (§6.2.2)**

| Symbol | Channel qty | Group description | Channel order |
|---|---|---|---|
| `M` | 1 | Mono | Mono |
| `DM` | 2 | Dual Mono | M1, M2 |
| `ST` | 2 | Standard Stereo | Left, Right |
| `LtRt` | 2 | Matrix Stereo | Left Total, Right Total |
| `51` | 6 | 5.1 Surround | L, R, C, LFE, Ls, Rs |
| `71` | 8 | 7.1 Surround | L, R, C, LFE, Lss, Rss, Lrs, Rrs |
| `222` | 24 | 22.2 Surround | per SMPTE ST 2036-2:2008 Table 1 |
| `SGRP` | 4 | One SDI audio group | 1, 2, 3, 4 |
| `U01`…`U64` | as indicated by symbol (`Unn` = `nn` channels) | Undefined | none specified — order is Undefined |

Channel names/symbols in Table 1 are per SMPTE ST 2067-8, except: the 22.2
Grouping (order per ST 2036-2 Table 1) and the Undefined Grouping, both
defined in this document itself. Undefined is explicitly **not** a
Soundfield Group — it marks the *absence* of one.

Notes (§6.2.2, informative):

1. This convention exists so phase-coherent multichannel audio groups (or
   single mono channels) can be clearly defined in SDP. It makes no claim
   about a channel's higher-level purpose (e.g. "secondary language") and
   does not address object-based audio — only fixed multichannel groups
   common at time of publication.
2. The 5.1/7.1 orders in Table 1 are based on typical ordering found in
   SMPTE ST 2035, EBU R 123, and SMPTE ST 429-2.

## 6. Conformance Levels (§7)

- **Senders**: to claim a level, must support that level's sampling rate,
  packet time, and **at least one** channel count within Table 2's range for
  that level.
- **Receivers**: to claim a level, must support **all possible combinations**
  of sampling rate, packet time, and channel count within Table 3's range
  for that level.
- **All senders and receivers shall be compliant to Level A.**
- Senders/receivers may support more channels than a level requires, as
  long as they can still operate within that level's max channel count.
- Senders/receivers **should** declare every Table 2/3 level they support
  when signaling conformance.

**Table 2 — Senders Conformance Levels (§7)**

| Level | Sampling Clock Rate (Hz) | Packet Time (µs) | Number of Channels |
|---|---|---|---|
| A | 48000 | 1000 | 1 to 8 |
| AX | 96000 | 1000 | 1 to 4 |
| B | 48000 | 125 | 1 to 8 |
| BX | 96000 | 125 | 1 to 8 |
| C | 48000 | 125 | 9 to 64 |
| CX | 96000 | 125 | 9 to 32 |

**Table 3 — Receivers Conformance Levels (§7)**

| Level | Sampling Clock Rate (Hz) | Packet Time (µs) | Number of Channels |
|---|---|---|---|
| A | 48000 | 1000 | 1 to 8 |
| AX | 48000 | 1000 | 1 to 8 |
| AX | 96000 | 1000 | 1 to 4 |
| B | 48000 | 1000 | 1 to 8 |
| B | 48000 | 125 | 1 to 8 |
| BX | 48000 | 1000 | 1 to 8 |
| BX | 48000 | 125 | 1 to 8 |
| BX | 96000 | 1000 | 1 to 4 |
| BX | 96000 | 125 | 1 to 8 |
| C | 48000 | 1000 | 1 to 8 |
| C | 48000 | 125 | 1 to 64 |
| CX | 48000 | 1000 | 1 to 8 |
| CX | 48000 | 125 | 1 to 64 |
| CX | 96000 | 1000 | 1 to 4 |
| CX | 96000 | 125 | 1 to 32 |

Note (§7, informative): SDI carries at least 16 embedded audio channels; a
sender wanting to stay within Level A can split those into multiple AES67
streams, ideally grouped per the logical channel groupings of §6.2.2.

Observe the sender/receiver asymmetry: a receiver claiming e.g. Level BX
must handle *every* row listed for BX (four rate/packet-time/channel-count
combinations), whereas a sender claiming BX need only pick one channel
count within *each* rate/packet-time pair it actually uses — the two tables
encode fundamentally different compliance shapes, not just different
numbers.

## 7. Bibliography (§Bibliography, informative)

- EBU Recommendation R 123 — "EBU Audio Track Allocation for File Exchange"
- IETF RFC 6410 — "Reducing the Standards Track to Two Maturity Levels"
- SMPTE ST 428-12:2013 — "D-Cinema Distribution Master Common Audio
  Channels and Soundfield Groups"
- SMPTE ST 429-2:2023 — "D-Cinema Packaging – DCP Operational Constraints"
- SMPTE ST 2035:2020 — "Audio Channel Assignments for Digital Television
  Recorders"
- SMPTE ST 2059-1:2021 — "Generation and Alignment of Interface Signals to
  the SMPTE Epoch"
- SMPTE ST 2067-8:2013 — "Interoperable Master Format — Common Audio
  Labels"
- VSF TR-03 — "Transport of Uncompressed Elementary Stream Media over IP"

## 8. What this document leaves unstated

- **No RTP payload bit layout is defined in this document at all** — it
  normatively defers the entire wire format to AES67 (§6.2.1) and, through
  AES67, to RFC 3190 for linear PCM framing. An implementer needs both of
  those texts (neither vendored in this repo) to build the actual payload
  parser/serializer; this transcription cannot supply that layout without
  fabricating it.
- No packet-time-to-octet-count formula is given here (packet time is
  signaled/constrained by Table 2/3 and AES67 §7.5, but the byte-level
  consequence per channel/sample-format is AES67's, not this document's).
- SDP parameter names/syntax for signaling sampling rate, packet time, and
  channel count are not restated here — this document's only own SDP
  addition is `channel-order` (§6.2.2, via RFC 3190 syntax); the rest is
  inherited from AES67's own `a=fmtp` parameters, not reproduced in this
  standard's text.
- No conformance level exists between the receiver Table 3 rows shown for a
  given letter grade beyond exactly what's tabulated — e.g. there is no
  Level "B" 96 kHz row (that combination only appears starting at BX).
