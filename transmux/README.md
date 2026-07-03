# transmux — any-to-any media container muxing hub

Demux any supported container into one neutral in-memory IR (`Media`/`Track`) and
mux from it into any supported container — so every `{input} → {output}` composes.
Built to spec (ISO/IEC 14496-12, 13818-1, 23009-1; RFC 8216/3550; [MS-SSTR]).
No transcode, no codec bitstream en/decode. `no_std` + `alloc`.

The spokes are the `broadcast_common` inverse-pair traits **`Unpackage`** (container
→ IR) and **`Package`** (IR → container):

| Demux → IR (`Unpackage`) | IR → mux (`Package`) |
|---|---|
| MPEG-2 TS (`TsDemux`) | CMAF/fMP4 (`CmafMux`) · progressive MP4 (`ProgressiveMux`) |
| fMP4/CMAF (`Fmp4Demux`) | MPEG-2 TS (`TsMux`) |
| MPEG Program Stream (`PsDemux`) | CMAF-HLS (`HlsPackager`) · TS-HLS (`TsHlsPackager`) |
| WebM/Matroska (`WebmDemux`) | DASH MPD (`DashPackager`) · LL-DASH (`LlDashPackager`) · Smooth (`SmoothPackager`) |
| RTMP chunk stream (`RtmpDemux`) | RTMP chunk stream (`RtmpMux`) |

Plus transforms — resegment/trim/track-select (`Repackage`), streaming CMAF
(`Segmenter`), IR timeline conditioning — PTS/DTS rebase-to-zero / offset /
33-bit MPEG wrap-unroll / discontinuity-gap insertion (`rebase_to_zero`,
`apply_offset`, `unroll_33bit_wraps`, `insert_discontinuity_gap`, over each
`Track::start_decode_time` anchor) — CENC decrypt (`CencDecryptor`), and RTP
de/packetize + SDP (`RtpPacketizer`/`RtpDepacketizer`).

## Scope — container muxing only

`transmux` **packages** coded media; it never encodes or decodes it. It parses
codec *config/parameter headers* (SPS/PPS/VPS, AAC ASC, syncframe BSI, EBML/box
config) only far enough to build the container boxes and derive metadata
(dimensions, profile, sample rate, `codecs=` MIME). Compressed samples are
**opaque payloads copied through byte-for-byte** — decode/encode is the caller's
job (WebCodecs, FFmpeg, hardware).

Every box has a symmetric `Parse` / `Serialize` with byte-identical round-trip
coverage against real fixtures; every demux/mux spoke is gated on byte-exact
round-trips through the IR.

## Feature matrix

### Container — ISOBMFF / CMAF boxes

| Group | Boxes | Status |
|---|---|---|
| File structure | `ftyp`, `styp`, `mdat`, `free`/`skip`, `uuid` | ✅ |
| Movie | `moov` · `mvhd` · `trak` · `tkhd` · `mdia` · `mdhd` · `hdlr` · `minf` · `vmhd`/`smhd` · `dinf`/`dref` | ✅ |
| Sample tables | `stbl` · `stsd` · `stts` · `ctts` · `cslg` · `stsc` · `stsz` · `stco`/`co64` · `stss` · `stsh` | ✅ |
| Edit / fragment-init | `edts`/`elst`, `mvex`/`trex` | ✅ |
| Movie fragments | `moof` · `mfhd` · `traf` · `tfhd` · `tfdt` · `trun` | ✅ |
| Random access / index | `sidx`, `mfra`/`tfra`/`mfro` | ✅ |
| Inband events / refs | `emsg`, `tref` | ✅ |
| Encryption (CENC) | `senc`/`saiz`/`saio`/`tenc`/`pssh`/`sinf`/`schm`/`frma` | ✅ |
| Multi-DRM `pssh` init data | Widevine (proto) · PlayReady (PRO/WRMHEADER) · FairPlay (`skd://`) — `drm` module | ✅ |
| Sample-entry ext | `colr` (HDR), `pasp`, `clap` | ✅ |
| Live / grouping | `prft`, `sbgp`/`sgpd`, `subs` | ✅ |

