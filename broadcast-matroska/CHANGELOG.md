# Changelog

All notable changes to this crate are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this crate uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Initial crate: EBML (RFC 8794) element framing, the scalar payload codecs,
  the EBML header, `Void` and `CRC-32`.
- The full Matroska (RFC 9559) element table with typed master and leaf
  elements, the `SimpleBlock`/`Block` header with none, Xiph, EBML and
  fixed-size lacing, and typed field accessors.
- Byte-identical round trips for every committed Matroska/WebM fixture,
  checked element by element; the element-id table is pinned to the private
  transcription at runtime.
- Hostile-input caps (`Limits`) for element depth, element size and child
  count.
