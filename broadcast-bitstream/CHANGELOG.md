# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) with the
0.x caveat that the API is expected to grow.

## [Unreleased]

### Added

- `findings`: the report of what a tolerant parser skipped, in the workspace's
  one damage vocabulary. `ParseFinding` names the class (a rejected parameter
  set, a malformed SEI, unparsable HDR metadata, an access unit the splitter
  cannot delimit, an audio frame header, a LAME tag) with the offset the parser
  knows, and `From<ParseFinding> for Damage` converts it into
  `broadcast_core::damage::Damage`. `Findings` is the collector a parser is
  handed. The parsers that can continue past a broken structure now record one
  finding per skip instead of dropping it silently: `nal::sei_messages`,
  `nal::caption_cc_data`, `hdr::HdrStaticMetadata::absorb_hevc_sei_nal`/
  `from_hevc_record`, `hdr10plus::{Hdr10PlusMetadata::hevc_sei_nal_payloads,
  from_hevc_sei_nal}`, the AC-3/E-AC-3 splitters, and `au::split_access_units`/
  `AccessUnitSplitter::take_findings`.


- `mpeg_audio::{LameTag, parse_lame_tag}`: the LAME/Xing `Info` tag in the
  first MPEG Layer III frame, with the 9-byte encoder version string and, for a
  LAME-family encoder (`LAME`/`Lavf`/`Lavc`), the 12-bit encoder delay and
  padding. The layout is a de-facto format, cited in the module with its
  public document and the LAME/FFmpeg readers that define the offsets.
  `MpegAudioFrameHeader::xing_offset` locates the magic for a frame's version
  and channel mode.
- The MPEG-2 video sequence layer (`mpeg2` feature), moved from `transmux`:
  `mpeg2::{Mpeg2SeqHeader, SEQUENCE_HEADER_CODE}` (private fields with
  `width()`/`height()` accessors) and `h262::{SequenceHeaderGeometry,
  SequenceDisplayExtension}` with their parsers
  (`find_sequence_header`, `parse_sequence_at`, `sample_aspect_ratio`) and the
  `extension_and_user_data(2)` walk (`picture_user_data_payloads`,
  `afd_from_mpeg2_user_data`). The presentational `DisplayGeometry` conversion
  stays in `transmux`.
- `user_data` (no codec feature), moved from `transmux`: `AfdSignal`,
  `PictureUserData` (private fields with accessors and a constructor), the
  T.35 / GA94 identifiers and field constants, and the one AFD / bar-data
  decoder each (`afd_from_t35_payload`, `bar_data_from_t35_payload`,
  `bar_data_from_ga94_payload`). The H.262, AVC and HEVC display paths share
  them.
- The MPEG-1/2/2.5 audio layer (`mpeg_audio`, feature `mpeg-audio`), moved
  from `transmux`'s `mpeg_legacy`: `MpegAudioLayer`, `MpegAudioFrameHeader`
  (private fields with accessors) and `MpegAudioFrameHeader::parse`, plus
  `MPEG_AUDIO_SYNCWORD`. The frame header now exposes the MPEG version it
  already decoded as the typed `MpegAudioVersion` (MPEG-1 / MPEG-2 / MPEG-2.5,
  spec labels with `name()` + `Display`).
- The AAC layer (`aac_asc`, `aac` feature), moved from `transmux`:
  `AudioObjectType`, `SamplingFrequencyIndex`, `ChannelConfiguration` (spec
  labels with `name()` + `Display`), `AudioSpecificConfig` with its `Parse` /
  `Serialize` round trip and private fields, `HeAacSignaling`, `HeAacProfile`,
  and `AdtsHeader` with `parse_adts_header`, `build_adts_header`,
  `to_adts_header` and `adts_header_len`. `EsdsBoxCodecInfo` moved to
  `codec_config_ext` (`isobmff` + `aac`).
- The AC-3 / E-AC-3 layer (`ac3_syncframe`, `ac3` feature), moved from
  `transmux`: `Ac3SyncframeInfo`, `Ec3SyncframeInfo`, `Ec3SplitFrame`, the
  frame/access-unit splitters, the CRC helpers, and
  `ec3_specific_box_from_syncframes` / `into_dac3` / `into_dec3` behind
  `isobmff`.
