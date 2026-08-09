# VSF TR-06-2:2024 — RIST Main Profile

Source: VSF Technical Recommendation TR-06-2:2024, "Reliable Internet Stream
Transport (RIST) Protocol Specification — Main Profile", approved June 12
2024, 73 pages. CC BY-ND 4.0. Vendored at
`private/specs/vsf_tr-06-2_2024_rist_timing_main_profile.pdf`. Transcribed
with `pdf2md --engine hybrid` (exit code 0 — every `0x…` token in the
document verified byte-identical against the PDF's text layer) and
cross-checked against `pdftotext -layout` for every clause quoted below.

**Filename vs. content note:** the vendored PDF's filename says "timing
main profile," but TR-06-2 is not a timing-specific document — it is the
full RIST Main Profile specification (stream multiplexing/tunneling, DTLS,
Pre-Shared Key encryption, NULL packet deletion, high-bitrate operation, EAP
Authentication in Annex D). This transcription covers **only the timing-
relevant content** (per the brief this file was written for): what TR-06-2
says about retransmission/ARQ timing, buffer sizing, and its relationship to
TR-06-1 Simple Profile. It does not cover DTLS, PSK encryption mechanics, or
the Annex D authentication algorithm in full — those are out of scope for
`rist-runtime`'s current ARQ-timing question.

## 1. Headline finding: TR-06-2 does NOT define an RTT-driven ARQ
retransmission-timing formula either

This transcription was written to answer a specific question: TR-06-1
§5.2.6 states RTT "can be used to optimize retransmission requests" but
gives no formula, and TR-06-1's only concrete ARQ timing numbers are
Appendix B's flat, RTT-independent defaults (1000 ms receiver buffer, 70 ms
reorder section, 7 retransmission requests, 132 ms derived interval — see
`docs/tr-06-1-simple-profile.md` Appendix B).

**Having read the whole of TR-06-2, no RTT-driven retransmission-interval
formula, and no reorder-buffer/retransmission-buffer sizing formula, exists
in this document either.** There is no section titled or dedicated to
"timing," and a full-text search of the converted document for `buffer`,
`reorder`, `millisecond`/`ms`, `depth`, `jitter`, `latency` (beyond §8.2,
below) turns up nothing beyond what is transcribed in this file. If an
RTT-driven ARQ timing algorithm exists in the RIST specification family, it
is not in TR-06-1 or TR-06-2 — this transcription can rule those two
documents out as its source.

What TR-06-2 **does** contain that bears on ARQ timing is narrower and
different in kind from a formula: a single concrete numeric constraint
relating the existing TR-06-1 "7 retries" figure to sequence-number
wraparound at high bitrate (§8.2, below), plus non-ARQ timing content for
the Main-Profile-only tunnel/keep-alive and PSK-authentication layers. Each
is transcribed below with its exact clause citation; nothing here is
inferred beyond what is stated.

## 2. Relationship to TR-06-1 Simple Profile (§4, §5.1, §9)

- §4 ("RIST Profiles", informative): "RIST has multiple operational
  profiles, corresponding to increasing levels of complexity and
  functionality. Higher profiles include all the features and functionality
  of the preceding profiles." TR-06-2 Main Profile adds, on top of TR-06-1
  Simple Profile: stream multiplexing, tunneling, DTLS encryption, PSK
  encryption, NULL packet deletion, and high-bitrate/high-latency operation
  (extended sequence numbers).
- §5.1: "Streams running through the tunnel shall comply with VSF TR-06-1,
  RIST Simple Profile." The RTP/RTCP wire formats transcribed in
  `docs/tr-06-1-simple-profile.md` (Generic NACK, Range-Based retransmission
  request, RTT Echo, SR/RR/SDES) are unchanged by Main Profile — Main
  Profile's additions are a GRE-over-UDP (RFC 8086) tunnel wrapped *around*
  a Simple-Profile stream, plus the extended-sequence-number RTCP/RTP
  additions in §8.3–8.4 (below).
