# transmux 0.26.0

_Released 2026-10-13._

Epoch bump forced by `rtcp-packet` 0.5 (`RtpStreamDepacketiser::push_sender_report` takes the new `SenderReport`). Also: new `StreamingTsMux` (incremental TS muxer with persistent continuity counters, one timestamp origin and PSI schedule), panicking `to_bytes()` paths replaced by errors (Matroska CodecPrivate, Smooth moof), `ps_demux`/`ts_mux` wrap arithmetic on `clock33`, and a `clock33` tripwire test.


### Fixed

- Smooth fragment build and Matroska `CodecPrivate` (avcC/hvcC/av1C) propagate serialize errors instead of panicking via `to_bytes`.

### Changed

- PS demux and TS mux 33-bit wrap arithmetic now delegates to `broadcast_common::clock33` (no behaviour change); added `tests/clock33_guard.rs` lexical tripwire and a PS wrap-straddle test.
### Added
- `StreamingTsMux` (#1024): an incremental `Package` counterpart of `TsMux` for a live feed fed one chunk (even one sample) at a time. It keeps continuity counters, one shared time origin (absolute `Sample::dts`/`pts`) and the PAT/PMT repetition schedule across `package` calls, so the concatenated output is one continuous transport stream; per PID it is byte-identical to a whole-`Media` `TsMux` mux, only the PAT/PMT placement differs. Additive.

---

Published from tag `transmux-v0.26.0`.
