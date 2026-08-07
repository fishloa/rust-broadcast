# rtcp-packet 0.1.0

**Release date:** 2026-07-11

Initial release. `rtcp-packet` is a spec-complete RFC 3550 section 6 RTCP
control-packet codec, extracted from `transmux::rtcp` so it can be reused
independently. It covers all five standard RTCP packet types and the
compound-packet envelope.

## What's new

- `SenderReport` (SR, PT 200) / `ReceiverReport` (RR, PT 201) with the
  shared 24-byte `ReportBlock` (signed `cumulative_lost` correctly
  sign-extended on parse).
- `SourceDescription` / `SdesChunk` / `SdesItem` / `SdesItemType` (SDES,
  PT 202): typed CNAME/NAME/EMAIL/PHONE/LOC/TOOL/NOTE/PRIV item types.
- `Bye` (PT 203): SSRC/CSRC list + optional UTF-8 reason text.
- `App` (PT 204): subtype, SSRC, 4-byte ASCII name, application data.
- `RtcpPacket`/`RtcpPacketType` dispatch enum with `name()`/`Display`
  labels.
- `CompoundPacket`: enforces the "first packet must be SR or RR" rule on
  both parse and construction.
- Spec-derived wire vectors (one per packet type + compound) with
  byte-identical round-trip tests.
- Two runnable examples: `build_sender_report` and `parse_compound_packet`.
- `no_std` + `alloc`; builds on bare-metal targets.
- `serde` support behind the `serde` feature.

## Migration

No breaking changes. Initial release.
