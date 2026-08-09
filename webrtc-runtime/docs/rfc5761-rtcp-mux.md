# RFC 5761 — Multiplexing RTP Data and Control Packets on a Single Port

Source: `https://www.rfc-editor.org/rfc/rfc5761.txt`, April 2010. Updates
RFC 3550 §11 / RFC 3551. Section numbers below are RFC 5761's own.

Implemented by `handle_srtp_datagram` in
`webrtc-runtime/src/media/transport.rs`, which is the point where a
datagram already known (via RFC 5764 §5.1.2's demux, see
`rfc5764-dtls-srtp.md`) to be SRTP/SRTCP is further split into RTP vs.
RTCP.

## 1. The demux rule (§4)

When RTP and RTCP packets share a port, byte offset 1 of the packet is:

- for RTP: the RTP marker bit (M, 1 bit) + payload type (PT, 7 bits) —
  together an 8-bit field;
- for RTCP: the RTCP packet type (PT, 8 bits), in the same wire position.

This lets a receiver distinguish the two **only** if:

1. the RTP payload types in use are distinct from the RTCP packet types
   in use, and
2. for every RTP payload type PT in use, `PT + 128` is also distinct from
   the RTCP packet types in use (this is the case where the RTP marker bit
   is set — `M=1, PT` reads as the 8-bit value `128 + PT` at that byte
   offset, so an RTP PT and an RTCP PT that are `128` apart alias into the
   same byte value once M is set).

### 1.1 Known RTP/RTCP payload-type conflicts (§4, informative list)

| RTP payload type(s) | Conflicts with |
|---|---|
| 64–65 | Obsolete RTCP FIR/NACK (H.261 video, obsoleted format) |
| 72–76 | RTCP SR, RR, SDES, BYE, APP (RFC 3550 core) |
| 77–78 | RTCP RTPFB, PSFB (RTP/AVPF profile) |
| 79 | RTCP XR (Extended Report) |
| 80 | RSI (Receiver Summary Information, unicast-feedback multicast ext.) |

### 1.2 The resulting allocation rule

To keep future RTCP assignments from colliding with RTP payload types
already in the field: **new RTCP packet types SHOULD be assigned in
209–223 first, then 194–199** — this confines *all* the conflicts to RTP
payload types 64–95 (that range is what `[64,95] + 128 = [192,223]`
covers). RTCP types in `1–191` and `224–254` SHOULD only be used once
those ranges are exhausted.

Given that, this document's own recommendation for choosing RTP payload
types when rtcp-mux is in play: follow RTP/AVP's normal guidance, but
**MUST NOT use payload types 64–95**. Dynamic payload types SHOULD come
from 96–127; values below 64 MAY be used if 96–127 is insufficient, in
which case prefer PT numbers RFC 3551 didn't statically assign.

**Note on the effective RTCP range**: the *rule* (§4) constrains RTP PT to
avoid 64–95; the *practical* RTCP packet-type footprint that rule is built
around is `[192,223]` (`64..95 + 128`), with `[224,254]` reserved as a
last resort and `[1,191]` likewise last-resort on the RTCP side. So
"RTCP occupies 192–223" is an accurate description of *current and
near-future* assignments, but not a hard ceiling the base spec imposes —
see the discrepancy note below.

## 2. When multiplexing is appropriate (§5.1)