- §9 ("Compatibility between RIST Main Profile and Simple Profile Devices"):
  "RIST Main Profile adds several features to Simple Profile, but the
  underlying transport mechanism is still Simple Profile. Therefore, RIST
  Main Profile devices shall support operation in Simple Profile mode." In
  Simple Profile the sender is always the tunnel client and the receiver is
  always the tunnel server; if a Main Profile sender is talking to a
  receiver of unknown profile, the choice between Main and Simple Profile
  "shall be manually configured" (no auto-negotiation of profile is
  defined).

None of this changes or extends TR-06-1's ARQ timing figures — it changes
transport (tunneled vs. direct) and adds an orthogonal encryption/framing
layer.

## 3. §8.2 — High Bitrate/High Latency Operation: the one ARQ-adjacent
numeric constraint (informative)

Exact text (§8.2, page 35 printed / PDF page 36):

> "The RTP sequence number is only 16 bits, which means it wraps around
> every 65,536 packets. If the RIST link is carrying a 100 Mb/s transport
> stream, with the usual seven transport packets per RTP payload, the RTP
> sequence number will wrap around every 6.9 seconds. When using ARQ and
> allowing for the recommended 7 retries, this means that the maximum
> supportable round-trip delay is around one second. This is a significant
> limitation, which gets even worse as the bit rates go up. Therefore, the
> sequence number must be extended to support this operation, ideally to 32
> bits."

Traceability of every number in that paragraph:

- **65,536** — 2^16, the range of the RTP sequence number field (unchanged
  from TR-06-1/RFC 3550; not new to TR-06-2).
- **100 Mb/s** and **seven transport packets per RTP payload** — the
  worked example's own stated inputs, not derived from anything else.
- **6.9 seconds** — *derived*, and independently reproducible: at 100 Mb/s
  with 188-byte MPEG-TS packets, `100,000,000 / (188 × 8) ≈ 66,489.4`
  TS-packets/s; at 7 TS packets per RTP payload, `66,489.4 / 7 ≈ 9,498.5`
  RTP-payloads/s; the 16-bit sequence space wraps after `65,536 / 9,498.5 ≈
  6.899 s`, which rounds to the stated 6.9 s. (Recomputed here, not taken on
  faith — matches the spec's own figure.)
- **7 retries** — this is *not* a new TR-06-2 number. It is TR-06-1
  Appendix B's "Number of Retransmission Requests per Packet" default of 7
  (see `docs/tr-06-1-simple-profile.md` Appendix B), reused by reference
  here (the spec calls it "the recommended 7 retries" — TR-06-1's own
  informative Appendix B default, not a "shall").
- **"around one second"** — *derived, and loosely stated as an
  approximation ("around"), not an exact figure*: TR-06-1 Appendix B's own
  derived retransmission-request interval for the same 7-retry count is
  132 ms (`(1000 - 70) / 7 ≈ 132.86 ms`, rounded), and `7 × 132 ms ≈ 924 ms`
  — consistent with, but not identical to, "around one second." TR-06-2
  does not show its arithmetic for this figure; it is presented as a
  round-number conclusion, and this transcription is not stretching it into
  a precise value it does not claim to be.

**What this constraint actually says, restated precisely:** it is not an
RTT-driven *retransmission-timing* rule (it does not tell you when to space
NACKs, or how big to make a buffer). It is a **feasibility bound**: at a
given bitrate and packets-per-RTP-payload, the 16-bit sequence number
wraps fast enough that if the round-trip time approaches the total time
budget consumed by 7 retransmission requests at TR-06-1's suggested 132 ms
spacing (i.e., "around one second"), a lost packet's sequence number can
wrap back around *before* its retransmission arrives, making the
retransmission ambiguous or unusable. This is the stated motivation for
extending the sequence number to 32 bits (§8.3) — it is a wraparound-safety
argument, not an ARQ-scheduling formula. An ARQ engine implementing high-
bitrate operation needs the 32-bit extended-sequence-number mechanism
(§8.3–8.4, next section) specifically to make this bound moot, rather than
needing a new retry-timing algorithm.

## 4. §8.3–8.4 — Extended Sequence Numbers (normative, mechanism only, no
new timing numbers)

