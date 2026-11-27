# Changelog

All notable changes to this crate are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this crate uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `matroska::statistics`: the typed view of the de-facto tag names a real file
  carries, `Tag::statistics()` and `Tags::statistics_for_track()` over
  `DURATION`, `NUMBER_OF_FRAMES`, `NUMBER_OF_BYTES`, `BPS`, `ENCODER`,
  `ENCODER_SETTINGS` and the `_STATISTICS_*` names mkvmerge writes: a duration
  in nanoseconds, a count, a bitrate, a text, and a `_STATISTICS_WRITING_DATE_UTC`
  as a `CivilDateTime`. A value that does not match its documented form is a
  `MalformedTag` finding and the tag still reads. `Targets::track_uids()`,
  `Targets::target_type_value()` and `Targets::target_type()` expose the
  target a tag applies to.

### Breaking

- The wire-format contract types now come from `broadcast-core`.
- `reader::BlockStep` gains `discard_padding: Option<i64>` (nanoseconds): a
  `BlockGroup`'s `DiscardPadding` (RFC 9559 §5.1.3.5.7), positive for silence
  at the end of the Block and negative at its start. A caller that builds a
  `BlockStep` literal must set it.

### Changed

- In `Mode::Permissive` the reader types each `TrackEntry` on its own: a
  corrupt entry is reported and skipped while the other tracks are still
  announced (a strict reader still fails the whole `Tracks` master).
- A malformed or out-of-range `Info.TimestampScale` is reported and the
  default scale is kept instead of failing the file in permissive mode.
- An element whose id is not a known Segment/Cluster child, or whose declared
  size runs past its parent, is reported and recovered by scanning for the
  next known element id; a resync that finds no boundary no longer emits a
  second report for the same damage.

### Added

- `writer::Writer::cut_cluster`: ends the open `Cluster` at a caller-known
  boundary (a container-level timeline discontinuity), flushing held Blocks
  first so the next one opens a new `Cluster`.
- `reader::TrackInfo::codec_delay_ns`: the track's `CodecDelay` (RFC 9559
  §5.1.4.1.6), which Opus uses to state its pre-skip. The writer does not emit
  it yet.
- `reader::TrackInfo::seek_pre_roll_ns`: the track's `SeekPreRoll`
  (RFC 9559 §5.1.4.1.26), the decoder fill a seek needs before output is
  correct. The writer does not emit it yet.
- Initial crate: EBML (RFC 8794) element framing, the scalar payload codecs,
  the EBML header, `Void` and `CRC-32`.
- The full Matroska (RFC 9559) element table with typed master and leaf
  elements, the `SimpleBlock`/`Block` header with none, Xiph, EBML and
  fixed-size lacing, and typed field accessors.
- Byte-identical round trips for every committed Matroska/WebM fixture,
  checked element by element; the element-id table is pinned to the private
  transcription at runtime.
- Hostile-input caps (`Limits`) for element depth, element size and child
  count, plus the reader's master, track, lace, Cues, Cluster-index,
  Block-header, read and resync caps.
- `reader::Reader`: the sans-IO reader (`step`/`supply`), one Cluster header
  and one Block header at a time, unknown-size `Segment`/`Cluster` boundaries
  found by RFC 8794 §6.2 sibling ids, every laced frame yielded with its own
  byte range, `Info`/`Tracks`/`Cues` surfaced once, and permissive-mode
  warnings with a bounded level-1 resync.
- `Reader::seek_to`: the last Cue point at or before a time (RFC 9559 §20), or
  a binary search over the recorded Cluster index when a file has no Cues.
- `writer::Writer`: EBML header, unknown-size `Segment`, `SeekHead`, `Info`,
  `Tracks`, Clusters cut by a caller span plus video keyframes and the
  `SimpleBlock` `i16` range, `Cues` and the `Info.Duration` patch, every write
  returned as `Write::Append`/`Write::At`.
- `writer::{info_element, tracks_element}` builders over the typed element
  model.
- `driver::{FileReader, ForwardReader}` blocking drivers under `std`, with
  bounded reads, frame reads by range, and a retained header tail so a
  forward-only source never seeks back.
- Differential goldens under `fixtures/mkv/golden/` recorded from `origin/main`
  `WebmDemux`/`MkvMuxStage`, and tests for the six fixtures through both
  drivers, at 1-byte/4 KiB/whole supplies, with unknown sizes, all three lacing
  forms, Cues seeking and a cue-less Cluster scan.
- `list_samples` and `remux` examples, a `matroska_reader` fuzz target and a
  4 GiB soak.
