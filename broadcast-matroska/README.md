# broadcast-matroska

[![license](https://img.shields.io/crates/l/broadcast-matroska.svg)](#license)

EBML (RFC 8794) and Matroska/WebM (RFC 9559) **element syntax**, `no_std` +
`alloc`, depending only on `broadcast-common`.

This crate is the element layer below the `transmux` muxing hub: it parses and
serializes the EBML element framing (Element IDs, Element Data Sizes including
the unknown size, the EBML header, `Void` and `CRC-32`), the scalar payload
types, and every Matroska master and leaf element, including the `SimpleBlock`
and `Block` header with all four lacing forms. It never knows the media IR or a
codec bitstream.

A file read and rewritten through this crate comes back byte for byte: the
encoded width of every size and integer is preserved, child order is the file's
order, and an element the crate does not name keeps its id and payload.

## Coverage

| Module | Elements |
|---|---|
| `ebml::vint` | Element ID (1–4 octets), Element Data Size (1–8 octets, known and unknown) |
| `ebml::header` | element header, the size-driven element walker |
| `ebml::scalar` | unsigned/signed integer, float (4/8), string, UTF-8, date, binary |
| `ebml::document` | the EBML header (`EBMLVersion`, `DocType`, `DocTypeVersion`, …) |
| `ebml::global` | `Void`, `CRC-32` |
| `matroska::ids` | every Matroska element ID, its EBML type and its parent (RFC 9559 §5, §27.1) |
| `matroska::element` | `Segment`, `SeekHead`/`Seek`, `Info`, `Tracks`/`TrackEntry` (`Video` with `Colour`/`Projection`, `Audio`, `ContentEncodings`), `Cluster`, `Cues`, `Chapters`, `Tags`, `Attachments`, and the rest of the registry |
| `matroska::block` | `SimpleBlock` / `Block` (track VINT, relative timestamp, flags, none/Xiph/EBML/fixed lacing) |
| `matroska::accessors` | typed field accessors (`track_type`, `pixel_width`, `cue_cluster_position`, …) |
| `limits` | hostile-input caps (`Limits`) for depth, element size and child count |

## Tests

The round-trip gate walks every committed fixture (`fixtures/mkv/*`, six files
across H.264/HEVC, VP8/VP9 and Opus/Vorbis) to the end and re-serializes it
byte for byte, element by element. The fixtures are genuine ffmpeg `-c copy`
remuxes of committed captures (see `fixtures/mkv/PROVENANCE.md`), so ffmpeg is
the independent oracle for the frame bytes; the element syntax is pinned to the
RFC 9559 transcription loaded from the private repo at test time.

## Install

```toml
[dependencies]
broadcast-matroska = "0.1.0"
```

The default `std` feature is only glue; without it the crate is `no_std` +
`alloc`. The optional `serde` feature derives `serde::Serialize` on the element
types.

## Spec citations

- **EBML**: RFC 8794 (`private/specs-verified/ietf/specs/rfc8794_ebml.txt`) —
  §4 VINT, §5 Element ID, §6 Element Data Size, §7 element types, §8 document
  structure, §11.2 the EBML header, §11.3 global elements.
- **Matroska**: RFC 9559 (`private/specs-verified/ietf/specs/rfc9559_matroska.txt`)
  — §5 element definitions, §6 Segment, §11 timestamps, §12 Block structure,
  §27.1 the element-ID registry. The demux-relevant subset is transcribed at
  `private/specs-verified/mpeg/transmux/webm/ebml-matroska.md`.

## Examples

```text
cargo run -p broadcast-matroska --example dump_elements -- fixtures/mkv/vp9-opus.webm
cargo run -p broadcast-matroska --example list_tracks -- fixtures/mkv/vp9-opus.webm
```

## License

MIT OR Apache-2.0.
