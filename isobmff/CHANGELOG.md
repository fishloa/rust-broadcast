# Changelog

All notable changes to `isobmff` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Added

- `writer::Writer::progressive`, the sans-IO progressive (single-file) writer:
  it fixes the whole layout from an `OwnedPlan` before a sample byte arrives
  (interleaved chunks, `stco` or `co64`, the `mdat` header form and every
  sample's absolute offset), then takes each sample with `push` and emits
  `Write { offset, bytes }` values. A `Sink::Seekable` caller places them in
  any order and the writer holds no sample bytes; a `Sink::AppendOnly` caller
  gets file order enforced, with out-of-turn samples held in a buffer capped by
  its own `reorder_bytes` (`Error::ReorderBufferFull`). A pushed length that
  disagrees with the plan is `Error::PlanMismatch`, a missing one at `finish`
  is `Error::PlanIncomplete`, and `Interleave::AsPlanned` without a source
  chunk table is `Error::NoPlan`.
- `plan::OwnedPlan` and `plan::PlanBuilder`: a progressive file's sample tables
  in their compact wire form (no per-sample expansion), with the `stsd`
  entries, `elst` entries, durations and source chunk layout, plus a builder
  that run-length codes each table as samples are pushed.
  `SamplePlan::to_owned_plan` turns a reader's index into one.
- `sample_table::build_stsc_with_descriptions`, the `stsc` builder that keeps
  one `stsd` entry per chunk (a plan read from a file writes back the runs it
  was read with); `build_stsc` is the same builder with the first entry
  everywhere.
- `writer::TrackSetup::language` (`mdhd.language`), so a track read from a file
  keeps its language when it is written back; a handler with no media header of
  its own (`meta`, `text`, `tmcd`, …) now gets `nmhd` unless the caller named
  another.

- `writer`, the sans-IO fragmented MP4 writer: `Writer::fragmented` returns a
  `FragmentedWriter` whose `init()` assembles `ftyp`+`moov` from `TrackSetup`
  values (a typed `SampleEntry` or a raw sample-entry box, so a track this
  crate has no codec for still writes), and whose `push`/`flush` emit one
  `[styp] [emsg]* moof mdat` fragment capped by `Limits::max_fragment_bytes`
  (`Error::FragmentTooLarge`). `writer::fragmented_segment` is the same
  builder for a caller that already holds its samples, and
  `writer::fragmented::protect_media_segment` (with `FragmentProtection`) is
  the CENC media-segment pass moved down from `transmux` — the
  `senc`/`saiz`/`saio` boxes are emitted by the writer when a sample carries
  `SampleAux`.
- `reader::Reader`, the sans-IO progressive-file reader: it asks the caller
  for structure byte ranges with `Step::Need`, surfaces every track
  (`Step::Tracks`, including handlers and sample entries transmux cannot map,
  with each `stsd` entry's raw bytes), then yields one `SampleRef` at a time in
  decode-time or file order, and `seek_to` lands every track on its sync sample
  at or before a time. It owns the supplied `moov` and nothing else, so a
  `moov` after a multi-gigabyte `mdat` costs headers only; a forward-only
  source gets a typed `Error::MoovAfterMdatForwardOnly` instead.
- `plan::SamplePlan`, the read plan `Reader::plan()` returns from the parsed
  index, with a per-track `cursor(&Limits)` and no per-sample storage.
- `sample_table::CursorState`, the walk's owning state (plain integers) that
  `TrackCursor` is now expressed over, so a reader that owns its `moov` bytes
  can keep a position per track without a self-referential borrow.
- `Error::{MoovAfterMdatForwardOnly, SupplyMismatch, NoIndex}`, and
  `Reader::bytes_requested()` under `scan-counters`.

- `Limits`, the hostile-input caps for the Phase 2 reader: walk depth, `moov`
  size, samples per track, one sample, one fragment and one metadata item,
  each with a named default and a `with_*` builder.

- Typed `stz2` (`CompactSampleSizeBox`, 4/8/16-bit entries) and `sdtp`
  (`SampleDependencyTypeBox`, four 2-bit fields per sample) as `StblChild`
  variants, both symmetric `Parse`/`Serialize`.

- `sample_table::TrackCursor`, a lazy per-track cursor over
  `stts`/`ctts`/`stsc`/`stsz`/`stz2`/`stco`/`co64`/`stss` yielding one
  `SampleRef` at a time with `len()`, `peek_decode_time()` and
  `seek_sync()`. It validates the tables against `Limits` up front and
  allocates nothing proportional to a declared sample count.

- `sample_table::{build_stts, build_ctts, build_stss, build_stsc}`, the
  run-length sample-table encoders moved down from `transmux`.

- Four-CC constants for the boxes the crate already modelled but had not
  named: `segments::{FREE, SKIP}`, `init_segment::{MOOV, MVHD, TRAK, TKHD,
  MDIA, MINF, STBL, STSD}`, `movie_fragment::{TRAF, TFHD, TFDT, TRUN}`,
  `cenc::{TENC, SENC, SAIZ, SAIO, SINF}`, `sample_groups::PRFT`, and
  `box_types::UUID` — so a consumer names a box type through `isobmff` instead
  of a raw literal (enforced by the `one_box_home` guard).

- `BoxItem` (one box: header, offset, whole bytes and body), `BoxIter::clamped`
  (the tolerant mode of the one walker), `BoxItem::children`, and the
  one-level `box_types::find_box` lookup. Strict iteration is `BoxIter::new`;
  the strict `parse_box` remains the single-box entry point.

- The ISO/IEC 14496-12:2026 box layer, extracted from `transmux`: the box /
  FullBox headers (64-bit `largesize`, `uuid` usertype), the `BoxIter`
  walker, `ftyp`/`styp`/`mdat`, the sample-table and timing boxes, the
  visual-extension boxes, the sample-group boxes, the movie-fragment boxes,
  the CENC boxes (ISO/IEC 23001-7 §12), and the §8.8.7/§8.8.8 fragment
  sample-offset resolver.
- `Error`, `Result` and the box-level `TrackId` (the 14496-12 `track_ID`,
  which prints as a bare integer and serialises transparently).
- `ser_timescale` (feature `serde`): serialise a `Timescale` as the plain
  ticks-per-second integer the wire box carries.
- The codec configuration boxes moved from `transmux`: `avcC`/`hvcC`/`vvcC`
  (`avc_config`/`hevc_config`/`vvc_config`), the AVC/HEVC NAL-unit newtypes
  (`nalu_types`), `av1C`/`vpcC` (`av1`/`vp9`), `dOps`, `dfLa`, `dac4`,
  `dac3`/`dec3`, `ddts`, `mhaC`, the Dolby Vision record (`dovi`), the
  `mdcv`/`clli` bodies (`hdr_boxes`), the `esds` descriptor chain
  (`mp4esds`), and the AVS2/AVS3 configuration records
  (`avs2`/`avs3`/`avs3_audio`), with the real-fixture tests that exercise
  them.
- The sample entries and the `moov` tree moved from `transmux`:
  `init_segment` (`mvhd`, `tkhd`, `mdhd`, `hdlr`, `vmhd`/`smhd`, `dref`,
  the sample-table boxes, `stsd`, the audio/visual/subtitle sample entries,
  `SampleEntryVariant`, `StblChild`, `TrackBox`, `MovieBox`, `mvex`/`trex`,
  `edts`, the AVC parameter set stream and the box-level
  `protect_sample_entry`), `sample_entries`, `subtitle_entries`,
  `avc_parameter_set`, `track_reference`, and the `av01`/`vp09`/`avs3`/`avst`
  sample entries in `av1`/`vp9`/`avs3`, with their in-file and real-fixture
  tests.
- `sample_entries::SAMPLE_ENTRY_RESERVED_LEN` and
  `init_segment::MediaInformationBox::set_leading_opaque_header` are public:
  a caller above this crate builds an audio sample entry and installs an
  opaque media header.
- `emsg` (ISO/IEC 23009-1 §5.10.3.3, DASH-IF IOP Part 10): `EmsgBox`,
  `PresentationTime`, `EmsgVersion` and the box constants, moved from the
  absorbed `mp4-emsg` crate, plus its real-fixture tests and examples. The
  emsg parse/serialize errors are variants of this crate's `Error`.

### Changed

- `ftyp`/`styp` accept the 64-bit `largesize` and `size == 0` (to-end) box
  forms (ISO/IEC 14496-12:2026 §4.2); the serializer still emits the compact
  form.

### Breaking

- The box walkers are consolidated into one iterator. `BoxRef`, `ClampedBox`,
  `box_iter`, `clamped_boxes`, `find_top_box`, `find_child_box`, `child_boxes`,
  `walk_children`, `ChildBox`, `box_support::box_body`, `is_four_cc`,
  `child_extent`, `child_extent_in`, `encode_box_header` and
  `write_compact_box_header` are removed. Use `BoxItem`/`BoxIter` (with
  `BoxIter::new` or `BoxIter::clamped`), `BoxItem::children`, `find_box`,
  the single `box_types::box_body`, `BoxType::is`, `write_box_header` and
  `write_full_box_header`. Wire bytes and parsed results are unchanged.

### Breaking

- New crate. Nothing was released before this entry, so there is no previous
  path to break.
