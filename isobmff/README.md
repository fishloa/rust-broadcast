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
| `init_segment` | `moov`, `mvhd`, `trak`, `tkhd`, `mdia`, `mdhd`, `hdlr`, `minf`, `vmhd`/`smhd`, `dinf`/`dref`, `stbl` (`stts`/`ctts`/`stsc`/`stsz`/`stz2`/`sdtp`/`stco`/`co64`/`stss`), `stsd`, `edts`/`elst`, `mvex`/`trex`, the audio sample entries |
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
| `limits` | hostile-input caps (`Limits`) for the Phase 2 reader |
| `writer` | sans-IO fragmented writer (init segment, `moof`+`mdat`, CENC media-segment protection) and progressive writer (`Writer::progressive`: whole-file layout from a plan, `stco`/`co64`, seekable or append-only sink) |
| `sample_table` | lazy per-track sample cursor (`TrackCursor`/`CursorState`) and the run-length table builders |
| `reader` | sans-IO progressive-file `Reader`: asks for structure byte ranges, yields every track and one sample descriptor at a time, `seek_to` over `stss` |
| `plan` | `SamplePlan`, the progressive index a `Reader` hands out without reading media; `OwnedPlan` and `PlanBuilder`, its owned form; `MovieShell`/`TopLevelBox`, the boxes a plan carries back to a writer |
| `frag_offsets` | ISO/IEC 14496-12:2026 §8.8.7/§8.8.8 sample-data addressing rules |
| `reader` (fragmented) | the same `Reader` reads a fragmented file one `moof` at a time and seeks through `sidx` or the trailing `tfra` |
| `reader` (permissive) | `Mode::Permissive` reports the design §4.7 faults as `Step::Warning`; strict mode keeps rejecting them |
| `quicktime` | read-only QuickTime atoms: sound description versions 1/2, `wave`, `gmhd`, `tmcd`, `tapt`, chapter titles, `qt  `/brandless files |
| `driver` | blocking drivers over the reader: `FileReader` (`Read + Seek`) and `ForwardReader` (`Read` only), with a bounded reusable buffer |
| `timing` (random access) | `tfra`, `mfra`, `mfro` |
| `emsg` | `emsg` (versions 0 and 1, ISO/IEC 23009-1 §5.10.3.3) |
| `metadata` | `udta`, `meta`, `hdlr`, `keys`, `ilst`, `data`, `mean`, `name`, `chpl`, `loci`; QuickTime user-data text and `©xyz` ISO 6709 |
| `registry` | runtime registration of private boxes and sample entries (`BoxDef`, `BoxRegistry`, `RegisteredEntry`) |

Every type implements the workspace `Parse` / `Serialize` pair, so a parse →
serialize round-trip is byte-identical.

## Example

Open a file and iterate its samples in five lines:

```rust,no_run
use std::fs::File;
use isobmff::driver::{DriverStep, FileReader};
use isobmff::ReaderConfig;

let mut reader = FileReader::new(File::open("movie.mp4")?, ReaderConfig::default())?;
loop {
    match reader.step()? {
        DriverStep::Sample(sample) => println!("track {} at {}", sample.track, sample.decode_time),
        DriverStep::Done => break,
        _ => {}
    }
}
```

A box builder still works the same way:

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

Six runnable examples ship with the crate:

```bash
cargo run -p isobmff --example parse_boxes -- fixtures/mp4/h264-aac-prog.mp4
cargo run -p isobmff --example build_ftyp
cargo run -p isobmff --example parse_emsg
cargo run -p isobmff --example build_emsg
cargo run -p isobmff --example list_samples -- fixtures/mp4/progressive/av-prog.mp4
cargo run -p isobmff --example read_tags -- fixtures/mp4/progressive/av-prog.mp4
cargo run -p isobmff --example write_tags -- fixtures/mp4/progressive/av-prog.mp4 /tmp/tags.udta
```

## Drivers

`Reader` is sans-IO: it asks for structure byte ranges and yields descriptors.
`isobmff::driver` answers those demands for the two blocking source shapes.

- `FileReader<R: Read + Seek>` seeks to each demand. A `moov` after a
  multi-gigabyte `mdat` costs only its headers, and `read_sample` reads one
  sample's bytes on demand.
- `ForwardReader<R: Read>` never seeks: a demand ahead of the current position
  is reached by reading and discarding through one fixed buffer, and a header
  the reader must re-read with a `largesize`/`uuid` extension is answered from
  a retained header tail. A `moov` after an `mdat` is
  `Error::MoovAfterMdatForwardOnly`. It forces `SampleOrder::File`, so a pipe
  always reads samples in ascending file order, and the reader emits a
  fragment's samples before it asks for anything past that fragment's `mdat`:
  `read_sample` therefore works on a fragmented file read from a pipe or
  socket.

