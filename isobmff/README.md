# isobmff

ISO/IEC 14496-12:2026 (ISO Base Media File Format) **box syntax**, no_std +
`alloc`, depending only on `broadcast-common`.

This crate is the box layer below the `transmux` muxing hub: it parses and
writes the `size`/`type` box headers (including the 64-bit `largesize` and the
`uuid` extended type), the top-level and movie-fragment box set, the `moov`
tree and its sample entries, the sample-table/timing boxes, the
visual-extension boxes, the sample-group boxes, the CENC box syntax
(ISO/IEC 23001-7), and the DASH Event Message Box (`emsg`, ISO/IEC 23009-1).
It never knows the media IR or a codec bitstream.

## Coverage

| Module | Boxes (four-CC) |
|---|---|
| `box_types` | `size`/`type` header, `largesize`, `uuid` usertype, the `BoxIter` walker |
| `box_support` | shared opaque-child and container building helpers |
| `segments` | `ftyp`, `styp`, `mdat` |
| `init_segment` | `moov`, `mvhd`, `trak`, `tkhd`, `mdia`, `mdhd`, `hdlr`, `minf`, `vmhd`/`smhd`, `dinf`/`dref`, `stbl`, `stsd`, `edts`/`elst`, `mvex`/`trex`, the audio sample entries |
| `sample_entries` | `avc1`/`avc3`, `hvc1`/`hev1`, `vvc1`/`vvi1`, `mp4v`, `VisualSampleEntry` |
| `subtitle_entries` | `stpp`, `wvtt`, `vttc`/`vtte`/`vtta`/`payl`/`sttg`/`iden` |
| `avc_parameter_set` | `avcp` sample entry and parameter set sample |
| `track_reference` | `tref` |
| `track_id` | `track_ID` newtype (ISO/IEC 14496-12:2026 §8.3.2) |
| `avc_config` | `avcC` |
| `hevc_config` | `hvcC` |
| `vvc_config` | `vvcC` |
| `nalu_types` | `AvcSps`/`AvcPps`/`HevcNalUnit` newtypes over parameter-set NAL units |
| `mp4esds` | `esds` (ES/DecoderConfig/DecoderSpecificInfo/SLConfig descriptors) |
| `ac3` | `dac3`, `dec3` |
| `ac4` | `dac4` (`ac-4` entry) |
| `av1` | `av1C`, `av01` |
| `vp9` | `vpcC`, `vp09` |
| `opus` | `dOps`, `Opus` |
| `flac` | `dfLa`, `fLaC` |
| `dts` | `ddts`, `dtsc`/`dtsh`/`dtsl`/`dtse` |
| `mpegh` | `mhaC`, `mha1`/`mha2`/`mhm1`/`mhm2` |
| `dovi` | `dvcC`, `dvvC` |
| `avs2` | `av2c`, `la2c`, `avst`, `lav2` |
| `avs3` | `av3c`, `lavc`, `avs3`, `lav3`, plus its sample-group entries |
| `avs3_audio` | `dca3`, `av3a` |
| `hdr_boxes` | `mdcv`, `clli` |
| `timing` | `stts`, `ctts`, `cslg`, `elst`, `sidx` |
| `visual_ext` | `pasp`, `clap`, `colr` |
| `cenc` | `tenc`, `senc`, `pssh`, `saiz`, `saio`, `sinf`, `frma`, `schm`, `schi` |
| `sample_groups` | `prft`, `sgpd`, `sbgp`, `subs`, `ssrt` |
| `movie_fragment` | `moof`, `mfhd`, `traf`, `tfhd`, `tfdt`, `trun` |
| `frag_offsets` | ISO/IEC 14496-12:2026 §8.8.7/§8.8.8 sample-data addressing rules |
| `emsg` | `emsg` (versions 0 and 1, ISO/IEC 23009-1 §5.10.3.3) |

Every type implements the workspace `Parse` / `Serialize` pair, so a parse →
serialize round-trip is byte-identical.

## Example

```rust,ignore
use broadcast_common::Serialize;
use isobmff::segments::FileTypeBox;

let ftyp = FileTypeBox {
    major_brand: *b"iso5",
    minor_version: 0,
    compatible_brands: vec![*b"iso5", *b"iso6", *b"mp41"],
};
let bytes = ftyp.try_to_bytes().unwrap();
```

Four runnable examples ship with the crate:

```bash
cargo run -p isobmff --example parse_boxes -- fixtures/mp4/h264-aac-prog.mp4
cargo run -p isobmff --example build_ftyp
cargo run -p isobmff --example parse_emsg
cargo run -p isobmff --example build_emsg
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std` | yes | `std` glue; without it the crate is `no_std` + `alloc` |
| `serde` | no | `serde::Serialize` on the box types |

## Spec citations

- **Box / FullBox**: ISO/IEC 14496-12:2026 §4.2.
- **Skip-unknown rule**: §4.2 — unknown type → ignore + skip via `size`.
- **CENC boxes**: ISO/IEC 23001-7 §12.
- **`emsg`**: ISO/IEC 23009-1 §5.10.3.3; DASH-IF IOP Part 10 §6.1.
