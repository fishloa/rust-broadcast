# broadcast-bitstream

Codec bitstream parsers and stream-format conversion for `rust-broadcast`.

**This crate reads codec facts. It never decodes or encodes media.** It is the
layer below the container and muxing crates: a project that only wants to split
an Annex B stream, classify NAL units, or read an exp-Golomb field can depend on
this crate alone instead of taking the whole muxing hub.

What it holds today:

- **The bit reader** ([`bitreader`]) — the owning RBSP reader with `ue(v)` and
  `se(v)` Exp-Golomb decoding, on top of `broadcast_core::bits`.
- **RBSP handling** ([`rbsp`]) — emulation-prevention byte removal and
  insertion (ITU-T H.264 §7.4.1 / H.265 §7.4.2).
- **NAL units** ([`nal`]) — `NalCodec`, the H.264/H.265 NAL unit-type enums, key
  frame and random-access classification, the SEI message walk, and the ATSC
  A/53 caption extractor.
- **Stream formats** ([`annexb`]) — Annex B start-code scanning and the conversion
  to and from the fixed 4-byte length-prefixed form.
- **Access units** ([`au`]) — the incremental Annex B access-unit splitter.
- **Sequence parameter sets** ([`sps`]) — the H.264, H.265 and H.266 SPS
  decoders: profile/tier/level, coded dimensions, chroma and bit depth, VUI
  timing (H.265 also from the VPS) and the display facts a player needs.
- **Coded picture facts** ([`facts`]) — the sample aspect ratio tables, the
  cropping rectangle, the colour description and video signal type, and the
  active-format / bar-data codes as the specifications state them.
- **RFC 6381 codec strings** ([`rfc6381`]) — `avc1.PPCCLL`, `hvc1.…`, `vvc1.…`
  and `mp4a.40.<AOT>` built from the fields the caller already decoded.

Slice headers, AAC, AC-3, DTS, MPEG-2 and MPEG audio arrive as features on top
of this, one family at a time.

## The recorded exception

This crate is **facts-only in this version**. `docs/CRATE-ACCEPTANCE.md` requires
a symmetric serializer and a byte-exact round trip for every parser; here that is
waived for the types that do not already carry one, in writing, in
`CRATE-ACCEPTANCE.md`. Real-fixture parse tests, fuzz targets and the spec-drift
tests are **not** waived.

The roadmap removes the exception type by type: lossless parse plus a writer for
H.264 SPS/PPS, HEVC VPS/SPS/PPS, AC-3/E-AC-3, DTS core, AV1 sequence headers and
MPEG-2 sequence headers, each landing as an additive release.

## Features

- `std` (default) — forwards `std` to the dependencies. The crate is `#![no_std]`
  plus `alloc` either way.
- `h264`, `hevc`, `vvc` (all default) — the per-codec NAL unit-type enums and
  sequence parameter set decoders.
- `serde` — `Serialize` derives on the spec/field types.
- `isobmff` — conversions between the crate's typed parameter-set wrappers and
  `isobmff`'s decoder-configuration-record newtypes, for a caller that builds an
  `avcC`/`hvcC`.

It depends on `broadcast-core`, `thiserror` and `memchr` only. (`mpeg-ts` and
`mpeg-pes` are dev-dependencies, used by one example and one test to reach a
real capture's elementary stream.)

## Runnable examples

Run with `cargo run -p broadcast-bitstream --example <name>`:

- `split_annexb` and `annexb_to_avcc` take a real Annex B elementary stream,
  e.g. `fixtures/candidates/h264.annexb`.
- `decode_capture_sps` takes a transport-stream capture,
  e.g. `fixtures/ts/h264-aac.ts`, and prints the resolution, profile, level and
  RFC 6381 codec string of the SPS in force at its first IDR access unit.
