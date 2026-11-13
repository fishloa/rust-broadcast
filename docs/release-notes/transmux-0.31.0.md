# transmux 0.31.0

_Released 2026-11-14._

Breaking-class release (0.30 -> 0.31): breaking API changes are listed below, and the crate's sibling dependencies move to new caret epochs.

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

- the Smooth Streaming client Manifest moved to the new `broadcast-smooth` crate, below this container hub as the HLS playlist syntax is (#2356). Removed from `transmux` and the crate root: the module `smooth_parse` and its `SmoothManifest`, `StreamIndex`, `QualityLevel`, `C`, `StreamType`, `SmoothParseError`, `default_limits`, `MAX_CHUNK_RUN`, `MAX_CODEC_PRIVATE_DATA_HEX_LEN`, `TOKEN_BITRATE`, `TOKEN_START_TIME` and the private `xml_walk` plumbing. Parse and render with `broadcast_smooth::SmoothManifest` (`parse` / `parse_with_limits` / `to_xml`); `SMOOTH_TIMESCALE` is re-exported unchanged from `broadcast_smooth`. `track_spec_from_quality_level` stays here (now `transmux::smooth_codec`) because it builds this crate's `TrackSpec`, and takes a `broadcast_smooth::QualityLevel`.
- `base_url` is gone. `resolve`, `resolve_chain` and `first_forbidden_char` (with the `resolve_url_reference`/`resolve_base_url_chain` crate-root aliases) moved to `dash_runtime::mpd::base_url`, the only crate that resolved MPD `BaseURL` chains; transmux writes MPDs and never fetched them (#2355).
- the `xs:duration` reader moved to `broadcast_dash::duration` (`parse_iso8601_duration`, `DashParseError`). `transmux::dash_parse`, `transmux::parse_iso8601_duration` and `transmux::DashParseError` are removed; transmux keeps only the writer.
- the `xs:duration` writer `dash::xs_duration` is removed; use `broadcast_dash::duration::xs_duration`, the one writer in the workspace (the rendered strings are unchanged) (#2465, #2502).
- `rtp::base64_encode` and `rtp::base64_decode` are removed; use `broadcast_common::base64::{base64_encode, base64_decode_lenient}` (the same RFC 4648 encoding and lenient decoding) (#2463).
- transmux describes RTP media sections only; the SDP session description is built by `rtsp_runtime::sdp::session_description` (#2357). `RtpOutput::sdp` is replaced by `RtpOutput::medias` (`Vec<sdp_types::Media>`, one per stream, the same media descriptions as before); `build_sdp`, `build_sdp_with_connection` and `LOCAL_CONNECTION_ADDRESS` are removed. Build the document with `rtsp_runtime::sdp::session_description("transmux RTP", sdp_types::Origin::with_ip_addr(0, 0, Ipv4Addr::LOCALHOST), sdp_types::Connection::from_ip_addr(addr), out.medias)`; the bytes are unchanged (`fixtures/golden/rtp/rtp-sdp-*.sdp` and `fixtures/golden/rtsp/sdp-transmux-h264-aac.sdp` were recorded from the old renderer and match without regeneration).
- transmux no longer reads MPDs; `dash_runtime::mpd` is the workspace's one MPD reader (#2340). Removed: `Mpd`, `MpdType`, `Period`, `AdaptationSet`, `Representation`, `SegmentTemplate` (including `match_url`), `SegmentTimeline`, `S`, `TimelineSegment`, `TemplateMatch`, `dash_parse::default_limits` and `dash_parse::MAX_TIMELINE_SEGMENTS`, and every `DashParseError` variant except the one `parse_iso8601_duration` returns. Parse with `dash_runtime::mpd::Mpd::parse` (or `parse_with` for limits); reverse-match a segment URL with `dash_runtime::address::template::match_url`. transmux keeps MPD writing (`DashPackager`) and `parse_iso8601_duration`.
- removed `MAX_FORMAT_WIDTH` (and its `transmux` re-export); the `%0Nd` template width bound belongs to the MPD reader, `dash_runtime::address::template::MAX_FORMAT_WIDTH`.
- KLV framing comes from `broadcast_common::klv` (SMPTE ST 336, #2341). Removed from `transmux::klv` and the crate root: `encode_ber_length`, `ber_length`, `KlvItem`, `UniversalLabel` and `UNIVERSAL_LABEL_LEN`; use `broadcast_common::klv::{encode_ber_length, decode_ber_length, ber_length_size, KlvItem, UniversalLabel, UNIVERSAL_LABEL_LEN}`. Broadcast-common's borrower `KlvItem<'a>` is now the workspace's one KLV item type: `transmux::KlvItem` (the crate root) re-exports it, and transmux's own owned `KlvItem { key, value: Vec<u8> }` with its `Parse`/`Serialize` impls is gone (keep `item.key` and `item.value` as owned locals if a copy is needed). The MISB ST 0601 Local Set (`UasLocalSet`, `LocalSetItem`, BER-OID tags, checksum) stays in transmux.
- `klv::UasLocalSet::serialize_with_checksum` returns `Result<Vec<u8>>`: a value whose BER length cannot be encoded is reported instead of producing a packet with a corrupt length field.
- the RTMP spokes `RtmpDemux` and `RtmpMux` run on `rtmp-runtime`'s chunk stream (`ChunkAssembler`, `ChunkWriter`), and transmux's own copy of the RTMP wire layer is removed (#2339). Removed from `transmux` and `transmux::rtmp`: `AmfValue`, `BasicHeader`, `Command`, `DEFAULT_CHUNK_SIZE`, `HANDSHAKE_PACKET_LEN`, `Handshake0`/`Handshake1`/`Handshake2`, `Message`, `MessageHeader`, `ProtocolControl`, `RTMP_VERSION`, `read_chunks` and `write_chunks`. Use `rtmp_runtime::handshake`, `rtmp_runtime::chunk::{ChunkAssembler, ChunkWriter, Message, DEFAULT_CHUNK_SIZE}`, `rtmp_runtime::message::ProtocolControl` and `rtmp_runtime::amf0` instead. `RtmpFlvError` wraps wire errors as `RtmpFlvError::Rtmp(rtmp_runtime::RtmpError)` and adds `ChunkSizeTooLarge`; transmux gains a dependency on `rtmp-runtime`.

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

## Also in this release

### Added
- `skills/transmux-basics/SKILL.md`: a Symposium agent skill shipped in the published package, so an AI coding agent working in a project that depends on `transmux` gets usage guidance covering the `Unpackage`/`Package` hub loop, the `Media`/`Track`/`Sample`/`TrackSpec`/`CodecConfig` IR, streaming demux and segmentation, repackaging, timeline transforms and splice, CENC decrypt, the fMP4/CMAF validator, the `cli` packager, features and the common pitfalls.
- `audio_profile` (serde behind the `serde` feature): `AudioTrackSelector` (with `position`), `AudioProfile`, `AudioProfileOverride` and `apply_overrides`, the operator-declared HE-AAC description moved from multimux; it reports unmatched and refused declarations instead of logging them (issue #2554).
- `stitch::Stitcher` (`set_tracks`, `discontinuity`, `apply`): one continuous sample timeline across a source's reported discontinuities (DASH Period boundaries, skipped segments), one offset for every track so A/V alignment survives; moved from multimux (issue #2554).
- `rtp::rescale_ts(ticks, from, to)`: rescale a timestamp between clock rates, wrapping modulo 2^32 as RTP timestamps do (RFC 3550 §5.1) (#2463).
- `transmux::Container` (`Fmp4`, `MpegTs`, `WebVtt`) with `name()` and `segment_extension()`: which container a segmenter cuts and an origin serves, defined once here; moved from `hls_runtime::server` (issue #2554).

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `broadcast-hls` 0.6 -> 0.7, `cc-data` 0.8 -> 0.9, `container-probe` 0.3 -> 0.4, `dvb-si` 13 -> 14, `mp4-emsg` 0.6 -> 0.7, `mpeg-pes` 0.7 -> 0.8, `mpeg-ps` 0.7 -> 0.8, `mpeg-ts` 0.7 -> 0.8, `rtp-packet` 0.6 -> 0.7.
- New sibling dependencies: `broadcast-dash` 0.1, `broadcast-smooth` 0.1, `rtmp-runtime` 0.10.

---

Published from tag `transmux-v0.31.0`.
