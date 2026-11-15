# rtp-packet 0.8.0

_Released 2026-11-15._

Minor-class release (0.7 -> 0.8). The RTP/RTCP identities are typed instead of bare integers, so an SSRC can no longer be passed where a sequence number belongs, and a padded RTCP packet now re-serializes byte-identically with its padding and `P` bit intact. Two depacketizer/deserializer bugs are fixed.

Version: 0.7.0 -> 0.8.0.

## Breaking changes and migration

### Typed RTP/RTCP identities

`RtpPacket::sequence_number` is `SeqNo` (16-bit), `RtpPacket::ssrc`, `rtcp::ReportBlock::ssrc`, `rtcp::SenderReport::ssrc`, `rtcp::ReceiverReport::ssrc` and `rtcp::App::ssrc` are `Ssrc` (32-bit), and `rtcp::ReportBlock::ext_highest_seq` is `ExtendedSeqNo`. Each has `new`/`get` (plus `From`/`Into` for its raw integer). Wire bytes are unchanged. `SenderReport::new` and `ReceiverReport::new` take an `Ssrc`.

```rust
// before
let ssrc = packet.ssrc;
let sr = SenderReport::new(1, ntp_msw, ntp_lsw, rtp_timestamp, packet_count, octet_count)?;
// after
let ssrc = packet.ssrc.get();               // u32
let sr = SenderReport::new(Ssrc::new(1), ntp_msw, ntp_lsw, rtp_timestamp, packet_count, octet_count)?;
```

### Padded RTCP packets carry their padding

Each typed RTCP packet (`SenderReport`, `ReceiverReport`, `SourceDescription`, `Bye`, `App`) now has a public `padding: Option<RtcpPadding>` field for its trailing `P` octets, so a padded packet re-serializes byte-identically; the unpadded body region is validated separately from the padding octets.

```rust
// before
let sr = SenderReport { profile_extensions: vec![0; 4], ..sr };
// after
let sr = SenderReport { padding: Some(RtcpPadding::new(4)?), ..sr };   // count includes its own octet
```

### `rtcp::CommonHeader` is crate-private

It was an exported type with all-`pub` fields but no public constructor or consumer. Read a packet through the typed `SenderReport`/`ReceiverReport`/`SourceDescription`/`Bye`/`App` accessors instead.

```rust
// before
let header = rtcp::CommonHeader::parse(&bytes)?;
let kind = header.packet_type;
// after
let report = <rtcp::SenderReport as Parse>::parse(&bytes)?;   // the typed packet itself
```

## Fixes

- `Av3aDepacketizer` no longer emits the tail of an over-size frame as a bogus short frame: once a fragment overflows the cap, the depacketizer discards every later fragment up to the over-size frame's marker, then resumes on the next frame. Its bit-read truncation errors carry the real byte counts instead of a fabricated `need`/`have` of 0.
- RTCP SDES refuses to serialize an item whose type is the type-0 terminator or an `Unknown` carrying a defined code, and its parser errs on bytes after the declared chunk count instead of silently dropping them.

## Added

- `SeqNo::wrapping_next()` and `SeqNo::forward_distance(earlier)`: the 16-bit sequence-number wrap arithmetic.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).

---

Published from tag `rtp-packet-v0.8.0`.
