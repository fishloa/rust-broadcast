# Changelog

All notable changes to `transmux` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Added
- **RTCP control packets** (#514): typed `Parse`/`Serialize` for the RFC 3550 §6 set —
  `SenderReport` (PT 200), `ReceiverReport` (201), `SourceDescription` (202, with
  `SdesChunk`/`SdesItem`/`SdesItemType`), `Bye` (203), `App` (204), the shared
  `ReportBlock` (24-bit sign-extended cumulative-lost), `CommonHeader`, and a
  `CompoundPacket` that enforces the §6.1 "first packet must be SR/RR" rule on
  construction, parse, and serialize. Dispatch via `RtcpPacket` / `RtcpPacketType`
  (`name()` + `impl_spec_display!`). RTP companion to `rtp.rs`; not a hub spoke.
- **Public NAL keyframe helper** (#517): `nal_unit_type` / `is_keyframe_nal` /
  `access_unit_is_keyframe` + `NalCodec` (Avc/Hevc/Vvc) — Annex-B and 4-byte
  length-prefixed, spec-cited to H.264/H.265/H.266 §7.3.1. `ts_demux` IDR detection
  now delegates to it (behaviour byte-identical).
- **FLV container spoke** (#513): `FlvDemux` (`Unpackage`) / `FlvMux` (`Package`) ⇄ IR,
  Adobe FLV v10.1 Annex E. H.264 (AVCVIDEOPACKET, avcC seq-header, CompositionTime →
  composition offset) + AAC (AACAUDIODATA, ASC seq-header); reuses `CodecConfig::Avc`/`Aac`.
  ms timescale, lossless timing round-trip. Skips spurious empty sequence-header tags;
  trusts the ASC over contradictory `onMetaData`.

## [0.8.0] — 2026-07-02
### Added
- **Any-to-any hub** (#466): the container-agnostic IR (`Media` / `Track`, thin wrappers
  over `TrackSpec`/`Sample`) + implementations of the new **broadcast-common 8.2.0**
  traits — `CmafMux` / `HlsPackager` (`Package`) and `Fmp4Demux` (`Unpackage`, fMP4 →
  `Media`). Every demux/mux spoke now targets one hub API; `Unpackage`⇄`Package` and
  `Encrypt`⇄`Decrypt` are inverse pairs mirroring `Parse`/`Serialize`. Additive —
  `build_init_segment`/`build_media_segment`/`Segmenter`/`TrackSpec`/`Sample` unchanged.
- Requires broadcast-common ≥ 8.2.0 (the trait definitions).
- **MPEG-H 3D Audio** input (promotion): `Fmp4Demux` now reconstructs
  `CodecConfig::MpegH` from `mha1`/`mha2`/`mhm1`/`mhm2` sample entries (re-parsing
  the `mhaC` record, ISO/IEC 23008-3 §20) — MPEG-H was previously output-only.
  Verified byte-exact against a real Fraunhofer/DASH-IF MPEG-H bitstream. This
  makes the codec set demux+mux complete across the hub.
- **VVC / H.266** (#474): `CodecConfig::Vvc` + `vvc1`/`vvcC` (VvcDecoderConfiguration-
  Record as a FullBox, byte-exact Parse/Serialize) mirroring HEVC. `decode_vvc_sps`
  (H.266 §7.3.2.4/§7.3.3.1) recovers dims/profile/tier/level; `Fmp4Demux` reconstructs
  `Vvc` from `vvc1`/`vvi1`; `CmafMux` emits `vvc1`. Byte-verified against a real
  vvenc+ffmpeg fixture. vvcC layout doc grounded in the FFmpeg reference (§11).
- **VP8 + Vorbis** (WebM): `CodecConfig::Vp8` (dims from the RFC 6386 key-frame
  header) + `CodecConfig::Vorbis` (channels/sample_rate + verbatim `CodecPrivate`
  from the Vorbis I identification header). `WebmDemux` now covers all four WebM
  codecs (VP9/VP8 video, Opus/Vorbis audio). WebM-native (no mp4 mux path).
- **MPEG-2 video (H.262) + MPEG-1/2 audio (MP1/2/3)** codecs: `CodecConfig::Mpeg2Video`
  + `MpegAudio`. `Fmp4Demux` reconstructs `mp4v`/esds (OTI 0x60–0x65) → `Mpeg2Video`
  (dims from the in-band sequence header, ISO 13818-2 §6.2.2.1) and `mp4a` OTI
  0x69/0x6B → `MpegAudio` (layer/rate/channels from the frame header, ISO 11172-3
  §2.4.1.3). `TsDemux` handles `stream_type` 0x02/0x03/0x04 — the classic broadcast
  pair now round-trips through both fMP4 and TS. New `Mp4vSampleEntry` +
  `MpegAudioLayer` enum.
- **`CodecConfig::Hevc`** + **complete `Fmp4Demux` codec-config reconstruction**
  (#467 codec tail): `Fmp4Demux` now reconstructs the IR codec config for every
  codec the crate can output — `hvc1`/`hev1`→`Hevc`, `av01`→`Av1`, `vp09`→`Vp9`,
  `Opus`→`Opus`, `fLaC`→`Flac`, `dac3`→`Ac3`, `dec3`→`Eac3`, `ddts`→`Dts` (plus
  the existing `avc1`/`mp4a`) — was previously deferred to AVC/AAC only. New
  `Hevc` variant muxes to an `hvc1`+`hvcC` sample entry. Every codec round-trips
  byte-identically (config box + coded samples) via fragmented-mp4 fixtures.
  Unknown sample entries skip the track rather than erroring.
- **RTP spoke** (#469): `RtpPacketizer` (`Package`) and `RtpDepacketizer`
  (`Unpackage`) — de/packetize the `Media` IR ⇄ RTP. H.264 single-NAL / STAP-A
  (SPS+PPS) / **FU-A** fragmentation at MTU (RFC 6184), AAC `AAC-hbr` AU-headers
  (RFC 3640), RTP fixed header with marker/seq/90 kHz timestamps (RFC 3550), and
  SDP generation (`rtpmap`/`fmtp` with `sprop-parameter-sets` + AAC `config`,
  RFC 4566). Round-trips byte-identically through the real demuxed NALs; no new
  dependency (hand-rolled base64/hex). New `RtpMediaKind` enum.
- **Microsoft Smooth Streaming output** (#473): `SmoothPackager` implements the hub
  `broadcast_common::Package` trait — `Media` IR → a Smooth client Manifest
  (`SmoothStreamingMedia`>`StreamIndex`>`QualityLevel`+`c`) + Smooth fragment-MP4
  fragments (`moof` with the `tfxd` `uuid` box + `mdat`). FourCC `H264`/`AACL`,
  `CodecPrivateData` = start-code SPS+PPS / raw ASC; TimeScale 10 MHz. New
  `TfxdBox` uuid type + `SmoothStreamType` enum. Fragments round-trip losslessly
  via `Fmp4Demux`. Cites [MS-SSTR] + ISO/IEC 14496-12.
- **Low-latency DASH** (#461): `LlSegmenter` (chunked CMAF — each segment
  subdivided into `moof`+`mdat` chunks, first chunk `styp`-prefixed, contiguous
  `tfdt`/sequence numbers) + `LlDashPackager` (LL-DASH MPD: `type="dynamic"`,
  `SegmentTemplate@availabilityTimeComplete="false"` + `@availabilityTimeOffset`,
  `<ServiceDescription><Latency>`). Both `impl broadcast_common::Package`. Chunks
  concatenate losslessly to a whole segment (verified via `Fmp4Demux`). ISO/IEC
  14496-12 chunk structure + ISO/IEC 23009-1 / DASH-IF LL IOP signalling.
- **WebM / Matroska demuxer** (#471): `WebmDemux` implements the hub
  `broadcast_common::Unpackage` trait — WebM (EBML) → `Media` IR, the fourth hub
  input (TS / fMP4 / MPEG-PS / WebM). Hand-written EBML/VINT tree walker
  (RFC 8794 framing, RFC 9559 element IDs); maps `V_VP9`→`CodecConfig::Vp9`
  (synthesised `vpcC`) and `A_OPUS`→`CodecConfig::Opus` (`dOps` from the CodecPrivate
  `OpusHead`); (Simple)Block timestamps in a millisecond IR timescale, Opus
  pre-skip codec delay applied. Gated against a per-frame **size-column** ffprobe
  oracle + a CMAF output round-trip (vp09/`vpcC` + Opus/`dOps`).
- **CENC decrypt** (#465): `CencDecryptor` implements the hub
  `broadcast_common::Decrypt` trait — unprotect a CENC (`cenc` / AES-128-CTR) fMP4
  given the content key. Reuses the existing `cenc.rs` box parsers
  (`tenc`/`senc`/`saiz`/`saio`/`sinf`/`frma`); subsample-aware (clear ranges
  skipped, CTR streams across protected ranges), IV left-justified to 16
  (ISO/IEC 23001-7 §10.1). AES via RustCrypto `aes`/`ctr` behind an optional
  `cenc` feature (`--no-default-features` drops it). `cbcs` documented
  unsupported. Verified by decrypting a real ffmpeg-encrypted fixture to
  byte-identical cleartext (+ wrong-key negative). New `CencScheme` enum.
- **MPEG-2 Program Stream demuxer** (#470): `PsDemux` implements the hub
  `broadcast_common::Unpackage` trait — MPEG-2 PS (`.ps`/VOB-style) → `Media` IR,
  the third hub input alongside TS and fMP4. Parses packs/system-header via
  `mpeg-ps`, maps elementary streams by `stream_id` (H.264 0xE0–0xEF; AC-3 in
  `private_stream_1` 0xBD), reassembles PES across packs, recovers H.264 `avcC`
  (in-band SPS/PPS) + AC-3 `dac3` (syncframe BSI). Gated against ffprobe timing +
  byte-identical `avcC`/video-NAL oracles (ISO/IEC 13818-1 §2.5).
- **Classic HLS (MPEG-2 TS segments)** (#472): `TsHlsPackager` implements the hub
  `broadcast_common::Package` trait (`Output = TsHlsOutput { segments, playlist }`),
  segmenting the `TsMux` output at keyframe boundaries into independently-decodable
  `.ts` segments (each re-emits PAT+PMT + a keyframe-aligned PES) plus an RFC 8216
  HLS media playlist (`#EXTINF` + `.ts` URIs, no `#EXT-X-MAP`). Per-segment base DTS
  keeps one monotonic timeline across boundaries — the concatenated segments
  round-trip losslessly through `TsDemux`.
- **DASH `.mpd` output** (#464): `DashPackager` implements the hub
  `broadcast_common::Package` trait (`Output = String`), emitting a DASH MPD
  (ISO/IEC 23009-1) alongside the HLS playlists from one CMAF —
  MPD→Period→AdaptationSet(`video/mp4`,`audio/mp4`)→Representation with
  `SegmentTemplate` (`$Number$`/`$RepresentationID$`). `codecs=` from the crate's
  own rfc6381 builders; `@width`/`@height` from the SPS, `@audioSamplingRate` from
  the ASC, integer `@bandwidth`; VOD (`static`) + `dynamic` (live). Dependency-free
  XML writer; integer-only arithmetic (`no_std`-clean).
- **fMP4/CMAF repackage** (#462): `Repackage` + `Media` IR transforms —
  `select_tracks` (track subset), `trim` (half-open presentation window, snapped
  back to the preceding sync sample per CMAF ISO/IEC 23000-19 §7.3.2.3), and
  resegment (via the existing `Segmenter`) to a new target duration. Composes
  demux → transform → mux with no new box parsers; lossless (byte-identical coded
  samples across identity repackage, verified against the `TsDemux` oracle).
- **TS muxer** (#460): `TsMux` implements the hub `broadcast_common::Package`
  trait — `Media` IR → a whole-188-byte-packet MPEG-2 TS, the byte-level inverse
  of `TsDemux`. Emits PAT→PMT (CRC-32/MPEG-2 sections), `stream_type` per codec,
  PCR on the first video PID; per-sample PES (PTS always, DTS when differing),
  video length-prefixed→Annex B with SPS/PPS re-injection on keyframes, AAC
  re-wrapped in ADTS from the `esds` ASC (ISO/IEC 13818-1 §2.4.3/§2.4.4). With
  `TsDemux` this closes the loop: `{fMP4/CMAF} → IR → {TS}` and byte-fidelity
  `TS → IR → TS` round-trips.
- **Progressive MP4 output** (#463): `ProgressiveMux` implements the hub
  `broadcast_common::Package` trait, muxing the `Media` IR into a single-file,
  non-fragmented `.mp4` (ftyp + one moov with full `stbl` sample tables + one
  mdat) — the VOD/download counterpart to `CmafMux`. Builds `stts`/`ctts`/
  `stsc`/`stsz`/`stco`|`co64`/`stss` from the sample stream (ISO/IEC 14496-12
  §8.5–§8.7); `faststart: bool` writes moov-before-mdat via a two-pass
  chunk-offset fixup. Adds typed `co64`/`stss` boxes. Gated against the ffmpeg
  faststart ref mp4 (byte-identical video samples + `avcC`).
- **TS demuxer** (#467, partial — H.264 + AAC): `TsDemux` implements the hub
  `broadcast_common::Unpackage` trait, turning MPEG-2 TS bytes into the `Media`
  IR — the input side of the any-to-any hub, so `{TS} → IR → {any}` works
  in-crate. Follows PAT→PMT, maps `stream_type`→codec, per-PID PES reassembly
  (PTS/DTS 33-bit unwrap), and recovers codec config from in-band parameters
  (H.264 SPS/PPS → `avcC`; AAC ADTS → ASC/`esds`; AC-3/E-AC-3 syncframe →
  `dac3`/`dec3`). Verified against ffprobe timestamp and ffmpeg `-c copy` byte
  oracles. HEVC/DTS are recognised in the PMT but not yet emitted (no IR
  HEVC-video variant / DTS-ES parser) — tracked on #467.
- `mpeg-ts` / `mpeg-pes` are now regular dependencies (were dev-deps).

### Fixed
- **ADTS `channel_configuration` decode** (`aac_asc`): the 3-bit field was split
  wrong (`byte2[0]<<3 | byte3[7:5]`); the correct ISO/IEC 13818-7 §6.2 layout is
  `byte2[0]<<2 | byte3[7:6]`. Build+parse were self-consistent so round-trip
  tests passed, but a real mono ADTS stream was misread as stereo. Both
  directions corrected.

### Fixed
- `TsDemux` now decodes AVC `width`/`height` from the in-band SPS (was left at 0).

## [0.6.0] — 2026-07-02
### Added
- **DTS** fMP4 carriage (#437, ETSI TS 102 114 §E.2): `dtsc`/`dtsh`/`dtsl`/`dtse`
  sample entries + `ddts` (DTSSpecificBox — DTSSamplingFrequency, max/avg bitrate,
  pcmSampleDepth, FrameDuration, StreamConstruction, channel layout, …) + a
  `CodecConfig::Dts` variant + `rfc6381()`. Typed Parse/Serialize with a spec-vector
  byte-exact round-trip + `build_init_segment` moov round-trip (ffmpeg has no `ddts`
  encoder, so the real-fixture gate is deferred).
### Changed
- `hvcC` value-verified against the ISO 14496-15:2017 §8.3.3.1 text (recovered via
  marker OCR of the scanned edition), matching FFmpeg movenc + the byte-exact oracle
  (#394). Docs only.

## [0.5.0] — 2026-07-01
### Added — fMP4 gap tier (real codecs + container completeness)
- **Codec sample entries + config boxes** (container-level; header parse only, samples
  pass through opaque): AV1 (`av01`/`av1C`, #436), VP9 (`vp09`/`vpcC`), Opus (`Opus`/
  `dOps`), FLAC (`fLaC`/`dfLa`) (#437), AC-4 (`ac-4`/`dac4`, #431), MPEG-H 3D Audio
  (`mha1`/`mhm1`/`mhaC`, #433), and HE-AAC SBR/PS AudioSpecificConfig signaling →
  `mp4a.40.5`/`mp4a.40.29` (#432). Each with a `CodecConfig` variant + `rfc6381()`.
- **CENC per-sample encryption** (#429, ISO/IEC 23001-7): `tenc`/`senc`/`saiz`/`saio`/
  `pssh` + `sinf`/`frma`/`schm`/`schi` + `enca`/`encv` sample entries.
- **Subtitle carriage** (#430, ISO/IEC 14496-30): `stpp` (TTML/IMSC) + `wvtt` (WebVTT +
  `vttC`/`vtte`/`vttc`/`payl`/`sttg`/`iden`).
- **Sample-entry extensions** (#434): `colr` (nclx — HDR/wide-gamut), `pasp`, `clap`.
- **Timing / grouping** (#435): `prft` (ProducerReferenceTimeBox), `sgpd`/`sbgp`
  (sample groups incl. `roll`), `subs` (sub-sample info).
- **avcC/hvcC value-verification** (#441/#394): byte-exact round-trip against real
  ffmpeg-muxer boxes; avcC now grounded on the text-layer 14496-15:2012.

All new boxes are typed with symmetric `Parse`/`Serialize` and byte-exact round-trip
tests against real ffmpeg-authored fixtures (config-box oracles = ffmpeg's own muxer
output); MPEG-H uses a spec vector (no redistributable fixture/encoder).

## [0.4.1] — 2026-07-01
### Changed
- Value-verified the `esds` / `mp4a` descriptor layout against the vendored
  ISO/IEC 14496-1 §7.2.6 (transcribed to `docs/codec/es-descriptor-14496-1.md`)
  and added a **byte-exact round-trip test on a real ffmpeg-authored `esds`**
  (AAC-LC, 4-byte-expanded descriptor sizes, real max/avg bitrates). No API change.

## [0.4.0] — 2026-07-01
### Added
- AC-3 / E-AC-3 audio in the fMP4 path (ETSI TS 102 366 Annex F):
  - `Ac3SpecificBox` (`dac3`) + `Ec3SpecificBox` (`dec3`) — typed config boxes with
    Parse + symmetric Serialize + round-trip.
  - AC-3 / E-AC-3 syncframe BSI parsers (`0x0B77` syncword → `syncinfo()`+`bsi()` /
    E-AC-3 syncframe): build a `dac3`/`dec3` from an elementary stream.
  - `CodecConfig::Ac3` / `Eac3` + `Ac3SampleEntry` / `Ec3SampleEntry`
    (`SampleEntryVariant::Ac3`/`Ec3`), wired through `build_init_segment` to emit
    `ac-3` / `ec-3` sample entries.
  - `rfc6381()` → `"ac-3"` / `"ec-3"`.
- Gate `tests/dolby.rs`: parses real ffmpeg-encoded AC-3/E-AC-3 fixtures and asserts
  the built `dac3`/`dec3` bytes match ffmpeg's own MP4-muxer output byte-for-byte.

## [0.3.0] — 2026-07-01
### Added
- SPS/VPS decode + RFC 6381 codec strings, so transmux no longer needs an external
  parser (e.g. `h264_reader`) to learn what it must put in an `avcC`/`hvcC`:
  - `AvcSps::decode() -> AvcSpsInfo` (ITU-T H.264 §7.3.2.1.1): profile_idc,
    constraint byte, level_idc, chroma_format_idc, bit-depths, `frame_mbs_only`,
    and coded width·height after frame cropping (chroma-dependent CropUnit +
    interlaced height factor). Handles the high-profile branch and skips
    SPS-embedded scaling lists.
  - `HevcNalUnit::decode_sps() -> Option<HevcSpsInfo>` (ITU-T H.265 §7.3.2.2 +
    profile_tier_level §7.3.3): general profile/tier/level, compatibility +
    constraint flags, chroma/bit-depth, conformance-window-cropped dimensions.
  - RFC 6381 strings: `AvcSps::rfc6381()` → `avc1.PPCCLL`;
    `HevcNalUnit::rfc6381()` → `hvc1.…`; `AudioSpecificConfig::rfc6381()` →
    `mp4a.40.<AOT>`. Plus a public `bitreader` (Exp-Golomb + emulation-prevention
    unescape) and `sps` module.
- Gate `tests/codec_config.rs`: decodes real ffmpeg-encoded fixtures across the
  full H.264 profile matrix (baseline/main/high/high10/high422/high444 + interlaced
  + 1080-cropped) and HEVC main/main10, asserting every field against a
  `trace_headers` oracle, plus a scaling-list spec vector and an avcC round-trip.

## [0.2.0] — 2026-07-01
### Added
- `Segmenter`: a stateful streaming CMAF segmenter wrapping `build_init_segment` /
  `build_media_segment`. Feed coded samples in decode order (`push`), pull finished
  media segments (`take_ready`), and `flush` the trailing partial segment at
  end-of-stream. Segments are cut on the anchor track (first video track, else the
  first track) at a keyframe once the target duration is reached, so every video
  segment begins on a random-access point and the concatenation of all segments
  carries the full stream with contiguous per-track `tfdt`. This is the streaming
  state machine `build_media_segment` (a batch box builder) deliberately omits;
  it lets a live remuxer produce a CMAF track without hand-rolling segment cutting.
- `Error::InvalidInput(&'static str)` for caller-precondition violations (empty
  track list, non-positive segment duration, duplicate `track_id`, unknown
  `track_id` on `push`).

## [0.1.0] — 2026-07-01
### Added
- End-to-end `tests/ts_to_cmaf.rs`: demux a real H.264+AAC MPEG-TS
  (`fixtures/ts/h264_aac.ts`), synthesize `avcC` from the stream's SPS/PPS and
  `esds` from the AAC ADTS header, build init + media segments, then re-parse —
  asserting **byte-identical** avcC SPS/PPS + esds AudioSpecificConfig fidelity,
  75 video access units, the computed ADTS frame count, and first-sample
  round-trip. Closes the literal "TS in → CMAF out" acceptance (#408).
- HLS playlist generation (RFC 8216): `MediaPlaylist` / `MasterPlaylist` +
  `Variant` / `MediaSegment` with `to_m3u8()` emitters for the CMAF segments
  produced by the remux pipeline (`#EXTM3U` / `EXT-X-VERSION` / `TARGETDURATION` /
  `MEDIA-SEQUENCE` / `EXTINF` / `ENDLIST`; master `EXT-X-STREAM-INF` with
  bandwidth/codecs/resolution; `extra_tags` for `EXT-X-DATERANGE`). Generated
  playlists validate clean through `media-doctor::check_playlist`.
- Segment-level boxes: `FileTypeBox` (`ftyp`), `SegmentTypeBox` (`styp`),
  `MediaDataBox` (`mdat`, 32-bit + 64-bit largesize).
- Typed `MovieExtendsBox` (`mvex`) + `TrackExtendsBox` (`trex`) on `MovieBox`
  (fragmented-init movies); byte-identical round-trip of a real fragmented moov.
- Annex B ↔ length-prefixed NAL conversion (`annexb_to_length_prefixed`,
  `length_prefixed_to_annexb`, iterators).
- Samples-in TS→CMAF remux pipeline: `build_init_segment` (ftyp + fragmented-init
  moov with empty sample tables + mvex/trex) and `build_media_segment`
  (styp + moof{tfhd,tfdt,trun} + mdat) with `data_offset` computed from the
  finished `moof` and signed composition offsets. `CodecConfig` / `TrackSpec` /
  `Sample` / `FragmentTrackData` API.
- Typed init-segment `moov` box tree (`MovieBox`/`mvhd`/`trak`/`tkhd`/`mdia`/
  `mdhd`/`hdlr`/`minf`/`stbl`/`stsd` + sample descriptions) with byte-identical
  round-trip.
- `avcC`/`hvcC` config boxes, `esds`/ES_Descriptor, AAC AudioSpecificConfig +
  ADTS, movie-fragment boxes (`moof`/`mfhd`/`traf`/`tfhd`/`tfdt`/`trun`),
  timing boxes (`stts`/`ctts`/`stsc`/`stsz`/`stco`/`elst`/`sidx`), and generic
  box framing.

_Unreleased — `transmux` has not yet been published to crates.io._
