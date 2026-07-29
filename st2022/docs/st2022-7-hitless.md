# SMPTE ST 2022-7 — Seamless Protection Switching (hitless redundancy)

_Sources:
- **SMPTE ST 2022-7:2019**, "Seamless Protection Switching of RTP Datagrams", 11
  pages, approved 2018-12-26 (current edition). Fetched free from
  `https://pub.smpte.org/latest/st2022-7/st2022-7-2019.pdf`. Render-verified
  locally via `pdf2md`'s `textlayer` engine, **exit code 0** — every token
  round-tripped cleanly, no diagnostics.
- **SMPTE ST 2022-7:2013**, "Seamless Protection Switching of SMPTE ST 2022 IP
  Datagrams", 13 pages (predecessor edition, superseded by the 2019 revision).
  Fetched free from `https://pub.smpte.org/doc/st2022-7/20131011-pub/st2022-7-2013.pdf`.
  Cited below only where it states something the 2019 text dropped, softened,
  or generalized — every such place is flagged explicitly, since an
  implementation should follow the **2019 (current) text** as authoritative.

See [`README.md`](README.md) for full provenance._

Clause numbers below are the 2019 edition's own (§4/§5/§6/§7/Annex A) unless
marked "(2013 §n)". Conformance notation is identical in both editions (§2):
all text is normative except the Introduction, sections/paragraphs marked
"Informative", and "Note:" paragraphs.

---

## 1. Scope (§1)

ST 2022-7 defines requirements for **multiple redundant streams of RTP
packets** (2019 generalizes 2013's fixed "two streams" to "at least two",
§6) that allow a receiver to reconstruct a single output stream via seamless,
per-datagram protection switching. It constrains only the **RTP-and-above**
content of the redundant streams; it says nothing about how the streams are
carried at L2–L4 (that is deliberately left open — see §3 below).

## 2. Redundancy model (§6 "Creation of Streams for Seamless Reconstruction")

Normative requirements on the transmitter:

- **shall** transmit at least two streams, each carrying copies of every RTP
  datagram. **Both editions say this identically** — 2013 §6 already reads
  verbatim "shall transmit **at least two** streams". (An earlier draft of this
  document claimed 2013 required *exactly* two and that 2019 relaxed it; that
  was wrong, and it was caught by the adversarial fidelity audit. The only
  two-vs-more wording change between editions is in the Definitions clause —
  2013 §5.10 "two" became 2019 §4.10 "two or more" — which is a definition, not
  a change to this normative requirement.)
- **The RTP header and the RTP payload shall be identical for each datagram
  copy.**