- The DTS core layer (`dts_core`, `dts` feature), moved from `transmux`:
  `DtsCoreFrameInfo`, `DtsCoreFrame`, `split_dts_core_frames`, and `into_ddts`
  behind `isobmff`.
- The MPEG-H Audio Stream framing (`mhas`, `mpegh` feature), moved from
  `transmux`: `walk_mhas_packets` and `find_mpegh3da_config`.
- A `print_mpeg_audio_headers` example over `fixtures/audio/audio.mp3`: each
  frame's version, layer, bitrate, sample rate and length, then the frame count.
- A `legacy_fixtures` test over real captures: the committed MPEG-2 transport
  stream's sequence header (320x240, 4:3, 25 Hz), a capture carrying two
  sequence display descriptions plus AFD in MPEG-2 picture user data, and the
  MP3 fixture as a 44-frame chain walked to its last byte.
- A `print_adts_headers` example over `fixtures/audio/aac.adts`, an
  `audio_fixtures` test that walks that ADTS chain, a broadcast capture's AAC
  track and one committed AC-3 / E-AC-3 / DTS stream frame by frame, and the
  `broadcast_bitstream_aac_asc`, `broadcast_bitstream_adts`,
  `broadcast_bitstream_ac3`, `broadcast_bitstream_eac3`,
  `broadcast_bitstream_dts_core` and `broadcast_bitstream_mhas` fuzz targets,
  plus `broadcast_bitstream_mpeg2`, `broadcast_bitstream_mpeg_audio` and
  `broadcast_bitstream_user_data`.

- `annexb::start_code_positions` is crate-private: nothing outside the crate
  needs the allocation. `annexb::find_start_code_prefix` stays the one public
  scan primitive, so external callers lose no capability.

### Breaking

- The tolerant parsers take the `Findings` collector their findings go into:
  `nal::sei_messages`, `nal::caption_cc_data`,
  `hdr::HdrStaticMetadata::absorb_hevc_sei_nal`/`from_hevc_record`,
  `hdr10plus::{Hdr10PlusMetadata::hevc_sei_nal_payloads, from_hevc_sei_nal}`,
  `ac3_syncframe::{split_ac3_syncframes, split_ac3_syncframes_resyncing,
  split_ac3_syncframe_ranges, split_eac3_syncframes,
  split_eac3_syncframe_ranges}` and `au::split_access_units`. A caller with no
  channel for the reports passes a `Findings` it drops.
- `Hdr10PlusMetadata::from_hevc_sei_nal` returns `Option<Self>`: a message
  whose body does not parse is recorded in its `Findings` argument instead of
  being returned as an inner `Err`.

### Fixed

- An MPEG-2 video `sequence_header()` carrying a zero `horizontal_size_value`
  or `vertical_size_value` is rejected with `Error::InvalidValue` instead of
  yielding a zero geometry no decoder can use. H.262 §6.3.3 forbids either
  value being zero (a zero would emulate a start-code prefix).
  `Mpeg2SeqHeader::find` and `h262::parse_sequence_at` share the check, and
  `broadcast-bitstream/tests/mpeg2_sequence_size_zero.rs` pins it with the
  bytes that failed the fuzz smoke run.

### Changed

- The Annex B start-code scan (`annexb::find_start_code_prefix`) finds each `01`
  byte with `memchr` again, so a scan over a multi-megabyte access unit stays
  vectorised. The crate now depends on `memchr` (`default-features = false`).
- Feature `vvc` added (default on) for the H.266 decoders.

### Added

- The H.264 slice-header layer (`slice_header`, `h264` feature), moved from
  `transmux`: `AvcSliceHeader`, `AvcSlicePps`, `AvcPicture`, `AvcFieldInfo` and
  the `AvcParamSets` SPS/PPS cache with `decode_avc_slice_pps`,
  `parse_avc_slice_header`, `classify_au`/`classify_nals` and `parse_slice`.
  Private fields with accessors and `#[non_exhaustive]` throughout.
- The coded interlace facts (`interlace`, `h264`/`hevc`): `VideoScan`,
  `PicStruct`, `SourceScanType` and `PictureFacts`, and the public
  `avc_facts`/`hevc_facts` decoders over one access unit's NAL units.
