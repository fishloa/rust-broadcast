# mpeg-ts 0.10.0

_Released 2026-11-22._

Minor for a 0.x crate. Own additive change: `TsResync` now locks onto 192-byte M2TS source packets as well as 188- and 204-byte ones, which is what lets `transmux` demux an M2TS file at all. The breaking part is the epoch move: `broadcast-common` 14 and `mp4-emsg` 0.9.

## Breaking changes

- Builds against new caret epochs of workspace siblings (`broadcast-common` 13 -> 14, `mp4-emsg` 0.8 -> 0.9). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each at the jiff 1.0 epoch, and the `Timestamp`/`ByteSpan`/`emsg` values it takes are those versions' types.

## Added

- `TsResync` locks onto 192-byte M2TS (BDAV) source packets as well as 188- and 204-byte ones and strips each 4-byte `TP_extra_header`: new `PacketStride::M2ts192`, `PacketStride::packet_len`, `M2TS_PACKET_SIZE` and `M2TS_EXTRA_HEADER_LEN`. An M2TS packet is emitted as soon as its 188 bytes arrive, not when the next packet's header does (#3988).

## Changed

- Internal: error types derive `thiserror::Error` (messages unchanged) (#3974, #4020).

## Tests

The resync lattice, the section reassembly and packetisation boundaries, the adaptation-field and PCR paths, and the M2TS stride with its extra header are covered by the crate's suites and by the 192-byte fixture that exercised the fix.

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #4020 (`thiserror`).

---

Published from tag `mpeg-ts-v0.10.0`.
