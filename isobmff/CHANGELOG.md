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

### Breaking

- New crate. Nothing was released before this entry, so there is no previous
  path to break.
