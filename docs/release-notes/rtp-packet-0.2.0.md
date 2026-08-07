# rtp-packet 0.2.0

**Release date:** 2026-07-11

Adds RFC 8285 one-byte/two-byte multiplexed header-extension decoding as an
opt-in feature, so callers that need to inspect RTP header extensions used
by WebRTC, ST 2110, and other profiles can do so without a separate parser.

## What's new

- New `rfc8285` feature (additive, off by default): decodes RFC 8285
  one-byte and two-byte multiplexed extension elements from the RFC 3550
  `HeaderExtension`.
- `parse_extensions` dispatches on `profile_id` (`0xBEDE` for one-byte,
  `0x1000`-range for two-byte).
- `OneByteElement`/`OneByteElements` -- section 4.2: local ID 1-14, 1-16 byte
  payloads, correct halt at reserved ID-15 and malformed ID-0 cases.
- `TwoByteElement`/`TwoByteElements` -- section 4.3: local ID 1-255, 0-255
  byte payloads.
- Both container types implement byte-identical Parse/Serialize round trips
  with canonicalized trailing padding.
- `rfc8285_extensions` runnable example.
- New fuzz target `rtp_packet` covering the base packet and both RFC 8285
  forms.
- Spec-derived vectors instantiating both RFC 8285 worked examples
  byte-for-byte.

## Migration

No breaking changes. The `rfc8285` feature is additive and off by default.