Relevant to an ARQ engine because it changes what a NACK's sequence number
means at high bitrate, though it carries no new timing constants:

- §8.3: an RTP header extension (RFC 3550 §5.3.1 generic extension,
  identifier `0x5249` = ASCII "RI", `Length=1`) carries an `E` bit; when
  `E=1`, a 16-bit "Sequence Number Extension" field supplies the MSBs, so
  the effective RTP sequence number becomes 32 bits (16-bit RTP header
  field as LSBs + this 16-bit field as MSBs). The same extension header
  also optionally carries NULL Packet Deletion metadata (`N` bit, `Size`,
  `T`, NPD bitmask) — unrelated to timing.
- §8.4: TR-06-1's Generic NACK and Range-Based retransmission-request
  formats (16-bit `PID`/`Start` fields) are extended with a new RTCP
  packet, **EXTSEQ**, that "conveys the higher 16 bits of the sequence
  number for the following NACK packet" in the same compound RTCP packet
  (ordering: `..., EXTSEQ, NACK, CNAME, EXTSEQ, NACK, ...`). If a receiver
  needs to NACK packets whose high-order 16 bits differ, it "shall" send
  them as separate NACK packets, each preceded by its own EXTSEQ. No
  interval, buffer, or retry-count number is introduced by this section —
  it only extends the *addressing space* of the existing TR-06-1 wire
  formats, not their timing.

## 5. Main-Profile-only timing: GRE tunnel keep-alive (§5.6.2, normative)

This is genuine Main-Profile-specific timing, but it governs *tunnel/session
liveness*, not ARQ retransmission — it is the closest thing in either
document to a concrete "timing model" with hard numbers, so it is
transcribed in full. Exact requirements (§5.6.2, page 19 printed / PDF page
20):

| Requirement | Value | Normative strength |
|---|---|---|
| Keep-alive transmission frequency | between 1 second and 10 seconds | "shall be sent periodically... shall be between" |
| Tunnel timeout | 60 seconds | "should be" (recommended default, not mandatory — TR-06-2:2022 changed this from a fixed value to a recommended default; implementations may use other values) |
| Startup burst | minimum 3, maximum 10 back-to-back keep-alive messages | "shall send" |

Behavioural rules around these numbers (all "shall"):
- Tunnel client starts sending as soon as enabled; tunnel server starts
  sending as soon as it receives the client's first message.
- Tunnel timeout is declared when an endpoint receives *no* data at all
  (keep-alive or actual traffic) for the timeout duration — actual stream
  traffic counts as a liveness signal, not just keep-alive messages.
- On timeout, an endpoint stops sending keep-alives/traffic to the remote
  end and releases session resources (tears the session down) — this is a
  hard stop, not a retry.

This 1–10 s / 60 s pair is a **session-liveness timeout**, structurally
unrelated to the packet-level ARQ retry timing in TR-06-1 Appendix B (which
operates on a ~100 ms scale per-packet). Do not conflate the two — a
`rist-runtime` implementation of Main Profile tunneling would need this
table for its keep-alive state machine, but it says nothing about how to
pace NACKs or size a reorder buffer.

## 6. PSK/EAP-SRP authentication timing (Annex D, informative-on-the-numbers)

Also Main-Profile-only, also not ARQ timing. Two places in Annex D mention
round-trip time or retry timing, and both are explicit that **no formula or
default is given** — the numbers are left to the implementer:

- **D.5 Re-Authentication** (page 66 printed / PDF page 67, normative): "The
  interval between successive re-authentication sessions between a server
  and a given client shall be no less than 60 seconds." This is the one
  hard, "shall"-level number in Annex D's timing content — a **minimum**
  interval, not a target or default.