### Codecs — sample entries + config (header parse only)

| Codec | Sample entry | Config box / decode | RFC 6381 | Status |
|---|---|---|---|---|
| H.264 / AVC | `avc1` (`avc2`/`avc3`/`avc4`) | `avcC` + **SPS/PPS decode** (profile/level/chroma/bit-depth/dims/interlaced) | `avc1.PPCCLL` | ✅ |
| H.265 / HEVC | `hvc1` (`hev1`) | `hvcC` + **VPS/SPS + profile_tier_level decode** | `hvc1.…` | ✅ |
| AAC | `mp4a` | `esds` / ES_Descriptor + `AudioSpecificConfig` (+ADTS) | `mp4a.40.N` | ✅ |
| AC-3 | `ac-3` | `dac3` + syncframe BSI parse | `ac-3` | ✅ |
| E-AC-3 | `ec-3` | `dec3` + syncframe BSI parse | `ec-3` | ✅ |
| HE-AAC (SBR/PS) | `mp4a` | explicit-SBR/PS ASC | `mp4a.40.5/29` | ✅ |
| AC-4 | `ac-4` | `dac4` | `ac-4` | ✅ |
| MPEG-H 3D | `mha1`/`mhm1` | `mhaC` (record decode) | — | ✅ |
| AV1 | `av01` | `av1C` | `av01.…` | ✅ |
| Opus / FLAC / VP9 | `Opus`/`fLaC`/`vp09` | `dOps`/`dfLa`/`vpcC` | — | ✅ |
| DTS | `dtsc`/`dtsh`/`dtsl`/`dtse` | `ddts` | — | ✅ |
| **H.266 / VVC** | `vvc1`/`vvi1` | `vvcC` + **SPS/profile_tier_level decode** | `vvc1.…` | ✅ |
| **VP8** | (WebM `V_VP8`) | keyframe-header dims (RFC 6386) | — | ✅ |
| **Vorbis** | (WebM `A_VORBIS`) | `CodecPrivate` id-header decode | — | ✅ |
| **MPEG-2 video (H.262)** | `mp4v` / TS 0x02 | `esds` + sequence-header dims | `mp4v.61` | ✅ |
| **MPEG-1/2 audio (MP1/2/3)** | `mp4a` / TS 0x03/0x04 | `esds` + frame-header decode | `mp4a.6B/69` | ✅ |

### Text / captions

