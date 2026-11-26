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
