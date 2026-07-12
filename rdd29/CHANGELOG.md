# Changelog

All notable changes to `rdd29` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-07-12

### Added

- `AtmosFrame` — parser+serializer for one complete SMPTE RDD 29:2019
  `ATMOSFrame` element (§2.1/§4.2): `ATMOSVersion`, `SampleRate`, `BitDepth`,
  `FrameRate`, `MaxRendered`, and its `SubElementCount` sub-elements.
- `AnyElement` — dispatch enum over the three concrete sub-element types
  plus `Unknown` (reserved/unrecognized `ElementID`s, round-tripped
  verbatim per §5.1.1's "the decoder shall skip the element").
- `BedDefinition1` — a channel-based audio bed's channel list (§2.2/§4.3):
  `MetaID` + `ChannelId`/`AudioDataID` pairs. `ChannelId` (Table 6) carries
  the `name()`/`Display` label pair per the workspace's #204 convention.
- `ObjectDefinition1` — one panned audio object's per-`NumPanSubBlocks`
  (Table 7, derived from `FrameRate`) rendering metadata (§2.3/§4.4):
  `ObjectPosX`/`Y`/`Z`, `ObjectSnap`, per-zone `ZoneGain` (Table 9, 9 zones
  per Table 8), `ObjectSpreadMode`/`ObjectSpread` (Table 10), decorrelation
  (`DecorCoefPrefix`, Table 11), and `AudioDescription` (NULL-terminated
  ASCII text, gated by a flag bit — the only field in the whole disclosure
  document with no `§5.4.x` prose description; see `docs/rdd29.md` scope
  decision 2 for how this crate handles the gap honestly). Cannot implement
  `broadcast_common::Parse` (its pan-info loop length needs the enclosing
  `ATMOSFrame`'s `FrameRate` as context) — exposes `parse_with_frame_rate`
  instead; still implements `Serialize`.
- `AudioDataDlc` — one track's audio-essence pointer (§2.4/§4.5):
  `AudioDataID` + `DLCSize` + the opaque remainder as `&[u8]`. The Dolby
  Lossless Coding (DLC) codec's own predictor/residual bitstream past those
  two fields is never parsed — this crate's audio-essence/metadata boundary
  (see `docs/rdd29.md` scope decision 3).
- `distance` module — `distance_xy`/`distance_z` (§3.2): decode the raw
  `DistanceXY`/`DistanceZ`-coded position/spread fields to their `[0,1]`
  linear values. Read-only derived views; the raw wire code is always the
  round-tripped source of truth.
- `Plex(n)` variable-length integer coding (§3.4), used for `ElementID`,
  `ElementSize`, `MetaID`, `AudioDataID`, `MaxRendered`, `SubElementCount`,
  `ChannelCount`, and `ChannelID`. Implements the general escape-doubling
  algorithm per the spec's prose + worked example rather than the literally
  printed (and internally inconsistent) `Plex(8)` pseudocode — see
  `docs/rdd29.md` scope decision 1.
- `ElementId` (Table 1), `SampleRate` (Table 2), `BitDepth` (Table 3),
  `FrameRate` (Table 4, also driving `NumPanSubBlocks` via
  `FrameRate::num_pan_sub_blocks`), `ZoneId`/`ZoneGain` (Tables 8/9),
  `ObjectSpreadMode` (Table 10), `DecorCoefPrefix` (Table 11) — each with
  the `name()`/`Display` label pair per the workspace's #204 convention.
- All "Reserved (set to `0xNN`)" fields are hard-validated against their
  documented literal constants on parse (`Error::InvalidReserved`), and
  always re-emitted literally on serialize — see `docs/rdd29.md` scope
  decision 4 for why this differs from `st337`'s soft-preserved `Pf`.
- `docs/rdd29.md` — curated transcription of SMPTE RDD 29:2019 §1-§5
  (fetched directly from `pub.smpte.org`), the implementation/audit oracle
  for this crate, including the crate's scope decisions and the two honest
  gaps this disclosure document itself left unresolved.
- Real-fixture test (`tests/fixture_eac3.rs`) wrapping the same real
  834-byte E-AC-3 syncframe already committed for `st337`'s own real-fixture
  test (`tests/fixtures/eac3_frame0.bin`, originally extracted from this
  workspace's `fixtures/ts/dolby/eac3.ts`, issue #426) as an opaque
  `AudioDataDLC` payload, with byte-identical parse/serialize round-trip
  coverage and a mutation test guarding against a `self.raw`-style
  passthrough serializer.
- Integration round-trip tests (`tests/round_trip.rs`): full multi-element
  frames at every `FrameRate`, `Plex`-escalated `ElementSize`s (payload
  large enough to force the 8-to-16-bit escape), reserved/unknown-element
  pass-through, and (behind the `serde` feature) serde round-trips.
- `tests/label_coverage.rs` — the workspace's issue #204 label-convention
  drift-guard (nine spec/field enums).
- Two runnable examples: `build_frame` (construct a bed + object + audio
  essence from typed fields and serialize) and `parse_frame` (wrap the real
  E-AC-3 fixture as an opaque `AudioDataDLC` payload, parse it back, and
  confirm the byte-identical payload).
- `#![no_std]` (via `#![cfg_attr(not(feature = "std"), no_std)]`) + `alloc`;
  builds standalone with `--no-default-features` and on a bare-metal
  (`thumbv7em-none-eabi`) target.
- `serde` support (`Serialize`/`Deserialize` derives) behind the `serde`
  feature.
- New fuzz target `rdd29`: exercises `AtmosFrame::parse` + round trip on
  arbitrary bytes.