- **D.6 UDP Transport Considerations** (page 67 printed / PDF page 68,
  normative process, non-normative numbers): the EAP-SRP authentication
  protocol runs over UDP and needs its own retry/timeout handling, stated
  as:
  > "The number of retries is left at the discretion of the implementer but
  > should be no less than three. The timeout is also left at the
  > discretion of the implementer, and it should be a multiple of the
  > round-trip time between server and client. While RIST includes
  > mechanisms to measure the round-trip time, such mechanisms are only
  > available after the connection is established. Therefore, the
  > round-trip time needs to be determined by means outside of this
  > Specification."

  Restated precisely, because this is the passage closest in spirit to
  "RTT-driven retransmission timing" anywhere in either document, and it is
  important not to over-read it: it does **not** give a multiple (not "3×
  RTT", not "N× RTT" for any stated N) — "a multiple of the round-trip
  time" is qualitative guidance, and the RTT value itself is explicitly
  stated to be unavailable from RIST's own RTT-measurement mechanism at the
  point this timeout is needed (authentication happens *before* a RIST
  session — and hence RTT Echo per TR-06-1 §5.2.6 — is established), so the
  spec says outright that RTT "needs to be determined by means outside of
  this Specification." The only hard number is "no less than three"
  retries, framed as a floor, not a recommended value.
  - The client-side timeout for expected server messages during
    authentication (same section) has the same "left to the discretion of
    the implementer... should be a multiple of the round-trip time"
    language, with no numeric floor at all.
  - The Passphrase Request/Response timeout (D.3.4.8/D.3.4.9) is likewise
    "left to the discretion of the implementer" for both timeout and retry
    count, with no floor, ceiling, or multiplier given.

This is authentication-handshake retry policy (EAP messages, once, at
session setup), not the ARQ engine's per-packet NACK/retransmit loop —
included here only because it is the only other place in TR-06-2 that
mentions RTT in a retry/timing context, and because the brief for this
transcription specifically asked to check whether TR-06-2 states an
RTT-driven formula. It does not, here either.

## 7. PSK Nonce rotation timing (§7.2, informative note; not ARQ-relevant)

Noted for completeness, explicitly out of scope for ARQ: §7.2 requires a
new PSK nonce "at least every time the sequence counter/number of the GRE
packet wraps to zero," and includes a CPU-load caveat (key regeneration is
CPU-intensive; "Receivers can choose to mitigate the risk of excessive CPU
loading by limiting how often they process a change in the Nonce") with no
numeric limiting rate given — again left to the implementer. Unrelated to
retransmission or buffer timing; mentioned only so a future reader doesn't
assume this transcription missed it.

## 8. Summary table — every timing-relevant number found in TR-06-2

| # | Value | Clause | Normative strength | Relation to ARQ |
|---|---|---|---|---|
| 1 | 65,536 (2^16) sequence space | §8.2 | fact (RFC 3550 field width) | Bounds max loss-recovery window before extension |
| 2 | 6.9 s wrap time @ 100 Mb/s, 7 TS/RTP | §8.2 | derived example, informative | Motivates 32-bit sequence extension |
| 3 | 7 retries (reused from TR-06-1 App. B) | §8.2 | informative, reused reference | Same figure as TR-06-1, not new |
| 4 | ~1 s max RTT (rounded, "around") | §8.2 | derived approximation, informative | Feasibility bound, not a scheduling rule |
| 5 | 1–10 s keep-alive interval | §5.6.2 | "shall" | Tunnel liveness, not ARQ |
| 6 | 60 s tunnel timeout | §5.6.2 | "should" (default) | Tunnel liveness, not ARQ |
| 7 | 3–10 startup keep-alive burst | §5.6.2 | "shall" | Tunnel liveness, not ARQ |
| 8 | ≥60 s re-authentication interval | Annex D.5 | "shall" (floor) | Auth handshake, not ARQ |
| 9 | ≥3 auth retries | Annex D.6 | "should" (floor) | Auth handshake, not ARQ |
| — | Auth timeout = "a multiple of RTT" | Annex D.6 | qualitative, no multiplier given | Auth handshake, not ARQ |

**Nothing in this table is a per-packet retransmission-interval or
buffer-sizing formula.** For that, `rist-runtime`'s ARQ engine has only
TR-06-1 Appendix B's flat defaults (1000 ms receiver buffer / 70 ms reorder
section / 7 retries / 132 ms derived interval) — TR-06-2 does not supersede,
refine, or add an RTT-driven variant of them.