| Format | Sample entry | Status |
|---|---|---|
| WebVTT / TTML | `wvtt` / `stpp` | ✅ |
| CEA-608/708 (in-band SEI) | SEI extraction | ⬜ [#430](https://github.com/fishloa/rust-broadcast/issues/430) |

### Pipeline & packaging

| Feature | API | Status |
|---|---|---|
| Init segment | `build_init_segment` (ftyp + fragmented-init moov) | ✅ |
| Media segment (batch) | `build_media_segment` (styp + moof + mdat) from `TrackSpec` / `Sample` | ✅ |
| **Streaming segmenter** | `Segmenter` — `push` samples → `take_ready` segments, keyframe-cut at a target duration | ✅ |
| **LL-HLS segmenter** | `LlHlsSegmenter` — `with_part_target` + `take_ready_parts` → partial-segment CMAF chunks (RFC 8216bis §4.4.4.9) before the segment closes | ✅ |
| NAL conversion | Annex B ↔ length-prefixed (`annexb_to_length_prefixed` / `length_prefixed_to_annexb`) | ✅ |
| NAL keyframe classification | `nal_unit_type` / `is_keyframe_nal` / `access_unit_is_keyframe` (`NalCodec` AVC/HEVC/VVC) | ✅ |
| RTCP control packets | `RtcpPacket` — SR/RR/SDES/BYE/APP + `CompoundPacket` (RFC 3550 §6) | ✅ |
| HLS playlists | `MediaPlaylist` / `MasterPlaylist` (RFC 8216); `#EXT-X-DISCONTINUITY` / `#EXT-X-DISCONTINUITY-SEQUENCE` (RFC 8216 §4.3.4.3/§4.3.3.3) | ✅ |
| LL-HLS playlist directives | `MediaPlaylist::low_latency` (`LowLatencyConfig`) → `#EXT-X-SERVER-CONTROL` · `#EXT-X-PART-INF` · `#EXT-X-PART` · `#EXT-X-PRELOAD-HINT` (RFC 8216bis §4.4.3.7/§4.4.3.8/§4.4.4.9/§4.4.5.3) | ✅ |

### Hub spokes (`Unpackage` / `Package`)

| Spoke | Type | API | Status |
|---|---|---|---|
| TS demux | `Unpackage` | `TsDemux` (PAT→PMT, PES, in-band config: H.264 `avcC` · H.265 `hvcC` · MPEG-2 video `esds` · AAC/MPEG audio `esds` · AC-3/E-AC-3) | ✅ |
| fMP4 demux | `Unpackage` | `Fmp4Demux` (moov/moof → IR, all codecs) | ✅ |
| MPEG-PS demux | `Unpackage` | `PsDemux` | ✅ |
| WebM demux | `Unpackage` | `WebmDemux` (EBML) | ✅ |
| CMAF / progressive / TS mux | `Package` | `CmafMux` · `ProgressiveMux` · `TsMux` | ✅ |
| DASH / LL-DASH / Smooth | `Package` | `DashPackager` · `LlDashPackager` · `SmoothPackager` | ✅ |
| TS-HLS | `Package` | `TsHlsPackager` | ✅ |
| Repackage (resegment/trim/select) | — | `Repackage` | ✅ |
| IR timeline conditioning (rebase / offset / 33-bit unroll / gap) | — | `rebase_to_zero` · `apply_offset` · `unroll_33bit_wraps` · `insert_discontinuity_gap` (over `Track::start_decode_time`) | ✅ |
| IR timeline splice / concat → SSAI | — | `concat` · `splice_insert` (keyframe-snapped via `snap_to_preceding_sync`, → `SpliceResult` with `discontinuity_points`) | ✅ |
| CENC decrypt | `Decrypt` | `CencDecryptor` (`cenc` AES-CTR) | ✅ |
| HLS Sample-AES / AES-128 encrypt+decrypt | — | `sample_aes` (`h264_encrypt_nal` · `aac_encrypt_frame` · `ac3_encrypt_frame` · `aes128_encrypt_segment` · `ExtXKey`; feature `sample-aes`) | ✅ |
| fMP4/CMAF conformance validator | — | `validate_init_segment` / `validate_media_segment` / `validate_cmaf_track` (ISO 14496-12 + CMAF structural checks → `ConformanceIssue`) | ✅ |
| RTP de/packetize + SDP | `Package`/`Unpackage` | `RtpPacketizer` / `RtpDepacketizer` | ✅ |
| KLV metadata (SMPTE ST 336 / MISB ST 0601) | — | `KlvItem` · `UasLocalSet` (BER length + BER-OID tags, tag 2 precision timestamp, tag 1 CRC-16/CCITT checksum) | ✅ |
| KLV-over-RTP | — | `packetize_klv` / `depacketize_klv` (RFC 6597 `smpte336m`, timestamp-shared fragmentation, marker on last) | ✅ |
| RTMP transport (carries FLV A/V) | `Unpackage`/`Package` | `RtmpDemux` / `RtmpMux` (chunk stream, AMF0, → FLV spoke) | ✅ |
| FLV demux/mux | `Unpackage`/`Package` | `FlvDemux` / `FlvMux` (H.264 + AAC, Adobe FLV v10.1 Annex E) | ✅ |
| I-frame trick-play track | — | `derive_iframe_track` / `append_iframe_track` (sync-sample-only, timeline-conserving) | ✅ |

✅ = implemented + round-trip-tested · ⬜ = planned (issue linked)

## Quick start

```rust
use transmux::{build_init_segment, build_media_segment, CodecConfig, TrackSpec,
               Sample, FragmentTrackData};

// Describe each track (codec config synthesised by the caller from SPS/PPS, ADTS…).
let tracks: Vec<TrackSpec> = /* … */ Vec::new();
let init = build_init_segment(&tracks, 1000)?;         // ftyp + moov

// Feed samples per fragment.
let video: Vec<Sample> = /* … */ Vec::new();
let media = build_media_segment(1, &[FragmentTrackData {
    track_id: 1,
    base_media_decode_time: 0,
    samples: &video,
}])?;                                                  // styp + moof + mdat
# Ok::<(), transmux::Error>(())
```

For a live/streaming source, `Segmenter` owns the segment-cutting state machine:

```rust
use transmux::{Segmenter, Sample};
# fn tracks() -> Vec<transmux::TrackSpec> { Vec::new() }
# if false {
let mut seg = Segmenter::new(tracks(), 1000, 2.0)?;    // ~2 s segments
let init = seg.init_segment()?;                        // ftyp + moov, once
seg.push(1, /* Sample */ unimplemented!())?;           // coded AUs, decode order
for media in seg.take_ready() { /* write out */ }
seg.flush()?;                                          // trailing segment at EOS
# }
# Ok::<(), transmux::Error>(())
```

Codec metadata comes straight from the parsed headers — no external SPS parser:

```rust
# if false {
let sps = transmux::AvcSps(sps_nal_bytes);
let info = sps.decode()?;              // profile / level / width·height / chroma …
let mime = sps.rfc6381()?;             // e.g. "avc1.4D400D" for WebCodecs / MSE
# }
```

`tests/ts_to_cmaf.rs` demonstrates the full path end-to-end: demux a real
H.264+AAC TS, synthesise `avcC`/`esds`, and emit byte-identical-config CMAF.

## Command-line packager (`cli` feature)

The optional `cli` feature builds a `transmux` binary that wires the demux and
mux spokes into an any-to-any packager: it autodetects the input container from
its leading bytes, runs it through the [`Media`] hub IR, and writes the chosen
output format. The library itself stays `no_std`; only this feature pulls
`clap` + `std`. It follows the workspace CLI standard
([`docs/CLI-STANDARD.md`](../docs/CLI-STANDARD.md)) — clap derive, named flags,
auto `--help`/`--version`.

```console
$ cargo install transmux --features cli      # or: cargo run -p transmux --features cli --

# TS → CMAF (autodetected input; format inferred from the .cmaf extension)
$ transmux in.ts -o out.cmaf

# MP4 → CMAF-HLS, 4-second segments
$ transmux in.mp4 -o out.m3u8 -f hls --segment-duration 4

# WebM → progressive single-file MP4
$ transmux in.webm -o out.mp4 -f progressive

# PS → DASH MPD (low-latency chunked variant)
$ transmux in.ps -o out.mpd -f dash --ll
```

| Flag | Meaning |
|---|---|
| `<IN>` / `-i, --input <PATH>` | input file (container autodetected) |
| `-o, --output <PATH>` | output file / playlist / manifest path |
| `-f, --format <FMT>` | `cmaf` \| `hls` \| `ts-hls` \| `dash` \| `ts` \| `progressive` (else inferred from the output extension) |
| `--segment-duration <SECS>` | target segment duration (default 6) |
| `--ll` | low-latency variant where supported (LL-DASH) |
| `--tracks <IDS>` | restrict to these track IDs (comma-separated) |
| `--decrypt` / `--key <KID:KEY>` | decrypt CENC input (requires the `cenc` feature) |

Input autodetect signatures: MPEG-TS (`0x47` sync at 0 and +188), MP4/CMAF
(`ftyp`/`styp`/`moov`/`moof` box at offset 4), MPEG-PS (`00 00 01 BA`),
WebM/Matroska (EBML `1A 45 DF A3`), FLV (`"FLV"`).

## Spec grounding

Every layout cites its source (ISO/IEC 14496-12 boxes; 14496-1 §7.2.6 esds;
ITU-T H.264 §7.3.2.1.1 / H.265 §7.3.2.2 SPS; ETSI TS 102 366 Annex F AC-3/E-AC-3).
Codec-config tests decode **real ffmpeg-encoded fixtures** and assert against an
independent oracle (`trace_headers` fields; ffmpeg's own MP4-muxer config boxes).

## License

MIT OR Apache-2.0.
