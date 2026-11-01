# transmux

[![Crates.io](https://img.shields.io/crates/v/transmux.svg)](https://crates.io/crates/transmux)
[![docs.rs](https://img.shields.io/docsrs/transmux)](https://docs.rs/transmux)

Any-to-any media **container** muxing hub. Demux TS, fMP4/CMAF, progressive MP4, MPEG-PS, WebM/Matroska, FLV or RTMP into one neutral in-memory IR (`Media` / `Track` / `Sample`), and mux from it to CMAF/fMP4, progressive MP4, TS, Matroska, FLV, RTMP, HLS (CMAF, TS and low-latency), DASH (including low-latency) or Smooth Streaming.

## What it does / what it does not

**Does:** parse containers into one IR and write that IR into any other container, so every input composes with every output (one documented exception below). Around that it provides streaming segmenters (CMAF, LL-HLS parts, TS-HLS, LL-DASH chunks), IR timeline transforms (rebase, offset, discontinuity gap, splice and concat for SSAI, trick-play track derivation), CENC and CBCS decrypt and encrypt with DASH and HLS DRM signalling, HLS Sample-AES, RTP and RTCP packetising with SDP helpers, KLV metadata, and a structural fMP4/CMAF conformance validator. A `cli`-gated `transmux` binary wires it into a packager.

**Does not:** encode or decode media. It parses codec *config headers* (SPS/PPS/VPS, AAC `AudioSpecificConfig`, AC-3 BSI, EBML and box config) only far enough to build container boxes and derive metadata (dimensions, profile, sample rate, the `codecs=` string). Compressed samples are opaque `bytes::Bytes` copied through byte for byte. It also does not do ABR ladders, transcoding, DRM licence acquisition, or HLS/DASH delivery (see `hls-runtime` and `multimux`). HLS *playlist syntax* lives in `broadcast-hls`; transmux's `ts_hls` and `ll_hls` segmenters produce container bytes and playlist values and depend on it.

**The one exception to any-to-any:** WebM's VP8 and Vorbis tracks have no ISOBMFF sample entry in this crate, so they cannot enter the fMP4/CMAF, progressive-MP4, DASH, HLS or Smooth mux path (`Error::UnsupportedCodec`); only `WebM -> WebM` or Matroska round-trips them. Other limits per output are in the matrix below.

## Standards supported

Editions: where a spec is vendored in `private/specs/` the edition is verified against `private/specs-md/INDEX.md`; otherwise the edition is the one the source module cites, marked "as cited".

| Standard | Edition | Scope here | Status | Notes |
|---|---|---|---|---|
| ISO/IEC 14496-12 ISO base media file format | 2015 as cited in `box_types` (not vendored) | Box framing (incl. 64-bit `largesize`, `uuid`), `moov`/`trak`/`stbl`, fragments (`moof`/`traf`/`trun`/`sidx`/`mfra`), `emsg`, `prft`, sample groups, `srat` | Full for the listed boxes | Every box has `Parse` and `Serialize` with byte-identical round trip |
| ISO/IEC 14496-1 / -14 / -15 / -30 | As cited (not vendored) | `esds`, AVC/HEVC/VVC config boxes, WebVTT and TTML sample entries | Full for the listed codecs | |
| ISO/IEC 13818-1 / ITU-T H.222.0 | 08/2023 (vendored) | PAT/PMT, PES, PCR, 33-bit timestamp unroll, stream types, ES_info passthrough (CA descriptors deliberately dropped on re-mux) | Full for demux and mux of the carriable codecs | |
| ISO/IEC 23009-1 MPEG-DASH | Not vendored; edition unverified | Static and dynamic MPD, `SegmentTemplate` (`$Number$`, `$Time$`), `SegmentTimeline`, `UTCTiming`, `InbandEventStream`, MPD parse | Partial | Built from DASH-IF and TS 103 285 text |
| ETSI TS 103 285 DVB-DASH, DASH-IF IOP | TS 103 285 v1.4.1 (vendored); IOP v4.3 as cited | Subtitle `AdaptationSet`, timeline resolution | Partial | |
| ISO/IEC 23001-7 Common Encryption | As cited (not vendored) | `cenc` (AES-CTR) and `cbcs` (AES-CBC pattern); `senc`/`saiz`/`saio`/`tenc`/`pssh`/`sinf` | Full for `cenc` and `cbcs` | `cens` and `cbc1` are not offered (`CencScheme` has two variants) |
| Apple HLS: RFC 8216 and `draft-pantos-hls-rfc8216bis` | RFC 8216; draft `-22` (vendored) | CMAF-HLS, TS-HLS, LL-HLS parts and preload hint, Sample-AES, `EXT-X-KEY` signalling | Partial | Playlist model in `broadcast-hls` |
| Microsoft [MS-SSTR] Smooth Streaming | Vendored `ms_sstr_smooth_streaming_protocol` | Manifest, fragments, `tfxd`, text streams, parse | Partial | H.264 video and AAC-LC audio (and text) only |
| RFC 3550 RTP/RTCP, RFC 6184 H.264, RFC 3640 AAC, RFC 6597 KLV, RFC 8866 SDP | RFC editions as numbered (3550, 3551, 6184 vendored) | Packetise and depacketise, SDP fmtp to codec config | Partial | Low-delay H.264 and one-AU-per-packet AAC for the live depacketiser |
| RFC 9559 Matroska, RFC 8794 EBML | Vendored | WebM and Matroska demux, MKV mux, laced blocks | Partial | |
| Adobe FLV v10.1 and RTMP 1.0 | Vendored | FLV demux and mux, RTMP chunk stream and AMF0 transport | Partial | H.264 and AAC only; `FlvDemux` silently skips other tracks |
| ITU-T H.264, H.265, H.266, H.262 | As cited (H.26x vendored) | SPS/PPS/VPS decode, `avcC`/`hvcC`/`vvcC`, NAL classification, keyframe detection | Config headers only | No slice or bitstream decode |
| ETSI TS 102 366 AC-3/E-AC-3, TS 103 190 AC-4, TS 102 114 DTS, ETSI TS 101 154 | TS 102 366 v1.4.1, TS 103 190-1 v1.4.1, TS 102 114 v1.6.1, TS 101 154 v2.10.1 (vendored) | `dac3`/`dec3`/`dac4`/`ddts`, BSI parse, E-AC-3 dependent substreams | Config headers only | |
| GY/T 420-2025 Annex A (AVS3 video and Audio Vivid), AVS2 | As cited | `av3c`, `lavc`/`la2c`, `av2c`, `dca3`, sample groups `telg`/`lrap`/`a3lg` | Partial | `lidx`, `av3l` and the `av3a` init-segment wiring are not modelled |
| SMPTE ST 336 / MISB ST 0601 | ST 336:2017 (vendored) | `KlvItem`, `UasLocalSet`, KLV over RTP | Full for the listed items | |
| ITU-T H.265 Annex D (SEI 137/144), SMPTE ST 2086, SMPTE ST 2094-40 (HDR10+), Dolby Vision | As cited (H.265 vendored; ST 2094-40 and Dolby specs not vendored) | HDR10 static metadata (`HdrStaticMetadata`, `colr`/`mdcv`/`clli` boxes), HDR10+ dynamic metadata (`Hdr10PlusMetadata`), Dolby Vision `dvcC`/`dvvC` records and the TS `DOVI_video_stream_descriptor`, with the RFC 6381 `dvh1`/`dvhe` strings | Partial | Signalling and carriage only; no tone mapping, and Dolby Vision RPU payloads stay opaque |
| ETSI EN 300 743 / EN 300 706 DVB subtitles and teletext | Vendored | Recovered as opaque `Data` tracks on TS demux | Typed-opaque | Interpretation is in `dvb-subtitle` and `dvb-si` |

## Input x output matrix

Rows are the demux (`Unpackage`) inputs, columns the mux (`Package`) outputs. A cell is "yes" when every codec the IR can carry from that input reaches that output, or lists the codecs that are rejected or dropped.

| Input \ Output | CMAF/fMP4 | Progressive MP4 | TS | MKV | CMAF-HLS / LL-HLS | TS-HLS | DASH / LL-DASH | Smooth | FLV / RTMP |
|---|---|---|---|---|---|---|---|---|---|
| MPEG-2 TS (`TsDemux`, `StreamingTsDemux`) | yes | yes | yes | AVC, HEVC, AAC, AC-3, E-AC-3 | yes | carriable TS codecs | yes | AVC + AAC | AVC + AAC |
| fMP4/CMAF (`Fmp4Demux`) | yes | yes | carriable TS codecs only (others dropped, not an error) | listed MKV codecs | yes | carriable TS codecs | yes | AVC + AAC (+ text) | AVC + AAC |
| Progressive MP4 (`ProgressiveDemux`) | yes | yes | carriable TS codecs | listed MKV codecs | yes | carriable TS codecs | yes | AVC + AAC | AVC + AAC |
| MPEG-PS (`PsDemux`) | yes | yes | yes | listed MKV codecs | yes | carriable TS codecs | yes | AVC + AAC | AVC + AAC |
| WebM / Matroska (`WebmDemux`) | not VP8, Vorbis | not VP8, Vorbis | carriable TS codecs | yes (round-trips VP8 and Vorbis) | not VP8, Vorbis | carriable TS codecs | not VP8, Vorbis | AVC + AAC | AVC + AAC |
| FLV (`FlvDemux`) | yes | yes | yes | yes | yes | yes | yes | yes | yes |
| RTMP (`RtmpDemux`) | yes | yes | yes | yes | yes | yes | yes | yes | yes |

Notes. The TS-carriable codecs are AVC, HEVC, MPEG-2 video, AAC, MPEG-1/2 audio, AC-3, E-AC-3, DTS, MPEG-H and opaque `Data`; VVC, AV1, VP9, Opus, FLAC, AC-4, subtitles, VP8 and Vorbis have no TS mapping in this crate, and `TsMux` drops such a track rather than erroring. The MKV mux carries AVC, HEVC, VP9, VP8, AV1, AAC, Opus, Vorbis, AC-3 and E-AC-3. `FlvMux` and `RtmpMux` carry only AVC video and AAC audio and reject any other codec with `FlvError::UnsupportedCodec`; `SmoothPackager` likewise rejects everything but H.264, AAC-LC and text. `FlvDemux` skips non-AVC video and non-AAC audio with no track and no error, whereas `TsDemux` never drops a stream (unknown stream types become opaque `Data` tracks). Where a cell names only some codecs, whether a track outside that set is rejected or dropped is per output (for TS it is dropped); this README does not enumerate it for MKV. Live producers also exist: `Segmenter` (CMAF), `LlHlsSegmenter`, `StreamingTsHlsSegmenter`, `LlSegmenter` / `LlDashPackager`.

MPEG-H over TS: the synthesized `MPEG-H_3dAudio_descriptor` (H.222.0 Table 2-121) writes `interactivityEnabled` = 0 and `compatibleProfileSetsPresent` = 1 (no compatible-set list) because the IR carries neither. The descriptor's 6-bit `referenceChannelLayout` is taken from the `mhaC` byte; a value above `0x3F` cannot be represented there and is written as `0` (unspecified) instead of failing the mux.

## Codec matrix

Header parse and the container box only; no encode or decode.

| Codec | Sample entry and config | RFC 6381 string | Where it flows |
|---|---|---|---|
| H.264 / AVC | `avc1`/`avc2`/`avc3`/`avc4`, `avcC`, SPS/PPS decode (profile, level, chroma, bit depth, dimensions, interlace) | `avc1.PPCCLL` | all outputs |
| H.265 / HEVC | `hvc1`/`hev1`, `hvcC`, VPS/SPS/profile_tier_level | `hvc1.…` | all but Smooth and FLV |
| H.266 / VVC | `vvc1`/`vvi1`, `vvcC`, SPS | `vvc1.…` | ISOBMFF outputs, not TS or MKV |
| MPEG-2 video (H.262) | `mp4v`, `esds`, sequence header | `mp4v.61` | TS, ISOBMFF outputs |
| AV1, VP9 | `av01`/`av1C`; `vp09`/`vpcC` | `av01.…` | ISOBMFF outputs and MKV, not TS |
| AVS2 / AVS3 video | `avs3`/`lav3`/`avst`/`lav2`, `av3c`, `lavc`/`la2c`, `av2c` | none | box level only |
| VP8 | WebM `V_VP8` (keyframe header dimensions) | none | WebM and MKV only |
| AAC, HE-AAC (SBR/PS) | `mp4a`, `esds`, `AudioSpecificConfig`, ADTS | `mp4a.40.N` (`.5`, `.29` for HE) | all outputs |
| AC-3, E-AC-3 | `ac-3`/`dac3`, `ec-3`/`dec3`; dependent substreams (7.1, Atmos) | `ac-3`, `ec-3` | TS, ISOBMFF, MKV |
| AC-4 | `ac-4`/`dac4` | `ac-4` | ISOBMFF only |
| DTS | `dtsc`/`dtsh`/`dtsl`/`dtse`, `ddts` | `dtsc` | TS, ISOBMFF |
| MPEG-1/2 audio | `mp4a`, `esds`, frame header | `mp4a.6B`, `mp4a.69` | TS, ISOBMFF |
| MPEG-H 3D Audio | `mha1`/`mhm1`, `mhaC` | none | TS, ISOBMFF |
| Opus, FLAC | `Opus`/`dOps`, `fLaC`/`dfLa` | none | ISOBMFF and (Opus) MKV |
| Vorbis | WebM `A_VORBIS`, id header | none | WebM and MKV only |
| Audio Vivid / AVS3 audio | `dca3` (config only), `av3a` entry not wired into init segments | none | box level only |
| Subtitles WebVTT / TTML | `wvtt` / `stpp` carried in `CodecConfig::Subtitle`, with `sthd`/`nmhd`; `VttCueBox`, `VttAdditionalTextBox`, `parse_wvtt_sample` | `wvtt`, `stpp` | CMAF, DASH (`contentType="text"`), Smooth (`Type="text"`); a TS DVB or teletext track stays opaque `Data` and is unmuxable as a subtitle |
| CEA-608/708 | In-band SEI `cc_data` extraction (ATSC and GY/T 270) | n/a | interpretation lives in `cc-data` and `caption-convert` |
| Opaque `Data` | TS PES or section carriage, `stream_type` and `ES_info` kept verbatim | n/a | TS round trip |

**Encryption and DRM:** `cenc` and `cbcs` decrypt (`CencDecryptor`, `KeyMap`) and encrypt (`CencEncryptor`, which fills `Track::encryption` so `protect_init_segment` and `protect_media_segment` emit `sinf`/`senc`/`saio`/`saiz`). `pssh` init data for Widevine (protobuf), PlayReady (PRO and WRMHEADER, `std`) and FairPlay (`skd://`) in `drm`. DASH `ContentProtection` is derived from `Track::encryption`; HLS `EXT-X-KEY` comes from `broadcast_hls::cenc_ext_x_key`. Sample-AES and whole-segment AES-128 are in `sample_aes` (feature `sample-aes`).

## Features

| Feature | Default | Description |
|---|---|---|
| `std` | yes | `std::error::Error` impls and every XML path (DASH and LL-DASH MPD generate and parse, Smooth, PlayReady WRMHEADER on `quick-xml`), SDP generation (`sdp-types`), `base_url` (`url`), `jiff` durations. Without it the crate is `no_std` + `alloc`. |
| `serde` | yes | `Serialize` on box types and IR types. |
| `cenc` | yes | CENC and CBCS crypto (`aes`, `ctr`, `cbc`, with `zeroize` and constant-time key compare). DRM signalling does not need it. |
| `sample-aes` | no | HLS Sample-AES and whole-segment AES-128; implies `cenc`. |
| `cli` | no | The `transmux` binary (`clap`, `container-probe`; implies `std`). |

## Quick start

Demux a real H.264 plus AAC broadcast capture once, then mux the same IR to CMAF, a DASH manifest and back to TS:

```rust
use broadcast_common::{Package, Unpackage};
use transmux::{CmafMux, DashPackager, Media, TsDemux, TsMux};

let ts = std::fs::read("../fixtures/ts/h264_aac.ts").unwrap();
let media: Media = TsDemux::new().unpackage(&ts[..]).unwrap();
assert!(media.tracks.len() >= 2, "an H.264 video and an AAC audio track");

let cmaf: Vec<u8> = CmafMux::default().package(&media).unwrap();
assert!(cmaf.len() > 1000);

let mpd: String = DashPackager::default().package(&media).unwrap();
assert!(mpd.contains("<MPD"));

// And back: TsMux is the byte-level inverse of TsDemux.
let ts_again: Vec<u8> = TsMux::default().package(&media).unwrap();
let again = TsDemux::new().unpackage(&ts_again[..]).unwrap();
assert_eq!(again.tracks.len(), media.tracks.len());
```

For a live source `Segmenter` owns the segment-cutting state machine (`push` samples, `take_ready` segments, `flush` at end of stream); `StreamingTsDemux` is the incremental demux (`feed`, `poll_event`, `finish`) - drain `poll_event` after every `feed`: the event queue is unbounded and only `poll_event` empties it (opt-in `with_max_degraded_events` caps the `InputDegraded` diagnostics). Runnable programs: `transmux_any_to_any` and `ssai_ad_stitch` (`cargo run -p transmux --example <name>`).

## Command-line packager (`cli` feature)

```console
$ cargo install transmux --features cli
$ transmux in.ts -o out.cmaf                                  # TS to CMAF, format inferred
$ transmux in.mp4 -o out.m3u8 -f hls --segment-duration 4     # MP4 to CMAF-HLS, 4 s segments
$ transmux in.ps -o out.mpd -f dash --ll                      # PS to low-latency DASH
$ transmux enc.mp4 -o dec.mp4 -f progressive --decrypt --key <KID>:<KEY>
```

`transmux --help` (abridged):

```text
Usage: transmux [OPTIONS] --output <PATH> [IN]

  -i, --input <PATH>          Input media file (alternative to the positional <IN>)
  -o, --output <PATH>         Output path (file, or the playlist/manifest path for HLS/DASH)
  -f, --format <FORMAT>       cmaf | hls | ts-hls | dash | ts | progressive
                              (inferred from the extension: .m3u8 hls, .mpd dash, .ts ts, .cmaf cmaf, .mp4 progressive)
      --segment-duration <SECS>  Target segment duration in seconds [default: 6]
      --ll                    Low-latency variant where supported (LL-DASH)
      --utc-timing-url <URL>  DASH only: advertise MPD/UTCTiming with this URL
      --tracks <IDS>          Restrict to these track IDs (comma-separated)
      --decrypt               Decrypt CENC input (requires the cenc feature)
      --key <KID:KEY>         Content key, repeatable, only with --decrypt
```

Input detection uses `container-probe`, which scores structural evidence over the leading bytes (MPEG-TS including 192-byte M2TS and mid-packet captures, ISOBMFF, MPEG-PS, WebM and Matroska, FLV). A container it recognises but this CLI has no demuxer for (MXF, WAV, Ogg, ADTS, MP3, Annex B) is reported as unsupported, and an inconclusive probe asks for the format to be given explicitly.

## What is implemented, by area

- **Hub IR** (`ir`): `Media`, `Track`, `TrackSpec`, `CodecConfig`, `Sample`, `DemuxEvent`. Sample `dts` and `pts` are absolute `Option<i64>` ticks in the track timescale, `duration` is `Option<u32>`, `data` is `bytes::Bytes`. 33-bit MPEG-2 and 32-bit RTP wrap is unrolled once at the demux edge. Typed ids `TrackId(u32)` and `ProgramNumber(u16)`; `TrackSpec::source_pid` is `Option<mpeg_ts::pid::Pid>`. `TrackSpec::display: Option<DisplayGeometry>` carries a video track's display geometry (coded size and the cropped output rectangle `output_rect` (H.264 frame cropping / H.265 conformance window), plus the H.265 `default_display_rect` suggestion; the SPS VUI sample aspect ratio as a typed `AspectRatioIdc` with `sample_aspect_ratio() -> Option<(u16, u16)>`; `overscan_appropriate`; and `video_signal` with `video_format`, the full-range flag and a typed colour description — `ColourPrimaries` / `TransferCharacteristics` / `MatrixCoefficients`, so a player knows BT.709 from BT.601 without guessing, plus `full_range()` and the spec's inferred-default accessors), derived by `TrackSpec::new` from the first SPS in the `avcC`/`hvcC`, so an anamorphic SD picture (SAR 64:45 or 16:11) can be shown at its correct shape. The **per-picture** state is `Sample::display: Option<Arc<DisplayInfo>>` (every video access unit of the TS and PS demuxers carries the geometry of the SPS its slices activate — `None` means "not tracked", never "unchanged"), and a change is announced by `DemuxEvent::DisplayChanged { display, changed: DisplayChangeSet, dts, pts, random_access, .. }`, placed before the first sample that shows the new shape and sharing its `Arc`, preceded by `TrackUpdated` when the codec-config bytes changed at the same access unit; both survive batch `Media`, IR transforms and a `media-plane` `Trunk`.
- **Demux:** `TsDemux` and `StreamingTsDemux` (PPS tracked by id, E-AC-3 waits for a whole access unit, open-GOP random access points marked; an E-AC-3 probe resolves early when its frame chain breaks or after 16 chained frames, and a PID still probing after 512 access units no longer holds back later PIDs, so a damaged capture still yields every track), `Fmp4Demux`, `ProgressiveDemux`, `PsDemux`, `WebmDemux`, `FlvDemux`, `RtmpDemux`. Damaged units no longer abort a one-shot demux: they are reported in `Media::degraded` as `InputDegradation::MalformedUnit`; a TS with broken sync bytes, garbage between packets, a truncated tail, transport errors or continuity gaps reports `SyncLoss`, `TruncatedPacket`, `TransportError` or `ContinuityGap`. `Media::degraded_at` carries the PID and packet index of each entry (TS), so damage on an unused data PID can be told from damage on a kept track. `ContinuityGap` is per PID, payload-bearing non-null packets only, ISO/IEC 13818-1 §2.4.3.3: a counter jump, a same-counter packet that is not a byte-identical repeat (PCR excepted), or any identical repeat beyond the one legal duplicate (one entry per excess packet); a signalled `discontinuity_indicator` suppresses the entry.
- **Mux and packaging:** `CmafMux`, `ProgressiveMux`, `TsMux`, `MkvMux`, `FlvMux`, `RtmpMux`, `HlsPackager`, `TsHlsPackager`, `DashPackager` (static and live, auto `ContentProtection`), `LlDashPackager`, `SmoothPackager`, `build_init_segment`, `build_media_segment`.
- **Live segmenters:** `Segmenter`, `LlHlsSegmenter` (parts before the segment closes), `StreamingTsHlsSegmenter` (rolling playlist, `with_start_sequence`, `TsContinuity`), `LlSegmenter`.
- **Transforms:** `Repackage`, `rebase_to_zero`, `apply_offset`, `insert_discontinuity_gap`, `concat`, `splice_insert` (keyframe-snapped, returns `SpliceResult`), `derive_iframe_track`.
- **DASH details:** `SegmentTimeline::resolve` and `resolve_in_period` (media-timeline `S@t` less `presentationTimeOffset`), `dash::xs_duration`, `dash_parse`, `base_url`.
- **RTP:** `RtpPacketiser`, `RtpDepacketiser`, `RtpStreamDepacketiser`, `rtp_sdp` helpers, KLV over RTP.
- **Validation:** `validate_init_segment`, `validate_media_segment`, `validate_cmaf_track` yielding `ConformanceIssue`.
- **Helpers:** `frag_offsets` (the single `trun`/`tfhd` sample-offset resolver shared by demux, decrypt and validator), NAL conversion Annex B and length-prefixed, `is_keyframe_nal`.

## Design notes

Container only, never a codec. Sans-IO pull and push APIs, zero-copy sample fan-out (`Bytes`), symmetric `Parse`/`Serialize` for every box with real-fixture round-trip gates, typed errors (`#[non_exhaustive]`, `FieldOverflow` for checked-width fields, `UnsupportedCodec`), large boxes written in the 64-bit `largesize` form past 4 GiB, hostile-input hardening (aggregate `trun` budget, `largesize` truncation guards) with fuzz targets, and content keys wiped on drop. A lexical no-hand-roll guard and a 2^33 clock guard apply.

## Related crates

Depends on `broadcast-common` (`Package`/`Unpackage`), `broadcast-hls`, `mp4-emsg`, `mpeg-ts`, `mpeg-pes`, `mpeg-ps`, `rtp-packet`, `rtcp-packet`, `dvb-si` (typed PMT audio metadata) and optionally `container-probe`. Consumed by `media-plane` (IR and `TrackId`), `hls-runtime`, `rtmp-runtime` (dev), `multimux` and `media-doctor`. Python bindings (`transmux-py`) live outside the workspace.

## MSRV and licence

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
