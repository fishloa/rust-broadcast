# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- New crate: `media-plane`, the media-plane integration layer (plan step 3a-i).
- `ByteStage`: the pre-demux byte-to-byte stage contract, defined as
  `Stage<In<'a> = &'a [u8], Out = Bytes>` rather than a second drive trait
  (per the 2026-07-27 revision of `docs/superpowers/specs/2026-07-26-media-plane-architecture.md`
  §1.1). Validated to compile at MSRV 1.86.
- `no_std` + `alloc` crate skeleton with a `std` feature, mirroring `transmux`.