Each driver yields `DriverStep::Metadata` once, after the track table, so a
driver user sees the tags as well as the tracks.

Neither driver holds a whole file: each keeps one `moov`/`moof` buffer bounded
by the reader's own `Limits`. For async sources see the `isobmff-tokio` crate;
for HTTP range requests see `broadcast-client`'s `mp4_range` module.

## Extending: private boxes and private tracks

A private box or a private sample entry needs no change to this crate.
Implement `Parse` + `Serialize` as any box here does, then `BoxDef` (its
four-CC or `uuid` extended type, and an optional parent scope), and register it:

```rust,ignore
use isobmff::registry::{BoxDef, BoxRegistry};

impl BoxDef for MyAtom {
    const FOURCC: [u8; 4] = *b"acme";
    const PARENT: Option<[u8; 4]> = Some(*b"udta"); // None: any parent
}

let mut registry = BoxRegistry::new();
registry.register::<MyAtom>()?;             // over a four-CC this crate types
registry.register_override::<MyAtom>()?;    // is an error without `override`
let mut reader = Reader::new(ReaderConfig::default().with_registry(registry));
```

- A registered `stsd` entry comes back typed in `SampleEntryInfo::registered`
  (`RegisteredEntry::downcast_ref::<MyEntry>()`); unregistered, it stays the raw
  entry bytes.
- `BoxRegistry::parse_children(parent, payload)` types the children of a box
  this crate leaves opaque (vendor `udta` atoms, a top-level `uuid`).
- The writer needs no registry: `SampleEntry::from_registered` turns the value
  into the raw entry bytes it carries, and `RegisteredEntry::try_to_bytes` gives
  the box bytes for an opaque child. An unregistered box is an `OpaqueBox` and
  writes back unchanged.

Payload formats (the meaning of the bytes) belong to the plug-in crate, not
here: this crate knows box syntax only. The `isobmff-private-format` workspace
example is a complete, copyable implementation of one private atom and one
private track.

## Round trip

Reading a file and writing it back through `Writer` reproduces it **byte for
byte** for every file this crate can model, because a plan read from a file
carries what the sample tables do not: the parsed `trak` shells (`tkhd` times
and flags, `hdlr` fields, `gmhd`, `tref`), the `moov`'s extra children
(`udta`/`meta`, `iods`, `mvex`), each `stbl` child the plan does not rebuild
(`sgpd`/`sbgp`/`sdtp`) in its wire position, and the top-level boxes around the
movie (`free`, `wide`, `uuid`, `skip`). A plan a `PlanBuilder` made carries
none of it and writes this crate's own conventional layout.

The `isobmff/tests/it/whole_file_round_trip.rs` test asserts the invariant over
every committed progressive fixture, the tagged `.m4a` set, the GoPro captures
and the QuickTime `.mov` set. Files that cannot be byte-identical are listed
there with the exact reason and compared structurally instead: the fragmented
fixtures state their sample layout differently in `moof` (a `tfhd` with sample
defaults or `base_data_offset`, a `trun` with a per-run keyframe flag) and end
with a trailing `mfra`, and one carries a top-level `sidx`; the writer emits
per-sample `trun` fields under `default-base-is-moof`, no `mfra` and no `sidx`.

## Coming from `mp4` or `mp4parse`

Those crates expose a whole-item parse: read a header, then read samples by
index from the same in-memory or seekable source. This crate keeps the same
two jobs but splits them into a sans-IO state machine and a driver:

- open the file once with `FileReader::new` (or `ForwardReader::new` for a
  source that cannot seek);
- call `step` until it returns `DriverStep::Done`, handling
  `DriverStep::Tracks`, `DriverStep::Fragment` and `DriverStep::Sample`;
- call `read_sample` when you want one sample's bytes.

The track table and the sample descriptors are plain data, so a consumer that
only needs an index never reads media.

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std` | yes | `std` glue; without it the crate is `no_std` + `alloc` |
| `serde` | no | `serde::Serialize` on the box types |
| `scan-counters` | no | test instrumentation: `Reader::bytes_requested()` counts the bytes a run asked for |

## Spec citations

- **Box / FullBox**: ISO/IEC 14496-12:2026 §4.2.
- **Skip-unknown rule**: §4.2 — unknown type → ignore + skip via `size`.
- **CENC boxes**: ISO/IEC 23001-7 §12.
- **`emsg`**: ISO/IEC 23009-1 §5.10.3.3; DASH-IF IOP Part 10 §6.1.
- **Metadata (`udta`/`meta`/`keys`/`ilst`)**: QuickTime File Format
  (`private/specs-verified/apple/isobmff/quicktime-file-format-online.md`),
  ISO/IEC 14496-12:2026 §8.10.1/§8.11, and 3GPP TS 26.244 Table 8.10 for
  `loci`.
