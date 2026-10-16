# st377-1

[![Crates.io](https://img.shields.io/crates/v/st377-1.svg)](https://crates.io/crates/st377-1)
[![docs.rs](https://img.shields.io/docsrs/st377-1)](https://docs.rs/st377-1)

SMPTE ST 377-1:2019 Material Exchange Format (MXF): KLV framing, Partition
Pack, Primer Pack, the Header Metadata Sets of an OP1a file, and the Random
Index Pack, `no_std` + `alloc`.

## What it does / what it does not

MXF is the professional file-interchange wrapper for audio/video (broadcast
playout servers, archives, camera and editing exports). This crate reads and
writes the **structure** of an MXF file: the KLV triplets, the partition
layout and the structural metadata that says what the file contains. It is the
first file-based-interchange crate in the workspace; every other crate is
live-stream-shaped.

It does **not**:

- decode essence: Essence Container payload bytes stay opaque `KlvItem`s;
- type Essence Descriptors (F.2 to F.6 and the per-codec registrations such as
  MPEG Video / Wave Audio): `SourcePackage::descriptor` is a bare `StrongRef`
  this crate can neither resolve nor build a target for;
- decode Index Table contents, DM Segments / DM Source Clips, or Application
  Metadata Sets (they are identified via `StructuralSetKind` and preserved as
  generic `LocalSet`s);
- assemble a file: nothing computes cross-Partition offsets,
  `HeaderByteCount`/`IndexByteCount`, or a `RandomIndexPack` pointing at real
  partition offsets (each piece parses and serializes on its own; nothing
  stitches them into one playable OP1a file);
- implement other Operational Patterns (OP1b, OP2x, OP3x, OPAtom), Essence
  Container mappings (ST 379 and siblings).

## Standards supported

| Standard | Edition | Scope / clauses | Status | Notes |
|----------|---------|-----------------|--------|-------|
| SMPTE ST 377-1 - MXF File Format Specification | 2019 (vendored, 187 pp.; revision of 377-1-2009) | KLV and BER framing (6.3), Partition Pack (7.1-7.4, Tables 4-8), Primer Pack (9.2), local-set framing (9.3), Header Metadata Set identification (Table 17), Random Index Pack (12), Annex A root sets, Annex B/E structural sets | Partial | Typed vs opaque split listed below |
| SMPTE ST 378 - MXF Operational Pattern 1a | 2004 (378M-2004, vendored) | OP1a Operational Pattern UL identification and qualifier bits (`op1a` module) | Partial | UL helpers only; the minimum-file requirement for an `EssenceDescriptor` is not met |
| Essence-container mapping specs (ST 379 family and siblings) | not vendored | none | Not implemented | Essence stays opaque |

Typed Header Metadata Sets (dedicated struct, decoded field by field,
byte-identical re-serialization):

- Root sets (Annex A): `Preface`, `Identification`, `ContentStorage`,
  `EssenceContainerData`.
- Packages: `MaterialPackage`, `SourcePackage`.
- Tracks: `TimelineTrack`, `EventTrack`, `StaticTrack`.
- Components: `Sequence`, `SourceClip`, `TimecodeComponent`,
  `FillerComponent`.

Typed packs: `PartitionPack`, `PrimerPack`, `RandomIndexPack`.

Identified only (`StructuralSetKind` name + generic `LocalSet`, bytes
preserved): DM Segment, DM Source Clip, Package Marker Object, File /
Generic Picture / CDCI / RGBA / Generic Sound / Generic Data / Multiple
Descriptors, Network and Text Locators, Application Plug-In and Referenced
Objects, plus `Unknown([u8; 2])` for any Set not in Table 17.

## Features

| Feature   | Default | Description |
|-----------|---------|-------------|
| `std`     | yes     | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde`   | no      | `serde::Serialize`/`Deserialize` derives on public types. |

## Quick start

```rust
use broadcast_common::{Parse, Serialize};
use st377_1::{BerLength, PartitionKind, PartitionPack, PartitionStatus};

let pack = PartitionPack {
    kind: PartitionKind::Header,
    status: PartitionStatus::ClosedComplete,
    major_version: 1,
    minor_version: 3,
    kag_size: 512,
    this_partition: 0,
    previous_partition: 0,
    footer_partition: 0,
    header_byte_count: 0,
    index_byte_count: 0,
    index_sid: 0,
    body_offset: 0,
    body_sid: 0,
    operational_pattern: [0u8; 16],
    essence_containers: Vec::new(),
    len_size: BerLength::Minimal,
};
let bytes = pack.try_to_bytes().unwrap();
assert_eq!(PartitionPack::parse(&bytes).unwrap(), pack);
```

More: `cargo run -p st377-1 --example parse_partition`, `--example build_preface`.

## What is implemented

- **KLV** (`KlvItem`, `walk_klv_items`, `collect_klv_items`, fill-item
  helpers) and `BerLength`, which records the on-wire BER length width so
  re-serialization is byte-identical even for the fixed-width long forms real
  encoders use.
- **`PartitionPack`** (`PartitionKind` x `PartitionStatus` and every Table 5
  field), **`PrimerPack`**, **`RandomIndexPack`** (`PartitionLocation`).
- **`LocalSet` / `LocalSetItem`** (generic local-set framing; `ItemLengthMode`)
  and `StructuralSetKind` (Table 17 identification).
- The typed Header Metadata Sets listed above, with the shared value types
  `Auid`, `PackageId`, `StrongRef`, `Rational`, `MxfTimestamp`,
  `ProductVersion`, `UlBytes` and UTF-16BE helpers.
- **`op1a`**: `OP1A_UL_PREFIX`, `Op1aQualifier`, `is_op1a`, `op1a_ul`.
- Validated against a real ffmpeg-muxed OP1a file
  (`tests/fixtures/op1a_mpeg2_pcm.mxf`: all 25 partition packs round-trip
  byte-identically) and a small synthetic file.

## Design notes

- Symmetric `Parse` / `Serialize` with byte-identical round-trips; equality
  ignores the serialization-form field `len_size`.
- `Preface::identifications` is an `Option` so an absent property is
  distinguished from an empty batch.
- Public enums are `#[non_exhaustive]` with `name()` + `Display` labels.
- `no_std` + `alloc`; one `thiserror` error enum.

## Related crates

Depends only on `broadcast-common`. Sibling file/bitstream crates with the same
"container, not codec" boundary: `st337` and `rdd29`.

## Minimum Supported Rust Version

1.95.0

## License

MIT OR Apache-2.0
