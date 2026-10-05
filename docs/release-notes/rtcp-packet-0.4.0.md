# rtcp-packet 0.4.0

_Released 2026-10-05._

Breaking (0.x minor) for behaviour, not for API shape. Padded RTCP packets are now parsed correctly, and a datagram containing a packet type outside 200-204 is no longer rejected as a whole. Source code that compiles against 0.3.1 still compiles, because the new `RtcpPacket::Unknown` variant lands on an enum that was already `#[non_exhaustive]`. Who must act: anyone who feeds this parser input that can carry the RTCP `P` bit, anyone who relied on `RtcpPacket::parse` or `CompoundPacket::parse` returning an error for an unrecognised packet type (RTPFB 205, PSFB 206, XR 207), and anyone whose code matches `RtcpPacket` and has a catch-all arm that assumed the five known variants. The previous published version is 0.3.1.

## Behaviour changes

### P-bit padding is validated and stripped (#1123)

SR, RR, SDES, BYE and APP parsing now honour RFC 3550 §6.4.1: when the `P` bit is set, the last octet of the packet is a count of padding octets (including itself), and those octets are removed from the body before the packet is decoded. Before, the padding stayed in the body: a padded BYE had its first padding byte read as the reason-length octet, and a padded APP had its padding appended to `data`.

- A padded packet now parses to the same value as the unpadded equivalent.
- A padded packet whose padding count is zero, or larger than the packet body, is rejected with `Error::InvalidValue` (field `rtcp_padding_count`). Input like that used to parse with garbage in the body.
- Serialization of these typed packets still emits `P=0`. A padded input therefore does **not** round-trip byte-identically through the typed packets; the padding is dropped. If you need byte-exact preservation of a packet, keep it as `Unknown` (below) or keep the original bytes.

```rust
// 0.3.1: a padded BYE parsed successfully with padding leaked into `reason`
// 0.4.0: padding is stripped; a bad count is an error
match RtcpPacket::parse(&bytes) {
    Ok(RtcpPacket::Bye(bye)) => use_reason(bye),
    Err(Error::InvalidValue { field: "rtcp_padding_count", .. }) => drop_packet(),
    _ => {}
}
```

### Unrecognised packet types parse instead of failing (#1071)

`RtcpPacket::parse` previously returned `Err` for any `PT` outside 200-204, and `CompoundPacket::parse` propagated that, so one such packet discarded the whole datagram, including a valid leading SR or RR. A browser's WebRTC RTCP is mostly RFC 4585 RTPFB/PSFB feedback (NACK, PLI, REMB, transport-cc) or RFC 3611 XR, so this threw away the peer's own SR/RR statistics on nearly every datagram. The fix is covered by `tests/real_browser_fixture.rs`, a real Chromium capture.

## API addition

`RtcpPacket::Unknown { packet_type: u8, count: u8, padding: bool, payload: Vec<u8> }` carries any packet type outside 200-204. Only the common header is interpreted; `payload` is the opaque body, and when `padding` is true the trailing padding octets are kept inside `payload`. It round-trips byte-identically, including the `P` bit. Serializing an `Unknown` whose `packet_type` is 200-204 is rejected, since it could not parse back as `Unknown`.

```rust
// before: a catch-all arm was never reached for RTPFB/PSFB/XR (they were errors)
match pkt {
    RtcpPacket::SenderReport(sr) => { /* ... */ }
    RtcpPacket::Unknown { packet_type, payload, .. } => forward_opaque(packet_type, &payload),
    _ => {}
}
```

`CompoundPacket`'s leading-packet rule now also accepts a leading `RtcpPacket::Unknown`, per RFC 5506 §3.4.2 and §4.1 (Reduced-Size RTCP). A leading SDES, BYE or APP is still rejected.

## Dependencies

Verified from the `Cargo.toml` diff against `rtcp-packet-v0.3.1`: only `broadcast-common = { version = "9.3" -> "9.4", default-features = false }`. No feature changes.

## Read together with

[rist-runtime-0.2.0.md](rist-runtime-0.2.0.md), which builds its compound packets over this crate and moved to this 0.4 epoch in the same wave. A crate depending on both must take both.

---

Published from tag `rtcp-packet-v0.4.0`.
