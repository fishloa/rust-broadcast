# transmux 0.10.0

Released 2026-07-03.

### Changed

Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation.

### Added

- **HEVC SPS VUI timing fields** (#546): `HevcSpsInfo` gains `num_units_in_tick`,
  `time_scale`, and `fps` fields — mirroring AVC equivalents from #523.
  `decode_hevc_sps` now walks the full HEVC SPS syntax (ITU-T H.265 §7.3.2.2.1)
  to parse `vui_parameters()` (§E.2.1).
- **HLS Sample-AES + full-segment AES-128 encryption** (#479): new `sample_aes`
  module (feature `sample-aes`). AES-128 full-segment, H.264/AAC/AC-3/E-AC-3
  SAMPLE-AES, and `EXT-X-KEY` rendering including FairPlay.
- **Multi-DRM `pssh` init-data generation** (#480): `drm` module building
  PlayReady (WRMHEADER v4.2.0.0 + PRO), Widevine (hand-encoded protobuf), and
  FairPlay PSSH boxes. Includes CENC-UUID ↔ PlayReady LE-GUID byte-swap.
- **KLV timed metadata + KLV-over-RTP** (#478): SMPTE ST 336 / MISB ST 0601 /
  RFC 6597 — BER length/OID codecs, `KlvItem`, `UasLocalSet` with CRC-16/CCITT
  checksum, and RTP de/packetization.