- Unicast only in this section (ASM/SSM multicast get their own rules in
  §5.2/§5.3, not relevant to this crate's WHIP/WHEP unicast media path).
- Signalled via SDP `a=rtcp-mux` at the media level (§5.1.1): offerer
  includes it to request muxing; answerer echoes it to accept. If the
  answer omits it, RTP and RTCP MUST go back to separate ports (whatever
  `a=rtcp:` / the port-pair convention signals).
- With ICE (§5.1.3): offering agent lists **both** an RTP candidate and an
  `a=rtcp:` fallback if `a=rtcp-mux` is offered, so a non-supporting
  answerer still gets a working RTCP path. An accepting answerer's
  candidates cover RTP only.

None of the SDP/ICE-candidate signalling in §5.1.1/§5.1.3 is this crate's
job — `webrtc-runtime`'s `whip`/`whep` modules move the SDP bytes, and the
caller is the one deciding whether to offer `a=rtcp-mux`, per the crate
README. `MediaTransport` itself simply receives whatever the caller
negotiated and demuxes accordingly (see below); it has no `a=rtcp-mux`
awareness of its own to get right or wrong.

## 3. Cross-check against `transport.rs`

```rust
/// RFC 5761 §4: RTCP packet types occupy `[192, 223]` once RTP and RTCP are
/// multiplexed on one port (`a=rtcp-mux`) ...
const RTCP_MUX_TYPE_MIN: u8 = 192;
const RTCP_MUX_TYPE_MAX: u8 = 223;
```

used as:

```rust
let is_rtcp = data.get(1)
    .is_some_and(|&pt| (RTCP_MUX_TYPE_MIN..=RTCP_MUX_TYPE_MAX).contains(&pt));
```

This picks off byte offset 1 exactly as §4 describes, and `[192,223]`
matches the range the spec's own allocation guidance (§4, "SHOULD be made
after the current assignments in the range 209-223, then ... 194-199")
concentrates all current and near-future RTCP packet types into.

**Discrepancy — fixed (issue #948 item 3)**: RFC 5761 §4 does not itself
put a hard ceiling at 223 — it says future assignments *SHOULD* land in
`[209,223]` then `[194,199]`, and that `[224,254]` (and `[1,191]`) SHOULD
only be used "when other values have been exhausted." IANA's live RTCP
packet-type registry has, since this RFC, continued to allocate inside
`192-223` (SR=200, RR=201, SDES=202, BYE=203, APP=204, RTPFB=205, PSFB=206,
XR=207, AVB=208, RTPS=209, ...), so `[192,223]` alone had **not yet been
wrong for any packet type actually in use** — but it was a narrower range
than the letter of the spec allows: a legitimately-registered future RTCP
packet type in `[224,254]` would misclassify as RTP by this code's demux
(`is_rtcp` false ⇒ treated as SRTP, decrypted via `decrypt_rtp`/parsed via
`rtp_packet::RtpPacket`, which would then fail to parse or parse into
garbage).

`RTCP_MUX_TYPE_MAX` in `transport.rs` is now `254`, folding `[224,254]`
into the RTCP band. `[1,191]` is deliberately **not** folded in — see
`RTCP_MUX_TYPE_MIN`/`RTCP_MUX_TYPE_MAX`'s own doc comment in `transport.rs`
for why: it overlaps essentially all unmarked (`M=0`) RTP traffic (`byte1
== PT`, `PT` in `0..=127`), so treating it as RTCP would be a live,
guaranteed misclassification, not a currently-dormant one.

**Residual risk worth flagging explicitly, not yet resolved**: widening to
`[224,254]` reintroduces the *same class* of aliasing this RFC already
accepts for `[192,223]` — but against a wider, more commonly-used RTP
range. RFC 5761 §4 itself recommends dynamic RTP payload types come from
`96..=127` (only `64..=95` is a hard MUST-NOT), and `96..=127` with the
marker bit set (`M=1`, extremely common — e.g. every video frame boundary)
aliases to byte1 values `224..=255`. So a media session using dynamic PT
`>= 96` with `a=rtcp-mux` negotiated now has a live (if narrow — one byte
value per PT) ambiguity window that did not exist before this widening,
traded against fixing a currently-unregistered RTCP gap. This crate has no
SDP/negotiated-payload-type awareness (by design, see the module doc), so
it cannot resolve this itself; a caller that both negotiates dynamic PT
`>= 96` *and* anticipates RTCP packet types being allocated in `[224,254]`
would need to pick payload types more carefully, or this crate would need
a way to accept the negotiated PT set to disambiguate. Flagged here rather
than silently traded off.

## 4. Bandwidth / QoS (§6) and security (§7) — not applicable here

§6 discusses RTCP bandwidth-share accounting when RTP and RTCP compete for
one port's allotment (already covered concretely for the SRTCP-added-field
case in RFC 3711 §3.4, see `rfc3711-srtp.md`); §7 notes muxing makes it
marginally easier for an attacker who can inject RTP to also inject RTCP
(mitigated by SRTP's own authentication, not by anything RTCP-mux-specific).
Neither imposes a wire-format or behavioural rule beyond §4's demux and
§5's negotiation — nothing here to check `MediaTransport` against beyond
what's already covered above.
