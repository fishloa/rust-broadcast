---
status: DRAFT
---
# transmux 0.27.0

_Released 2026-10-16._

**Breaking release.** The container hub gets typed ids (`TrackId`, `ProgramNumber`, `Pid`), subtitle tracks that survive a remux and appear in DASH/Smooth manifests, real `wvtt` producer fidelity (MP4Box/gpac), E-AC-3 7.1 (dependent substream) support verified against a real stream, DASH `SegmentTimeline::resolve`, ISOBMFF `largesize` boxes, hostile-input bounds (#1130: a 10 GB allocation from four overwritten bytes), and demuxers that skip one damaged unit instead of aborting (`Media::degraded`).

## Why this version

- **Proposed version:** 0.26.0 -> **0.27.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (TrackId/ProgramNumber newtypes, Pid, CodecConfig::Subtitle sample_entry, VttCueBox).
- **cargo-semver-checks vs live 0.26.0 (all features):** MAJOR (3 major, 0 minor lint failures). (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common, broadcast-hls, container-probe, dvb-si, mp4-emsg, mpeg-pes, mpeg-ps, mpeg-ts, rtcp-packet, rtp-packet, scte35-splice.
- **Caret epoch:** 0.26 -> 0.27; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-broadcast-hls = "0.3"
+broadcast-hls = "0.4"
-container-probe = "0.1"
+container-probe = "0.2"
-dvb-si = "11.1"
+dvb-si = "12.0"
-mp4-emsg = "0.4"
+mp4-emsg = "0.5"
-mpeg-pes = "0.5"
+mpeg-pes = "0.6"
-mpeg-ps = "0.5"
+mpeg-ps = "0.6"
-mpeg-ts = "0.5"
+mpeg-ts = "0.6"
-rtcp-packet = "0.5"
+rtcp-packet = "0.6"
-rtp-packet = "0.4"
+rtp-packet = "0.5"
-scte35-splice = "3.0" # dev-dependency
+scte35-splice = "4.0" # dev-dependency
```

## Breaking changes and migration

### 1. Typed ids (#1159)

New `TrackId(u32)` and `ProgramNumber(u16)` (re-exported at the crate root and `ir`; `new`/`value`, no silent `From`; `Debug`/`Display` render the bare number, serde is transparent, so every golden/JSON/manifest is byte-identical). Every public `track_id: u32` is a `TrackId`: `TrackSpec::{track_id,new}`, `Track::track_id()`, `FragmentTrackData`, the `DemuxEvent` track fields, the segmenters' `push(track_id, ..)`, `RtpInputStream`/`RtpTrackInit`, the ISOBMFF box structs (`tkhd`/`tfhd`/`trex`/`mvhd.next_track_id`/`prft.reference_track_id`), `Error::UnmuxableDataTrack`, the Smooth/DASH descriptors, and `TrackSpec::program_number` (`Option<ProgramNumber>`, `with_program(ProgramNumber)`).

```rust
// before
let spec = TrackSpec::new(1, 90_000, config);   seg.push(1, sample);   let id: u32 = track.track_id();
// after
let spec = TrackSpec::new(TrackId::new(1), 90_000, config);   seg.push(TrackId::new(1), sample);   let id: u32 = track.track_id().value();
```

`TrackId::next()` hands out consecutive ids. PIDs: `TrackSpec::source_pid` is `Option<Pid>` and `with_source` takes a `Pid`; `PcrSample::{pid,new}` and `EventProvenance::pid` are `Pid`; `Debug` of the IR renders `Pid(0x0100)` instead of `256`. `TsMux` reports `Error::InvalidInput` if a track would be assigned an elementary PID above 0x1FFF.

### 2. `CodecConfig::Subtitle` carries its sample entry (#1186)

```rust
// before
match codec { CodecConfig::Subtitle { format } => .. }
// after
match codec { CodecConfig::Subtitle { format, .. } => .. }      // gains sample_entry: Option<SubtitleSampleEntry>
```

`Fmp4Demux` fills the `stpp`/`wvtt` entry whole; `build_init_segment` and every packager built on it re-emit the track (entry verbatim, `sthd`/`nmhd` media header, handler `subt`/`text`). `CodecConfig::is_muxable_in_bmff()` is true for a subtitle track with an entry; one without (TS DVB/Teletext) is still `Error::UnmuxableSubtitleTrack`.

### 3. `VttCueBox` (#1249)

Gains `extra_children` (every `vttc` child other than `payl`/`sttg`/`iden`, kept verbatim) and remembers wire order, so a parsed cue serializes to the bytes it was parsed from (MP4Box and gpac write `iden, sttg, payl`). The struct literal can no longer be built outside the crate: use `VttCueBox::new` and set the public fields. New typed `VttAdditionalTextBox` (`vtta`), `WvttSampleBox`, `parse_wvtt_sample`.

### 4. Behaviour: damaged input no longer fails the whole demux (#1144)

`PsDemux`, `Fmp4Demux`, `WebmDemux`, `FlvDemux` and the batch `RtpDepacketiser` skip one damaged unit and report it in the new `Media::degraded` (`InputDegradation::MalformedUnit { position }`; both `#[non_exhaustive]`). Inputs that used to fail now succeed with `degraded` populated: callers that relied on the error as a validity signal must inspect `Media::degraded`.

## What changed (from CHANGELOG `[Unreleased]`)

#### Fixed (E-AC-3 dependent substreams, a real 7.1 stream, #1255)
- An AC-3 syncframe (`bsid` <= 8) is now an independent substream 0 of an E-AC-3 stream (ETSI TS 102 366 §E.1.3.1.2; Annex J.1.5.2.2 allows `bsid` 6 or 8 with a dependent substream). `Ec3SyncframeInfo`, `ac3::split_eac3_syncframes` and `StreamingTsDemux` recognised E-AC-3 syntax only, so a real Dolby Digital Plus 7.1 stream (AC-3 core + dependent E-AC-3 frame, FFmpeg FATE `the_great_wall_7.1.eac3`) produced no track from MPEG-2 TS (`stream_type` 0x87, or 0x06 + descriptor 0x7A) and no access units.
- `Ec3SpecificBox::from_syncframes`: `num_dep_sub` is the number of dependent substreams (highest dependent `substreamid` + 1). It was the last dependent frame's `substreamid`, which is 0 for the first (dependent substreams are numbered from 0), so a lone dependent substream gave `num_dep_sub` 0 and no `chan_loc`. MP4Box (GPAC 26.07) and ffmpeg 8.1.2 both write 1; the resulting `dec3` for the real stream matches MP4Box's byte for byte (`24 00 0c 0f 02 02`).
- Track counts on the three `m6-*` captures: track 1 (PID 0x82, signalled `stream_type 0x06` + Enhanced AC-3 descriptor) gains one real E-AC-3 frame (7 -> 8 samples): a `0B 77` pair in the payload whose byte 5 read as bsid <= 8 used to be parsed as an E-AC-3 header with a garbage length and swallow it; an AC-3-shaped candidate is now judged by its AC-3 CRC. Codec, track ids and the other tracks are unchanged; plain AC-3 PIDs are unaffected (`track_id_baseline` golden updated with this justification).
- Real-stream tests (`tests/eac3_real_dependent_substreams.rs`; the stream has an unstated licence so it is in the private submodule `private/fixtures/eac3-dep/` and the tests skip loudly without it) against the tools' own output: gpac's per-sample CRC32 of MP4Box's file, MP4Box's `dec3`, MPEG-2 TS from ffmpeg and from gpac (PES boundaries inside access units), both producers' MP4s through demux and CMAF. ffmpeg's `dec3` for the same stream disagrees (data_rate 576, `chan_loc` bit 4); the dependent frame's own `chanmap` (Lrs/Rrs) supports MP4Box.

#### Changed (BREAKING) / Added (real wvtt producers, #1249)
- **BREAKING:** `VttCueBox` gains `extra_children` (every `vttc` child other than `payl`/`sttg`/`iden`, e.g. `vsid`/`ctim`, kept verbatim) and remembers the wire order of its children. Before, a parsed cue was written back `payl, sttg, iden` and unknown children were dropped, so it did not serialize to the bytes it was parsed from; MP4Box and gpac both write `iden, sttg, payl`, which is now also the order of a cue built with `VttCueBox::new` (after any extras). The struct literal can no longer be built outside the crate; use `VttCueBox::new` and set the public fields.
- New `VttAdditionalTextBox` (`vtta`, the `NOTE` text both producers write ahead of the cue after a comment), `WvttSampleBox` (`Cue`/`Empty`/`AdditionalText`/`Other`) and `parse_wvtt_sample`: a `wvtt` media sample as typed boxes, serializing back to the same bytes.
- Real `wvtt` / `stpp` artifacts from MP4Box and gpac (progressive, fragmented, DASH) under `fixtures/subtitles/`, checked against the tools' own parse (gpac `inspect` per-sample dts/cts/dur/size/CRC32; MP4Box `-raw` WebVTT export for cue ids, settings, text and NOTE blocks).

#### Fixed (E-AC-3 orphan dependent substream, #1245)
- An E-AC-3 dependent-substream syncframe with no independent frame before it (the data starts on it at stream start, or after a continuity gap / dropped PES / re-anchor cleared the carry) was emitted as a sample of its own with an independent frame's duration, shifting every following timestamp by one frame. It cannot be decoded alone (ETSI TS 102 366 §E.1.3.1.2: dependent substreams immediately follow their independent substream), so `StreamingTsDemux` and `ac3::split_eac3_syncframes` now drop it; a dependent frame after its independent frame still folds into that access unit.

#### Changed
- The Annex B start-code scan (`find_start_code_prefix`, behind every NAL and access-unit
  splitter) finds each `01` byte with `memchr` instead of testing every offset. Same results,
  `annexb_to_length_prefixed` 2.6–4.7× faster and `StreamingTsDemux` 13–39 % less time on the
  H.264 fixtures (new `annexb_scan` bench). Adds a `memchr` dependency
  (`default-features = false`, so the crate stays `no_std`).
- The H.264 I-slice check behind `StreamingTsDemux`'s random-access-point detection unescapes
  only the first 32 bytes of each slice NAL, enough for the two `ue(v)` slice-header fields it
  reads, instead of copying the whole slice. Same results, 17–44 % less `StreamingTsDemux` time
  on the H.264 fixtures on top of the `memchr` start-code scan.

#### Added (subtitles in manifests, #1243)
- `DashPackager` writes a subtitle track as an `AdaptationSet` with `contentType="text"`, `mimeType="application/mp4"` and `Representation@codecs` `stpp` (TTML/IMSC) or `wvtt` (WebVTT), after the video and audio sets (ETSI TS 103 285 §7.1.1; bare `stpp` is the form that clause allows when no TTML profile is declared, and the `stpp` sample entry carries none). New `MediaKind::Text`. `rfc6381_codec_string` returns `stpp` / `wvtt` for such a track; a `Subtitle` track with no ISOBMFF sample entry (TS DVB / Teletext) is still `UnsupportedCodec`.
- `SmoothPackager` writes a subtitle track as `StreamIndex Type="text" Subtype="SUBT"` with `QualityLevel FourCC="stpp"` / `"wvtt"` and the samples as ordinary Smooth fragments ([MS-SSTR] §2.2.2.3 / §2.2.2.5). New `SmoothStreamType::Text`, `FOURCC_STPP`, `FOURCC_WVTT`, `SUBTYPE_SUBT`. The error for any other unsupported codec now names subtitles as supported.

#### Added (#1234 - AVS2/AVS3 video and Audio Vivid ISOBMFF boxes, GY/T 420-2025 Annex A)
- `avs3`: `Avs3ConfigurationBox` (`av3c`, A.3.2.2) with `LibraryDependencyIdc` (Table A.1), `LAvsConfigurationBox` (`lavc`/`la2c`, A.3.2.3/A.2.2.3, per-layer `TemporalLayerInfo`, typed `AvsFrameRateCode`), `AvsVideoSampleEntry` (`avs3`/`lav3`/`avst`/`lav2`; byte-exact children, secondary layer box decodable), and the `telg`/`lrap`/`a3lg` sample-group entries (`TemporalLayerEntry`, `VisualLrapEntry`, `LibrarySampleGroupEntry`).
- `avs2`: `Avs2ConfigurationBox` (`av2c`, A.2.2.2).
- `avs3_audio`: `Ca3SpecificBox` (`dca3`, A.4.3.1) with the general-bit-rate and lossless `Avs3Audio*SpecificConfig` (bit-packed, typed `content_type`).
- Not modelled: the `lidx` LRAP segment index box and the `av3l` restricted-scheme wrapper (A.3.3.3/A.3.3.6), the `av3a` audio sample entry wiring in the init-segment machinery, and `sequence_header()` decoding (carried verbatim).

#### Added (#1227)
- Caption SEI extraction (`caption_cc_data`) also accepts the China GY/T 270-2013 clause 6.3.3 form: `itu_t_t35_country_code` `0x26` (provider 0x0031, `GA94`, type 0x03), in addition to the ATSC `0xB5`.

#### Added
- `dash::xs_duration(Duration) -> String` is public: the workspace's one ISO 8601 / `xs:duration` spelling (jiff `SpanPrinter`); a duration beyond `i64::MAX` seconds saturates to the largest representable value. `multimux` now calls it instead of its own copy (#1216).

#### Changed
- Internal: the 5-byte marker-bit PTS/DTS/`DTS_next_AU` field is encoded/decoded by `broadcast_common::clock33::{pack_marked, pack_marked_with, unpack_marked}` instead of a local copy (each crate keeps its own prefix / marker validation and error variants); no behaviour change (#1208).

#### Fixed (DASH SegmentTimeline, #1187)

- `SegmentTimeline::resolve(start_number, period_end_ticks)` (new, with `TimelineSegment { number, time, duration }`) implements ISO/IEC 23009-1 §5.3.9.6 in full: absolute first `S@t`, explicit `S@t` jumps (timeline gaps) and a **negative `S@r`**, which repeats to the next `S`'s explicit `@t` (`ceil((t_next - t) / d)`) or to the Period end (`ceil((period_end - t) / d)`), as transcribed by DASH-IF IOP v4.3 §3.10.2.2.4 (`specs/rules/dash-mpd-rules.md`). `enumerate` is kept as the no-Period-end wrapper (a negative `r` with no bound is still a single occurrence).

#### Changed (BREAKING, #1186)

- **BREAKING: `CodecConfig::Subtitle` carries its ISOBMFF sample entry (ex-`TODO(#753)`).** The variant gains `sample_entry: Option<SubtitleSampleEntry>` (new `#[non_exhaustive]` enum `Ttml(XmlSubtitleSampleEntry)` / `WebVtt(WvttSampleEntry)`, re-exported from `ir` and `pipeline`). `Fmp4Demux` fills it from the `stpp`/`wvtt` entry whole (namespace, `vttC` header block, trailing boxes), and `build_init_segment` and every packager built on it now re-emit the track: the entry verbatim, an `sthd` (`stpp`/`subt`) or `nmhd` (`wvtt`/`text`) media header kept first in `minf` (ISO/IEC 14496-12 §6.2.3), handler `subt`/`text`. `CodecConfig::is_muxable_in_bmff()` is now true for a subtitle track that has a sample entry; one without (`sample_entry: None`, e.g. a TS DVB/Teletext subtitle) is still rejected with `Error::UnmuxableSubtitleTrack`. DASH/Smooth manifest packagers still return `UnsupportedCodec` for subtitles (no RFC 6381 string is derived). Migrate patterns `CodecConfig::Subtitle { format }` to `{ format, .. }`. Tests: `tests/subtitle_remux.rs` (real ffmpeg `stpp` CMAF round trip, `wvtt` spec vector, entry-less rejection).

#### Fixed (TS demux, #1186)

- **E-AC-3 dependent substreams join their access unit across PES boundaries.** A dependent substream (`strmtyp == 1`) that began a PES used to be emitted as a separate sample; when the programme has dependent substreams (7.1 / Atmos), the final access unit of each PES is now held back until the next PES shows whether a dependent frame follows (ETSI TS 102 366 §E.1.3.1.2: dependent substreams follow their independent substream), and released at the next PES or by `finish()`. Programmes without dependent substreams are untouched (nothing is held). A dependent-frame prefix no longer counts as an access unit commencing, so it cannot move the timeline anchor.
- **E-AC-3 config recovery waits for a complete access unit.** The `dec3` was built from the first independent frame found, so a PES ending right after it produced a config without the dependent substreams (7.1 signalled as 5.1). The probe now resolves only once the first access unit is provably complete (the next independent frame has been seen), joining PES payloads up to 16 syncframes, and at end of input accepts whatever the access unit holds. A leading dependent frame is skipped. Tests: `ts_demux::tests::eac3_*` (independent frames from the committed Dolby capture plus a spec-built dependent frame: no real capture with dependent substreams exists, ffmpeg cannot emit them), `tests/eac3_real_capture.rs` (committed captures and the local archive pinned to the pre-change output).
- **PPS `A -> B -> A` is re-reported (follow-up of #1166).** PPS tracking is keyed by `pic_parameter_set_id` (H.264 §7.3.2.2 / H.265 §7.3.2.3.1): re-sending an id with identical bytes is silent however ids interleave (the #1166 rotation), while an id announced with different bytes is a `TrackUpdated` at every switch; the old "never seen before" test missed the return to an already-seen set. Tests: `ts_demux::tests::pps_a_b_a_on_one_id_is_reported_at_every_switch`, `tests/pps_real_capture.rs` (real captures stay at zero updates). The #1166 rotation tests now rotate real distinct ids (their synthetic "rotation" reused one id with different bytes, which is a replacement per the spec).

#### Fixed (no_std builds, #1181)

- `--features cenc` and `--features sample-aes` now build for bare-metal (`thumbv7em-none-eabi`): `cenc_decrypt` and `sample_aes` were missing `alloc` imports (`ToString`, `format!`) that only the `std` prelude supplied. CI now builds these optional features on the no_std target.

#### Changed (#982)

- `cenc_decrypt`'s private box-navigation copies (`find_box`, `iter_child_boxes`, `iter_top_boxes`, `descend`) are replaced by shared `box_types` helpers (`child_boxes`, `find_child_box`, `descend_boxes`) built on `box_slices`. Behaviour is identical on every real fixture (pinned by an agreement test); the shared helpers also read the true container header length, so a `largesize`-form container's children are found where the old hardcoded 8-byte skip missed them.

#### Security (secrets and identity handling, #1142)

- `KeyMap` and `CencEncryptor` wipe their content keys when dropped (`zeroize`), and `KeyMap` equality compares keys in constant time (`subtle`). Both dependencies are pulled in by the `cenc` feature only. New `tests/key_hygiene.rs` and `tests/secret_debug_guard.rs`.

#### Fixed

- **Over-range version-0 box fields are rejected instead of wrapped (#1129).** `mvhd`/`tkhd`/`mdhd` `creation_time`/`modification_time`/`duration` and `prft` `media_time` above `u32::MAX` now return `Error::FieldOverflow` when `version == 0` (they were written as `value as u32`); `mp4esds` `bufferSizeDB` over 24 bits and `progressive` `stsz` sizes/counts over `u32` likewise. `flv_frame_payloads` (the RTMP payload path) now applies the same 32-bit-millisecond and signed-24-bit `CompositionTime` checks as `FlvMux::package` instead of `as u32`/`as i32`. Tests: `tests/serializer_length_truncation.rs` (also a lexical guard against unchecked `.len() as uN`), `tests/flv.rs::frame_payloads_reject_out_of_range_timestamps`.
- **`trun` with no per-sample fields can no longer drive an unbounded allocation (#1130).** Every entry of such a `trun` is zero bytes wide, so the remaining-bytes bound never limited `sample_count`: four overwritten bytes of `fixtures/mp4/llcmaf/ll_chunked.mp4` made `Fmp4Demux` request a 10 GB `Vec`. `sample_count` is now capped at `MAX_ZERO_WIDTH_TRUN_SAMPLES` (2^20) and `bounded_entry_count` never sizes an up-front allocation from a zero-width count. Found by a hostile-count sweep that is now committed (`tests/hostile_count_sweep.rs`, 21 702 inflated-count parses of every committed MP4 fixture under a 128 MiB live-byte cap) next to the pinned repro (`tests/trun_zero_width_alloc_bound.rs`).
- **ADTS multi-block frames with CRC (#1012).** A frame with `protection_absent == 0` and `number_of_raw_data_blocks_in_frame > 0` now has its `raw_data_block_position[]` table and every `crc_check` word (header-level and per-block, ISO/IEC 13818-7:2004 §6.2 Tables 5-7) stripped; the AAC sample is the concatenated `raw_data_block()` payloads. A self-inconsistent position table degrades to stripping only the header CRC and table instead of rejecting the frame. CRC values are not validated. Layout transcribed in `docs/codec/adts-error-check.md`. Single-block and unprotected frames are byte-identical to before.

#### Changed
- Internal: adapted to `broadcast-common`'s removal of the panicking `Serialize::to_bytes` (tests, examples and docs now use `try_to_bytes`); requires the new `broadcast-common` major (umbrella #1143). `AudioSpecificConfig`'s stack-less fallback no longer panics on an unserializable value (it hands the parser an empty buffer); `smooth` tfxd splice propagates the serializer error.

- **BREAKING (#1159): typed track and program ids.** New `TrackId(u32)` and `ProgramNumber(u16)` newtypes (re-exported at the crate root and from `ir`; `new`/`value`, no silent `From`, `Debug`/`Display` render the bare number and serde is `transparent`, so every golden, JSON and manifest is byte-identical). Every public `track_id: u32` is now `TrackId`: `TrackSpec::{track_id, new}`, `Track::track_id()`, `FragmentTrackData`, `DemuxEvent` track fields (`Sample`, `TrackRemoved`, `TrackAbandoned`, `Discontinuity`, `InputDegraded`), the `push(track_id, ..)` of the segmenters, `RtpInputStream`/`RtpTrackInit`, the ISOBMFF box structs (`tkhd`/`tfhd`/`trex`/`mvhd.next_track_id`/`prft.reference_track_id`), `Error::UnmuxableDataTrack`, the Smooth/DASH track descriptors, and `TrackSpec::program_number` (now `Option<ProgramNumber>`, `with_program(ProgramNumber)`). `TrackId::next()` hands out consecutive ids. Migrate with `TrackId::new(n)` / `.value()`.
- Test: `tests/track_id_baseline.rs` demuxes every committed fixture (and every `.test-streams` capture up to 40 MB when present) and compares per-track ids and sample counts with `tests/golden/track_ids.txt`, captured while `track_id` was a bare `u32`.

#### Changed (BREAKING)

- **BREAKING (#1159, typed PIDs):** `TrackSpec::source_pid` is `Option<Pid>` and `TrackSpec::with_source` takes a `Pid`; `PcrSample::{pid,new}` and `EventProvenance::pid` are `Pid`. `Debug` of the IR now renders a PID as `Pid(0x0100)` instead of `256` (the `sweep_output_golden` hashes normalise this and are unchanged: the demux output itself is identical). `TsMux` reports `Error::InvalidInput` if a track would be assigned an elementary PID above `0x1FFF`.

#### Fixed

- ISOBMFF box sizes past `u32::MAX` use the `size = 1` + 64-bit `largesize` form (ISO/IEC 14496-12 §4.2) instead of silently wrapping (#1147). Every variable-size box writer (sample tables `stts`/`ctts`/`stsc`/`stsz`/`stco`/`co64`/`stss`/`elst`, `stsd` and every sample entry, `hdlr`/`dref`/`url `, `esds`/`avcC`/`hvcC`/`vvcC`, CENC `senc`/`pssh`/`saiz`/`saio`/`schm`/`schi`/`sinf`, `sgpd`/`sbgp`/`subs`, the subtitle entries, `trun`, and the `moov`/`trak`/`mdia`/`minf`/`stbl`/`dinf`/`edts`/`mvex`/`traf`/`moof` containers) now derives its header through one shared helper, and its `serialized_len` counts the extra 8 `largesize` bytes, so the two stay consistent. `OpaqueBox`/`OpaqueChild` switch to `largesize` on their own when their payload no longer fits. Fixed-layout boxes (`mvhd`, `tkhd`, `mdhd`, `vmhd`, `smhd`, `mfhd`, `tfdt`, `prft`, `cslg`, `tenc`, `sidx`) and the brand-list `ftyp`/`styp` and the compact-only sample-entry `OpaqueBox` return a typed `FieldOverflow` error rather than a wrapped size. The typed readers (stsd/dref children, sample-entry config and extra-box scanners, `parse_body` slicing in every typed box) accept the `largesize` form. Output for every real fixture is byte-identical.
- One damaged unit no longer aborts a whole one-shot demux (audit T16, #1144): `PsDemux` resynchronises past a Program Stream pack that fails to parse (via `mpeg_ps::program_stream::scan_packs`; input with no recoverable pack is still an error), `Fmp4Demux` skips a `moof` whose body does not parse, and the batch `RtpDepacketiser` skips a packet with a malformed RTP header, STAP-A, FU-A or AAC AU-header, then carries on with the data after it.

- `WebmDemux` skips a corrupt `Cluster` (resynchronising at the next Cluster ID), `TrackEntry` or (Simple)Block, `Fmp4Demux` skips a `traf` whose `trun` addressing/timing is invalid (committed atomically per `traf`), and `FlvDemux` skips a tag with damaged framing (resynchronising on the next consistent tag chain via StreamID/`DataSize`/following-header checks), a corrupt NAL-length framing, or a corrupt AVC/AAC sequence header; each is reported in `Media::degraded` and the data after it is decoded unchanged. A truncated *final* tag/cluster is still the tail case, not damage. Behaviour change: inputs that used to fail the whole demux on such damage now succeed with `degraded` populated.

#### Added

- `Media::degraded` (`Vec<InputDegradation>`; `Media` is `#[non_exhaustive]`) and `InputDegradation::MalformedUnit { position }` (the enum is `#[non_exhaustive]`): the skipped unit is reported, never silent. `position` is the byte offset (PS pack, fMP4 box) or the packet index in the stream (batch RTP).

## Merged changes since `transmux-v0.26.0`

- transmux: AC-3 core as E-AC-3 independent substream, num_dep_sub counts dependents; real 7.1 stream tests (#1255) (#1257)
- tests: consume deterministic excerpts of the france-tnt/hotbird captures instead of the 540 MB LFS originals (#1263) (#1264)
- captions: mux.js/shaka media (unclear licence) moved to the private submodule; tests read private/fixtures/captions (#1261) (#1262)
- tests: one fixture resolver (public -> private/fixtures -> .test-streams); move local-only captures to the private repo (#1259) (#1260)
- transmux: real wvtt/stpp artifacts from MP4Box and gpac; vtta, typed sample parse, vttc child order (#1249) (#1254)
- transmux: drop an orphan E-AC-3 dependent substream instead of emitting it alone (#1245) (#1246)
- perf(transmux): memchr Annex B start-code scan + bounded unescape in avc_is_i_slice (supersedes #1153, #1154) (#1242)
- transmux: stpp/wvtt subtitle tracks in the DASH MPD and Smooth manifest (#1243) (#1244)
- feat(dvb-si,transmux): China closed caption (GY/T 270-2013) (C1) (#1228)
- feat(transmux): AVS2/AVS3 video and Audio Vivid ISOBMFF boxes (C4) (#1239)
- O1: ordering hygiene — registry tables in wire-value order + ordering guard (#1238)
- Real CEA-608/708 captures: fixtures, tests, and the bugs they exposed (#1220) (#1221)
- transmux: public dash::xs_duration; multimux uses it, copy and test seam removed (#1216) (#1217)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- clock33: pack_marked/unpack_marked for the 5-byte marker-bit timestamp; migrate mpeg-pes, mpeg-ts, st291, transmux (#1208) [stacked on #1207] (#1209)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- transmux: subtitle sample entry re-mux, E-AC-3 dependent substreams across PES, PPS A->B->A (#1186) (#1192)
- DASH: SegmentTimeline absolute t / negative r, AST = presentation origin (#1191)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- fix(transmux,dvb-si): no_std optional features build on thumbv7em (cenc, sample-aes, chrono) + CI step; dedupe cenc box-nav helpers (#1181, #982) (#1184)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit critical-bug verification pass: 72 issues re-verified against main; fixes for #1012 #1022 #1034 #1035 #1056 + regression tests (NO release) (#1180)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Typed ids wave 2: TrackId/ProgramNumber + HLS naming newtypes, Pid inside dvb-si (BREAKING, NO release) (#1178)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
- Audit systemic wave 2: T16 resync, largesize, ttml drop, WHIP/WHEP DELETE/PATCH/ICE restart, mpeg-ts/pes reserved+stuffing preservation, real-capture guards (NO release) (#1175)