- The HDR10+ dynamic metadata model (`hdr10plus`, `hevc`), moved from
  `transmux`: the ST 2094-40 Application #4 types with their `Parse`/
  `Serialize` round trip, `Hdr10PlusMetadata` (constructor plus `with_*`
  setters), a `WindowRegion` builder, and the SEI/T.35 walkers.
- The HDR10 static metadata type (`hdr`, `hevc` + `isobmff`), moved from
  `transmux`: `HdrStaticMetadata` with constructor and accessors, the prefix
  SEI absorb/emit codec, and the `mdcv`/`clli` sample-entry glue.
- The ISOBMFF codec-configuration glue (`isobmff`): `nalu_ext`
  (`AvcSpsCodecInfo`, `HevcNalUnitCodecInfo`), `codec_config_ext`
  (`HevcRecordCodecInfo`, `VvcRecordCodecInfo`) and
  `avc_config_from_sps_pps`, all moved from `transmux`.
- `nal` gains `AVC_NAL_SLICE`, `AVC_NAL_AUD` and `AVC_NAL_HEADER_LEN`.
- `Error::OutputBufferTooSmall` (a serialization target too small) and, behind
  `isobmff`, `Error::Isobmff`.
- A `print_capture_slices` example and `h264_capture_slices` real-fixture test
  that read every coded picture's slice type and frame/field flag out of
  `fixtures/ts/h264-aac.ts`; `broadcast_bitstream_slice_header` and
  `broadcast_bitstream_hdr10plus` fuzz targets.

### Changed

- `Error` drops the `Eq` derive (it keeps `PartialEq`): the optional `isobmff`
  glue wraps the box layer's error, which carries none.

- Initial contents: the exp-Golomb/RBSP bit reader (`bitreader`), RBSP
  emulation-prevention handling (`rbsp`), `NalCodec`, `AvcNalUnitType`,
  `HevcNalUnitType`, key-frame/IRAP and random-access-point classification, the
  SEI message walk and ATSC A/53 caption extraction (`nal`), Annex B and
  length-prefixed NAL iteration and conversion (`annexb`), and the incremental
  `AccessUnitSplitter` (`au`).
- `AvcSequenceParameterSet`/`AvcPictureParameterSet` typed wrappers, with
  conversions to `isobmff` decoder-configuration newtypes behind the `isobmff`
  feature.
- The access-unit splitter's open-NAL bound is the caller's
  (`AccessUnitSplitter::with_max_open_nal_bytes`); the default is unlimited.
- `split_annexb` and `annexb_to_avcc` examples over a real committed Annex B
  fixture.
- The sequence parameter set decoders (`sps`), moved from `transmux`:
  `decode_avc_sps`, `decode_hevc_sps`, `decode_vvc_sps` (new `vvc` feature) with
  their profile/tier/level, dimension, chroma, bit-depth and VUI timing facts;
  `decode_avc_vui_timing`, `decode_hevc_vui_timing`, `decode_hevc_vps_timing`,
  `decode_avc_reorder_depth`, `decode_avc_slice_sps` and
  `decode_hevc_interlace_sps`; `decode_ptl` and `is_high_profile`; and the
  display-facts entry points `decode_avc_display_facts` /
  `decode_hevc_display_facts` returning the new `SpsDisplayFacts`.
- The coded picture facts (`facts`), moved from `transmux`'s display-geometry
  module: `AspectRatioIdc`, `SampleAspectRatio`, `PictureRect`, `VideoFormat`,
  `ColourPrimaries`, `TransferCharacteristics`, `MatrixCoefficients`,
  `ColourDescription`, `VideoSignalType`, `ActiveFormatDescription`, `BarData`
  and `EXTENDED_SAR_IDC`, with private fields and accessors throughout.
- The per-codec RFC 6381 helpers (`rfc6381`): `rfc6381_avc1`,
  `rfc6381_hvc1`, `rfc6381_vvc1` and `rfc6381_mp4a` (the last ungated, since it
  describes an audio configuration).
- A `decode_capture_sps` example over a real transport-stream capture, and a
  real-fixture test that walks `fixtures/ts/h264-aac.ts` to its first IDR's SPS.
- A `broadcast_bitstream_sps` fuzz target over every SPS decoder.
