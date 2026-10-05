# rtcp-packet 0.4.0

_Released 2026-10-05._

### Changed (breaking)
- **Behaviour change (#1123)**: SR/RR/SDES/BYE/APP parsing now validates and strips the `P`
  (padding) octets per RFC 3550 §6.4.1 instead of leaving them in the body, and **rejects** a
  padded packet whose padding count is zero or exceeds the body. Input that previously parsed
  (with padding bytes leaked into a BYE reason / APP `data`) now parses correctly or returns
  `Error::InvalidValue`. Re-serialization of these typed packets still emits `P=0`.

### Added
- `RtcpPacket` gained a new variant, `Unknown { packet_type, count, padding, payload }`,
  for any `PT` outside the RFC 3550 §6 core set (200-204) — e.g. RTPFB=205/
  PSFB=206 [RFC 4585] or XR=207 [RFC 3611]. Common-header framing only; the
  body is opaque and round-trips byte-identical, including the `P` bit and trailing padding
  octets (kept inside `payload`, flagged by `padding`). Serializing an `Unknown` whose
  `packet_type` is 200-204 is rejected (it could not parse back as `Unknown`). `RtcpPacket` is
  already `#[non_exhaustive]`, so this is additive, not breaking (#1071).

### Fixed
- `CompoundPacket`'s leading-packet rule now also accepts a leading
  `RtcpPacket::Unknown` (an unrecognized `PT`), per RFC 5506 §3.4.2/§4.1
  Reduced-Size RTCP; a leading SDES/BYE/APP is still rejected (#1071).
- `CompoundPacket::parse` rejected the **whole** datagram — including a
  perfectly valid leading SR/RR — the moment it walked into a packet whose
  `PT` was outside 200-204, since `RtcpPacket::parse` returned `Err` for any
  unrecognized type. A browser's WebRTC RTCP is mostly RFC 4585 PSFB/RTPFB
  feedback (NACK/PLI/REMB/transport-cc) or RFC 3611 XR, so this discarded a
  real peer's own SR/RR stats on nearly every datagram. Fixed by the new
  `RtcpPacket::Unknown` variant above; see `tests/real_browser_fixture.rs`
  for a real Chromium capture exercising exactly this shape (#1071).

---

Published from tag `rtcp-packet-v0.4.0`.
