# transmux 0.33.0

_Released 2026-11-22._

Minor for a 0.x crate, and breaking: two dead public items are removed, one error class changes, and the crate builds against the `broadcast-common` 14 / jiff 1.0 epoch. The headline feature is the opt-in `motion-imagery-metadata` feature: typed MISB ST 0601 Local Set values from the new, separately released `motion-imagery-metadata` crate, built on Matt Busel's `misb` crate. The rest of the release is the audit-23 test-hardening pass over the container and RTP paths.

## Breaking changes

- `HEVCDecoderConfigurationRecord::is_valid_length_size` is removed. It had no caller (the parser validates `lengthSizeMinusOne` against `VALID_LENGTH_SIZES`; the VVC record keeps its own, used copy) and its doc described an unrelated reserved-bit rule (#3781).
- `TrackFragmentBox::into_bytes` is removed (unused); use `Serialize::try_to_bytes` (#3698).
- `BitReader::read_bits` reports an over-wide field (`n > 64`) as `Error::InvalidValue`, the class `VvcBitReader::bits` and `bit_error` already use, instead of `Error::BufferTooShort`. A caller matching the short-buffer variant for that input sees the new class (#3998).
- Builds against new caret epochs of `broadcast-common` 14, `broadcast-dash` 0.3, `broadcast-hls` 0.9, `broadcast-smooth` 0.3, `cc-data` 0.11, `container-probe` 0.6, `dvb-si` 16, `mp4-emsg` 0.9, `mpeg-pes` 0.10, `mpeg-ps` 0.10, `mpeg-ts` 0.10, `rtmp-runtime` 0.12, `rtp-packet` 0.9 and `scte35-splice` 8. A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single copy of each at the jiff 1.0 epoch.

## Added

- Optional `motion-imagery-metadata` feature: `LocalSetItem::value_typed()` returns `motion_imagery_metadata::St0601Value`, the item decoded and scaled per the MISB ST 0601 tag table (times as `jiff::Timestamp`, nested ST 0102 Security and ST 0903 VMTI sets typed, other nested sets and packs kept with their standard and raw bytes). Enable with `transmux = { version = "0.33", features = ["motion-imagery-metadata"] }`. The decoding lives in [`motion-imagery-metadata`](https://github.com/fishloa/motion-imagery-metadata) 0.1.0, which wraps the [`misb`](https://crates.io/crates/misb) crate by Matt Busel (#3955); transmux itself keeps only the KLV framing.

## Fixed

- The BER-OID documentation no longer claims every ST 0601 tag fits one byte: tags 128..=143 take two-byte keys (contributed by Matt Busel, #3955).
- The `vvcC` record reader no longer swallows an error from its byte skip or advances its cursor unchecked over `ptl_reserved_zero_bit` padding: the skip's error is propagated, a length that would overflow the byte cursor is a typed `BufferTooShort` instead of a panic, and alignment reads its padding bits (#3999).
- 192-byte M2TS source packets demux: the resynchroniser locks at the 192-byte stride and strips the 4-byte `TP_extra_header`, so `TsDemux` yields the tracks an M2TS file really carries instead of nothing (#3988).

## Changed

- Internal: error types derive `thiserror::Error` (messages unchanged) (#3974, #4020).
- Internal: the public `bitreader::BitReader` (Exp-Golomb, RBSP unescape) is a thin wrapper that resumes `broadcast_common::bits::BitReader` per read; public API, results and errors are unchanged (#3961).
- Hex and base64 go through `broadcast_common::{hex, base64}`; output is unchanged (#3957).
- README lists the optional `motion-imagery-metadata` feature (#4015).

## Tests

The audit-23 suites pin the `av1C` flag/marker/delay bits, the buffered-NAL and access-unit caps at their exact boundaries, HEVC/VVC access-unit splitting (AUD-less, prefix/suffix, reserved NAL types), `finish()` on junk-only input, HE-AAC override targeting, aggregated-AAC duration arithmetic and reorder windows, the SRT/RTCP wallclock alignment and skew refusals, SSRC changes and wild forward jumps, and the 4 MiB access-unit buffer. Each has a test that fails if the operator or constant moves.

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #3990 (ST 0601 transcription and tests), #3997 (transcription citations), #3998 (over-wide field error class), #3999 (`vvcC` swallowed error), #4015 (README), #4018 (duplicate headings), #4020 (`thiserror`). #4041 records the nested/pack raw faces as a documented gap for a later release.

---

Published from tag `transmux-v0.33.0`.
