# transmux 0.22.0

Released 2026-08-02.

### Changed

- HLS playlist syntax extracted to `broadcast-hls` (issue #878). `transmux`'s
  HLS/LL-HLS segmenters stayed put and depend on the new crate.
- Requires `broadcast-hls` 0.1.
