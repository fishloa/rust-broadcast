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

Plus transforms — resegment/trim/track-select (`Repackage`), streaming CMAF
(`Segmenter`) — CENC decrypt (`CencDecryptor`), and RTP de/packetize + SDP
(`RtpPacketizer`/`RtpDepacketizer`).

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
| NAL conversion | Annex B ↔ length-prefixed (`annexb_to_length_prefixed` / `length_prefixed_to_annexb`) | ✅ |
| HLS playlists | `MediaPlaylist` / `MasterPlaylist` (RFC 8216); `#EXT-X-DISCONTINUITY` / `#EXT-X-DISCONTINUITY-SEQUENCE` (RFC 8216 §4.3.4.3/§4.3.3.3) | ✅ |

### Hub spokes (`Unpackage` / `Package`)

| Spoke | Type | API | Status |
|---|---|---|---|
| TS demux | `Unpackage` | `TsDemux` (PAT→PMT, PES, in-band config) | ✅ |
| fMP4 demux | `Unpackage` | `Fmp4Demux` (moov/moof → IR, all codecs) | ✅ |
| MPEG-PS demux | `Unpackage` | `PsDemux` | ✅ |
| WebM demux | `Unpackage` | `WebmDemux` (EBML) | ✅ |
| CMAF / progressive / TS mux | `Package` | `CmafMux` · `ProgressiveMux` · `TsMux` | ✅ |
| DASH / LL-DASH / Smooth | `Package` | `DashPackager` · `LlDashPackager` · `SmoothPackager` | ✅ |
| TS-HLS | `Package` | `TsHlsPackager` | ✅ |
| Repackage (resegment/trim/select) | — | `Repackage` | ✅ |
| CENC decrypt | `Decrypt` | `CencDecryptor` (`cenc` AES-CTR) | ✅ |
| RTP de/packetize + SDP | `Package`/`Unpackage` | `RtpPacketizer` / `RtpDepacketizer` | ✅ |

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

## Spec grounding

Every layout cites its source (ISO/IEC 14496-12 boxes; 14496-1 §7.2.6 esds;
ITU-T H.264 §7.3.2.1.1 / H.265 §7.3.2.2 SPS; ETSI TS 102 366 Annex F AC-3/E-AC-3).
Codec-config tests decode **real ffmpeg-encoded fixtures** and assert against an
independent oracle (`trace_headers` fields; ffmpeg's own MP4-muxer config boxes).

## License

MIT OR Apache-2.0.
