# rtp-packet 0.1.0

**Release date:** 2026-07-11

Initial release. `rtp-packet` parses and serializes the RFC 3550 RTP fixed
header -- the framing every real-time media stream (video, audio, ancillary
data) rides on. Zero-copy, `no_std`, and independently versioned so it can
be used without pulling in the full `transmux` container-muxing hub.

## What's new

- `RtpPacket` -- parser/serializer for the RFC 3550 section 5.1 RTP header: version,
  padding, CSRC list (0-15 entries), marker, payload type, sequence number,
  timestamp, SSRC, optional header extension, and payload.
- `HeaderExtension` -- RFC 3550 section 5.3.1 generic header extension with
  profile-specific identifier and opaque data.
- Real-fixture test over a 324-byte RTP packet captured from this
  workspace's own `transmux::RtpPacketizer`.
- Spec-derived vectors for padding/CSRC/header-extension edge cases.
- Two runnable examples: `build_packet` and `parse_packet`.
- `no_std` + `alloc`; builds on bare-metal targets.
- `serde` support behind the `serde` feature.

## Migration

No breaking changes. Initial release.
