# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) with the
0.x caveat that the API is expected to grow.

## [Unreleased]

### Breaking

- `annexb::start_code_positions` is crate-private: nothing outside the crate
  needs the allocation. `annexb::find_start_code_prefix` stays the one public
  scan primitive, so external callers lose no capability.

### Changed

- The Annex B start-code scan (`annexb::find_start_code_prefix`) finds each `01`
  byte with `memchr` again, so a scan over a multi-megabyte access unit stays
  vectorised. The crate now depends on `memchr` (`default-features = false`).
- Feature `vvc` added (default on) for the H.266 decoders.

### Added

- Initial contents: the exp-Golomb/RBSP bit reader (`bitreader`), RBSP
  emulation-prevention handling (`rbsp`), `NalCodec`, `AvcNalUnitType`,
  `HevcNalUnitType`, key-frame/IRAP and random-access-point classification, the
  SEI message walk and ATSC A/53 caption extraction (`nal`), Annex B and
  length-prefixed NAL iteration and conversion (`annexb`), and the incremental
  `AccessUnitSplitter` (`au`).
- `AvcSequenceParameterSet`/`AvcPictureParameterSet` typed wrappers, with
  conversions to `isobmff` decoder-configuration newtypes behind the `isobmff`
  feature.
- The access-unit splitter's open-NAL bound is the caller's
  (`AccessUnitSplitter::with_max_open_nal_bytes`); the default is unlimited.
- `split_annexb` and `annexb_to_avcc` examples over a real committed Annex B
  fixture.
- The sequence parameter set decoders (`sps`), moved from `transmux`:
  `decode_avc_sps`, `decode_hevc_sps`, `decode_vvc_sps` (new `vvc` feature) with
  their profile/tier/level, dimension, chroma, bit-depth and VUI timing facts;
  `decode_avc_vui_timing`, `decode_hevc_vui_timing`, `decode_hevc_vps_timing`,
  `decode_avc_reorder_depth`, `decode_avc_slice_sps` and
  `decode_hevc_interlace_sps`; `decode_ptl` and `is_high_profile`; and the
  display-facts entry points `decode_avc_display_facts` /
  `decode_hevc_display_facts` returning the new `SpsDisplayFacts`.
- The coded picture facts (`facts`), moved from `transmux`'s display-geometry
  module: `AspectRatioIdc`, `SampleAspectRatio`, `PictureRect`, `VideoFormat`,
  `ColourPrimaries`, `TransferCharacteristics`, `MatrixCoefficients`,
  `ColourDescription`, `VideoSignalType`, `ActiveFormatDescription`, `BarData`
  and `EXTENDED_SAR_IDC`, with private fields and accessors throughout.
- The per-codec RFC 6381 helpers (`rfc6381`): `rfc6381_avc1`,
  `rfc6381_hvc1`, `rfc6381_vvc1` and `rfc6381_mp4a` (the last ungated, since it
  describes an audio configuration).
- A `decode_capture_sps` example over a real transport-stream capture, and a
  real-fixture test that walks `fixtures/ts/h264-aac.ts` to its first IDR's SPS.
- A `broadcast_bitstream_sps` fuzz target over every SPS decoder.
