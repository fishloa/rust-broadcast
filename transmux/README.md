# transmux — MPEG-2 TS → CMAF/fMP4 remux

Container-layer remux from MPEG-2 Transport Stream to fragmented MP4 / CMAF,
built to spec (ISO/IEC 14496-12:2015, RFC 8216). **Samples-in**: the caller
supplies demuxed *encoded* access units + codec config; `transmux` produces the
initialization and media segments (and the HLS playlists). No transcode, no
codec bitstream decoding. `no_std` + `alloc`.

## What's in the box

| Layer | Types |
|---|---|
| ISOBMFF boxes | `Box`/`FullBox` framing, `MovieBox` tree (`mvhd`/`trak`/`tkhd`/`mdia`/`minf`/`stbl`/`stsd`), `mvex`/`trex`, movie-fragment boxes (`moof`/`mfhd`/`traf`/`tfhd`/`tfdt`/`trun`), `ftyp`/`styp`/`mdat`, timing boxes (`stts`/`ctts`/`stsc`/`stsz`/`stco`/`elst`/`sidx`) |
| Codec config | `avcC`/`hvcC` decoder-configuration records, `esds`/ES_Descriptor, AAC `AudioSpecificConfig` (+ ADTS) |
| NAL | Annex B ↔ length-prefixed conversion (`annexb_to_length_prefixed` / `length_prefixed_to_annexb`) |
| Pipeline | `build_init_segment` (ftyp + fragmented-init moov) and `build_media_segment` (styp + moof + mdat) from a `TrackSpec` / `Sample` samples-in API |
| Packaging | HLS media + master playlist generation (`MediaPlaylist` / `MasterPlaylist`, RFC 8216) |

Every box has a symmetric `Parse` / `Serialize` with byte-identical round-trip
coverage against real fMP4 fixtures.

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

An end-to-end `tests/ts_to_cmaf.rs` demonstrates the full path: demux a real
H.264+AAC TS, synthesise `avcC`/`esds`, and emit byte-identical-config CMAF
segments.

## Scope

`transmux` is the **container layer only**. Demux (TS → elementary streams) and
transcode live in the caller. It is a clean-room, spec-built alternative to
ripping a C muxer — every layout cites ISO/IEC 14496-12.

## License

MIT OR Apache-2.0.
