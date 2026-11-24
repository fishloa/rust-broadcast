# Changelog

All notable changes to `isobmff` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Added

- The ISO/IEC 14496-12:2026 box layer, extracted from `transmux`: the box /
  FullBox headers (64-bit `largesize`, `uuid` usertype), the `box_iter`
  walker, `ftyp`/`styp`/`mdat`, the sample-table and timing boxes, the
  visual-extension boxes, the sample-group boxes, the movie-fragment boxes,
  the CENC boxes (ISO/IEC 23001-7 §12), and the §8.8.7/§8.8.8 fragment
  sample-offset resolver.
- `Error`, `Result` and the box-level `TrackId` (the 14496-12 `track_ID`,
  which prints as a bare integer and serialises transparently).
- `ser_timescale` (feature `serde`): serialise a `Timescale` as the plain
  ticks-per-second integer the wire box carries.

### Breaking

- New crate. Nothing was released before this entry, so there is no previous
  path to break.
