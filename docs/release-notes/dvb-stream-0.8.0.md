
# dvb-stream 0.8.0

_Released 2026-11-07._

Breaking-class release (0.7 -> 0.8): three unused public items are removed and `SectionPump`/`T2miPumpDriver` are merged into one internal `PacketPump` with no API change; the `dvb-si`, `dvb-t2mi` and `mpeg-ts` epochs move.

Version: 0.7.0 -> 0.8.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- Unused `resync::aligned_packets`; the `resync` module doc no longer claims to wrap `mpeg_ts::resync::TsResync`.
- Unused `SectionStream::with_builder` and `MulticastConfig::with_interface`.

## Other changes

- `SectionPump` and `T2miPumpDriver` replaced by one `PacketPump` over a `PacketSink` trait; no public API change.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11
- `dvb-si` 12 -> 13
- `dvb-t2mi` 12 -> 13
- `mpeg-ts` 0.6 -> 0.7
- `rtp-packet` 0.5 -> 0.6

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `dvb-stream-v0.8.0`.
