# rtp-packet 0.8.0

_Released 2026-11-15._

### Changed
- BREAKING: the RTP/RTCP identities are typed instead of bare integers: `RtpPacket::sequence_number` is `SeqNo` (16-bit), `RtpPacket::ssrc`, `rtcp::ReportBlock::ssrc`, `rtcp::SenderReport::ssrc`, `rtcp::ReceiverReport::ssrc` and `rtcp::App::ssrc` are `Ssrc` (32-bit), and `rtcp::ReportBlock::ext_highest_seq` is `ExtendedSeqNo`. Each has `new`/`get` (plus `From`/`Into` for its raw integer), so an SSRC can no longer be passed where a sequence number belongs. `SenderReport::new` and `ReceiverReport::new` take an `Ssrc`. Wire bytes are unchanged. Migration: read `.get()` where a raw integer is needed, wrap with `Ssrc::new`/`SeqNo::new` when building a packet (`pkt.ssrc` -> `pkt.ssrc.get()`, `ssrc: 1` -> `ssrc: Ssrc::new(1)`) (#2768).
- `SeqNo` carries the 16-bit wrap arithmetic: `wrapping_next()` and `forward_distance(earlier)`.
- The `panic_idioms` workspace guard now covers this crate (already clear of panicking idioms in non-test code) (#2773).
### Added
- BREAKING: each typed RTCP packet (`SenderReport`, `ReceiverReport`, `SourceDescription`, `Bye`, `App`) now carries its trailing `P` (padding) octets as a public `padding: Option<RtcpPadding>` field, so a padded packet re-serializes byte-identically with its padding and `P` bit intact (audit19b #2742). The unpadded body region is validated separately from the padding octets.

### Removed
- BREAKING: `rtcp::CommonHeader` is crate-private (it was an exported type with all-`pub` fields but no public constructor or consumer). Migration: none — the typed `SenderReport`/`ReceiverReport`/`SourceDescription`/`Bye`/`App` accessors are the public read path (#2771).

### Fixed
- AV3A `Av3aDepacketizer` no longer emits the tail of an over-size frame as a bogus short frame: once a fragment overflows the cap, the depacketizer discards every later fragment up to the over-size frame's marker, then resumes on the next frame (audit19b #2750). Its bit-read truncation errors now carry the real byte counts, not fabricated `need/have` of 0.
- RTCP SDES refuses to serialize an item whose type is the type-0 terminator or an `Unknown` carrying a defined code (audit19b #2751), and its parser now errs on bytes after the declared chunk count instead of silently dropping them.

---

Published from tag `rtp-packet-v0.8.0`.
