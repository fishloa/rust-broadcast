# transmux 0.25.6

_Released 2026-10-13._

### Fixed

- Damaged TS input no longer makes `TsDemux`/`StreamingTsDemux` hand a consumer an unserializable codec config (#1171). A garbled access unit could yield an H.264 "SPS" of 85 704 bytes (a plausible NAL header followed by garbage running to the end of the access unit), which became the track's `avcC` and raised `TrackAdded`/`TrackUpdated`; `config.to_bytes()` then panicked with `FieldOverflow` (`sps NALU length` > 65 535, the `u16` length field of ISO/IEC 14496-15 §5.3.3.1.2 / §8.3.3.1.2). H.264 SPS/PPS and HEVC VPS/SPS/PPS are now validated before use: non-empty, at most 65 535 bytes, correct `nal_unit_type` and header bits, RBSP stop bit present, and an SPS must decode to non-zero geometry. An invalid set is ignored (never recorded as current, no `TrackAdded`/`TrackUpdated`) and counted as one `InputDegraded` event. The same validation guards the Program Stream demuxer's in-band SPS/PPS.
- `Ec3SpecificBox::from_syncframes` now returns `Error::InvalidValue` when the independent substreams are not numbered contiguously from 0 (previously it built a `dec3` that `serialize_into` rejects), and the MPEG-H TS probe ignores an `mpegh3daConfig()` longer than the `mhaC` `u16` length field, so every config the TS demuxer emits serializes.

### Added

- `InputDegradation::InvalidParameterSet` (the enum is `#[non_exhaustive]`): an H.264/HEVC parameter-set NAL was rejected as unusable for a decoder configuration and ignored (#1171).

---

Published from tag `transmux-v0.25.6`.