- "The seamless reconstruction method described herein makes no assumptions
  about the Ethernet or IP headers of the source streams" — i.e. copies
  **may** be sent to different IP destinations and/or different UDP ports
  (§4.3's definition of "datagram copies", below); only the RTP-and-above
  bytes are constrained.
- Note (2019 §6, informative): "In copies of a SMPTE ST 2022-6 RTP datagram,
  the `VSID` field, as part of the RTP payload, will be identical across the
  datagram copies" — consistent with (and implied by) the "RTP payload shall
  be identical" requirement above, since `VSID` lives inside the ST 2022-6
  Payload Header (see [`st2022-6-framing.md`](st2022-6-framing.md) §3.1),
  which is part of the RTP payload.

**Version note**: 2013 §6 stated this more strongly and specifically for the
HBR case: *"For Class HBR streams, as specified in SMPTE ST 2022-6 the RTP
timestamp shall be required. Furthermore, the VSID field ... is part of the
RTP payload, and shall be identical across the datagram copies."* The 2019
text drops the explicit "shall" for `VSID` identity and folds it into an
informative Note instead (quoted above). This is flagged as an unresolved
discrepancy between editions in [`README.md`](README.md) — treat "RTP header
and RTP payload shall be identical" (both editions, unambiguous "shall") as
the actual load-bearing normative requirement; the `VSID`-specific language
in either edition is redundant with (implied by) that broader requirement,
not an independent constraint.

## 3. Definitions load-bearing for a receiver implementation (§4)

| Term | §4.n | Definition |
|---|---|---|
| **datagram copies** | §4.3 | "set of redundant RTP datagrams, perhaps transmitted to a different IP destination or port, but having identical contents in the RTP header and RTP payload" |
| input stream | §4.5 | "stream of RTP datagrams in accordance with RFC 3550, which might represent essence or FEC" (Note lists ST 2022-1..6, ST 2110-20/30/40, AES67 as example RTP stream specs this can wrap) |
| output stream | §4.8 | "reconstructed datagram stream that results from processing the multiple copies of the input stream" |
| seamless reconstruction | §4.10 | successful creation of a reconstructed output stream from ≥2 (potentially impaired) input streams such that the reconstructed stream's RTP header and payload are **identical** to the input stream(s) |
| class HBR stream | §4.1 | payload bit rate ≥ 270 Mbit/s |
| class SBR stream | §4.2 | payload bit rate < 270 Mbit/s |

## 4. Correlating duplicate datagrams — exactly which fields a receiver compares

This is the one piece of normative substance that actually drives an
implementation, so it is worth stating precisely, separating **what the
standard mandates** from **what is left to the implementer**.

### 4.1 The formal definition of "the same datagram" (normative)

By §4.3 + §6: two RTP datagrams are copies of the same logical datagram if
and only if their **RTP header (all 12 bytes: `V,P,X,CC,M,PT`, sequence
number, timestamp, SSRC) and RTP payload (payload header + media/FEC
payload, in full) are byte-identical.** Nothing about the surrounding
Ethernet/IP/UDP framing (source/destination MAC, IP address, or UDP port) is
constrained — those are explicitly allowed to differ between copies (§4.3,
§6, and 2013 Annex B "Stream Differentiation": *"the reconstruction method
... makes no assumptions about the contents of the Ethernet or IP headers of
either stream. Only the RTP packet contents are required to be
identical."*).

So, formally, a fully faithful comparator would byte-compare the entire RTP
header + payload of every candidate datagram. That is correct but
expensive — full-payload compare of every arriving packet against every
buffered candidate does not scale to HBR rates (~270k pkt/s and up). The
standard's own informative Annex A explains the practical shortcut:

### 4.2 The practical bounded comparator (informative, Annex A — "left to the implementer")

- **RTP sequence number** is the primary, cheap correlator: it increments by
  exactly 1 per datagram (guaranteed by the underlying stream spec — e.g.
  ST 2022-6 §6.3, see [`st2022-6-framing.md`](st2022-6-framing.md)), and
  copies of the same datagram carry the **same** sequence number (it is part
  of the identical RTP header).
- **But the 16-bit sequence number is not sufficient alone** once the
  receiver's buffering/skew window can span more datagrams than `2^16`
  (65,536) — Annex A's own worked numeric example: a Class C (high-skew)
  receiver processing a 1080p60 HD-SDI (ST 2022-6) stream needs roughly a
  300 ms window, which holds **≈81,000 packets** — larger than one
  sequence-number rollover period. "Therefore it is impossible to tell by
  sequence number alone the relative values of `Pn` while maintaining the
  buffers implied for a class C receiver."
- The fix: **RTP timestamp** (32 bits, required for HBR streams, identical
  across copies by the same "RTP header shall be identical" requirement) is
  used **in addition to** the sequence number to disambiguate rollovers —
  either by using the timestamp directly to correlate, or by "augmenting"
  the monotonically increasing sequence number with a function of the
  timestamp difference (Annex A, verbatim: *"the monotonically increasing
  sequence numbers can be augmented with a function of the difference
  between the timestamps to correctly correlate the arriving packets"*).
- **The standard explicitly declines to mandate a specific matching
  algorithm**: §7 (2019) ends with *"Note: The exact the method of
  reconstruction is left to the implementer."* [sic, "the" duplicated in the
  source PDF]. So "select whichever copy arrives first, discard the other"
  is a reasonable, common implementation choice consistent with the
  standard's invariants — but it is **not itself a normative requirement**;
  the standard mandates only the *inputs* that make correlation and
  selection possible (identical RTP header+payload across copies,
  sequence-number monotonicity, identical/required timestamps for HBR), not
  the selection policy itself.

### 4.3 What this means for a receiver's minimum required parse

To implement duplicate-detection/selection per this standard, a receiver
needs, at minimum, an **RTP header parse** (RFC 3550 — 12 fixed bytes,
generic, not ST-2022-6-specific) to extract:

- **sequence number** (16-bit `u16`) — primary correlation key, monotonic mod 2^16.
- **timestamp** (32-bit `u32`) — required disambiguator once the skew/buffer
  window can exceed one sequence-number rollover (in practice: always
  worth carrying for HBR-class streams, per the Annex A analysis above).
- **SSRC** (32-bit) — not itself a documented part of the matching key in
  Annex A's worked example, but is part of the "RTP header shall be
  identical" invariant, and is the natural way to bind a given pair (or set)
  of ports/sockets to the *same* logical redundant stream if multiple
  sessions share a receiving process.

Notably, this parse is **generic RFC 3550 RTP header parsing** — it needs no
ST 2022-6-specific (or ST 2022-1..5-specific) payload decoding at all. That
matches ST 2022-7's own framing: "input stream" (§4.5) is deliberately
generic over any RFC-3550-conformant stream (ST 2022-1..6, ST 2110-20/30/40,
AES67), so the dedup mechanism lives entirely at the RTP-header level,
independent of which payload format rides inside.

## 5. Skew / buffer requirements — Receiver Classifications (§7, Table 1)

```
| Receiver Classification  | Use Case (example)              | Class SBR Streams | Class HBR Streams |
|---------------------------|----------------------------------|--------------------|--------------------|
| Class A: Low-Skew         | Intra-Facility Links             | PD <= 10ms         | PD <= 10ms         |
| Class B: Moderate-Skew    | Short-Haul Links                 | PD <= 50ms         | PD <= 50ms         |
| Class C: High-Skew        | Long-Haul or special circumstance| PD <= 450ms        | PD <= 150ms        |
| Class D: Ultra Low-Skew   | Physical Layer LAN Redundancy    | PD <= 150µsec      | PD <= 150µsec      |
```

**Version note**: Class D (Ultra Low-Skew, ≤150 µs) is **new in the 2019
edition** — the 2013 edition's equivalent table (2013 §7) has only Classes
A/B/C. An implementation targeting only the 2013 feature set would omit
Class D.

Timing-point definitions (§7, Figure 2 — `n ∈ 1…N` paths in 2019, generalized
from 2013's fixed `P1`/`P2`):

- `Pn` — instantaneous transmit-to-receive latency on path `n`, inclusive of network jitter.
- `PT` — latency from transmission to the final reconstructed output; the *latest* time a packet can arrive and still be used.
- `EA` — the *earliest* time a packet can arrive and still be used for seamless reconstruction.
- `MD = PT - EA` — maximum differential.
- `PD = max_{i,j in 1..N} |Pi - Pj|` — instantaneous path differential (2013: `PD = |P1 - P2|`, fixed two-path form).

Normative constraint: a compliant receiver of a given class **shall**
support seamless reconstruction as long as `PD` stays within that class's
table bound.

Path usability rule (§7, normative): "Only paths with a latency `Pi` that is
greater than `EA` and less than `PT` can be used to source packets ... As
long as there are at least two paths with latencies `Pi` and `Pj` that are
greater than `EA` and less than `PT` then seamless reconstruction is able to
recover from a packet loss in one of those two paths." (2013's equivalent
text was more explicit about the **degraded, single-qualifying-path** case:
*"When only one of P1 or P2 fall within the range [EA, PT] then successful
reconstruction is possible but only if there is no packet loss on the stream
which is arriving within the range. If neither P1 nor P2 fall within the
range ... then successful reconstruction of the payload is not possible."*
2019 does not restate this degraded case as explicitly, but it follows from
the same "at least two qualifying paths" rule.)

## 6. Startup buffer sizing — informative worked example (Annex A)

Not normative, but the only concrete numbers the standard gives for setting
`PT`/`EA` absent prior network knowledge, given here because they are the
basis for the ≈81,000-packet buffer-window figure used in §4.2 above:

- Class C, HBR streams: `PT` = 150 ms after the earlier of the streams; `EA` = `PT` − 300 ms.
- Class C, SBR streams: `PT` = 450 ms after the earlier of the streams; `EA` = `PT` − 900 ms.
- Worked numeric context (ST 2022-6, 1080p/60 HD-SDI @ 2.970 Gbit/s): ≈270 packets/ms, 1376 octets/packet (11,008 bits); a 300 ms Class-C/HBR window ⇒ ≈81,000 packets in flight, versus a 65,536-packet (`2^16`) sequence-number rollover period — the numeric justification for needing the RTP timestamp as well as the sequence number (§4.2 above).
- RTP timestamp rollover period at ST 2022-6's 27 MHz reference clock: `2^32 / 27,000,000 ≈ 159.07 s` (2019 figure; 2013's edition computed the same ratio as "40,722.6 seconds" — the two editions disagree on this derived number and neither shows its arithmetic; see [`README.md`](README.md) "could not establish", since `2^32/27e6 ≈ 159.07`, not 40,722.6, and this looks like an error in the 2013 edition that 2019 silently corrected, but this crate's docs cannot independently confirm SMPTE's intent behind the change).

## 7. What this means for `media_plane::byte_merge::MergePolicy::Hitless2022_7`

The `media-plane` crate's `byte_merge.rs` module (see its module doc)
deliberately has **no** `Hitless2022_7` variant yet, with a comment noting
that dedup "needs an RTP header parse and per-stream sequence-number
bookkeeping this layer does not have." Per §4.3 above, the minimum parse
that variant's producer needs is:

1. RFC 3550 RTP header parse (12 fixed bytes) to read `sequence number` (u16), `timestamp` (u32), and `SSRC` (u32).
2. Per-input-stream bookkeeping of the last-seen sequence number (for
   monotonicity/rollover tracking) and a bounded window of
   `(sequence_number, timestamp) -> Bytes` pending-candidate entries, sized
   per the receiver's target Class (Table 1, §5 above) and stream Class
   (SBR/HBR).
3. A selection policy (this crate's choice, **not** mandated by the
   standard per §4.2's "left to the implementer" note) for which candidate
   to emit and when to give up waiting on a slower path — e.g. first-arrival-wins,
   or prefer-primary-VSID-unless-late.

No ST 2022-6-specific (or other per-standard) payload parsing is required
for the dedup mechanism itself — only the generic RTP header.
