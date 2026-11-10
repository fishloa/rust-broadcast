# transmux 0.31.0

_Released 2026-11-10._

Breaking-class release (0.30 -> 0.31). One public type rename, four ISO BMFF struct changes, and an `i32` typing fix on `CleanApertureBox`. Public constructors and builders are otherwise unchanged.

Version: 0.30.0 -> 0.31.0.

## Breaking changes and migration

- `rtmp::RtmpError` is renamed `rtmp::RtmpFlvError` (re-exported as `transmux::RtmpFlvError`), so it no longer shares a name with `rtmp_runtime::RtmpError`.

```text
RtmpError  ->  RtmpFlvError
```

- `CleanApertureBox`'s eight fields are `i32` (ISO/IEC 14496-12:2026 §12.1.4.2); `horizOffN`/`vertOffN` may be negative and were misread as large positive values. New `CleanApertureBox::aperture_within`.

- `SampleGroupDescriptionBox::default_sample_description_index` is renamed `default_group_description_index`.

- `SampleAuxInfoSizesBox::default_sample_info_size` and `sample_info_sizes` are `u32` / `Vec<u32>` (were `u8` / `Vec<u8>`).

- `DataReferenceBox::entries` is `Vec<DataEntry>` (was `Vec<DataEntryUrlBox>`); wrap an existing url entry as `DataEntry::Url(..)`.

```rust
// before
let e: Vec<DataEntryUrlBox> = box_.entries;
// after
let e: Vec<DataEntry> = box_.entries;
// url entry becomes DataEntry::Url(..)
```

## Added

- `LlHlsSegmenter::set_start_time_ns` and `StreamingTsHlsSegmenter::set_start_time_ns`: start the decode clock at a given time.
- `SubSampleInformationBoxV2` and `SubSampleReferenceTableBox` (`ssrt`); `DataEntry` (`url `, `urn `, `imdt`, `snim`, plus `Other`).
- `HEVCSampleEntry::array_completeness_violations` plus `HEVC_VPS_NUT`/`HEVC_SPS_NUT`/`HEVC_PPS_NUT`/`HEVC_SUFFIX_SEI_NUT`.
- `avc_parameter_set` module: `AVCParameterSampleEntry`, `AVCParameterSample`, `AvcNalUnit`; `SampleEntryVariant::AvcParameterSet`.
- `track_reference` module: `TrackReferenceBox` / `TrackReferenceTypeBox` with `AVCP_REFERENCE_TYPE`; `TrackBox::track_references`.
- `InputDegradation::Scrambled` (on the `#[non_exhaustive]` enum).

## Fixed

- TS time-base discontinuity lifts every stream of the program by one common offset.
- `StreamingTsDemux::finish` demuxes aligned packets at end of input.
- `SubSampleInformationBox` rejects version 2 with a typed error.
- `SampleGroupDescriptionBox` reads/writes versions 2 and 3 per ISO/IEC 14496-12:2026 §8.9.3.2.
- `SampleAuxInfoSizesBox` reads/writes versions 1 and 2.
- `DataReferenceBox` keeps each entry's four-CC.
- Demuxed `TrackSpec::display` takes sample aspect ratio from the sample entry's `pasp`.
- `AVCDecoderConfigurationRecord` rejects `bit_depth_luma_minus8`/`bit_depth_chroma_minus8` above 4.
- `AVCDecoderConfigurationRecord::parse` accepts `numOfSequenceParameterSets == 0`.
- `HEVCDecoderConfigurationRecord::parse` keeps unmodelled NAL arrays.
- Demuxed `DisplayGeometry::output_rect` follows the sample entry's `clap`.
- PTS/DTS unrolling, lost-tail PES, PMT version removal, scrambled payload handling, CRC-valid PMT, Fmp4Demux trailing moof, FLV 32-bit timestamp unroll, WebM TimestampScale validation, `vvcC` writer bounds, AVS3-audio `dca3` write overrun.

## Other changes

- Container box writers share one FullBox header writer and one visual sample entry writer.
- Shared helpers replacing private copies; internal refactors with no public effect.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11 (already live)
- `broadcast-hls` 0.6 -> 0.7
- `cc-data` 0.7 -> 0.8
- `container-probe` 0.2 -> 0.3
- `dvb-si` 12 -> 13
- `mp4-emsg` 0.5 -> 0.6
- `mpeg-pes` 0.6 -> 0.7
- `mpeg-ps` 0.6 -> 0.7
- `mpeg-ts` 0.6 -> 0.7
- `rtp-packet` 0.5 -> 0.6

---

Published from tag `transmux-v0.31.0`.
