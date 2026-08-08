# st377-1

[![Crates.io](https://img.shields.io/crates/v/st377-1.svg)](https://crates.io/crates/st377-1)
[![docs.rs](https://img.shields.io/docsrs/st377-1)](https://docs.rs/st377-1)

SMPTE ST 377-1:2019 "Material Exchange Format (MXF) — File Format
Specification": KLV (Key-Length-Value) framing, the Partition Pack, the
Primer Pack, the four Root Metadata Sets every MXF file has, and the OP1a
("single item, single package") Operational Pattern's structural metadata
Sets, `no_std`.

This is the first file-based-interchange crate in the workspace — every
other crate here is live-stream-shaped.

- **[`KlvItem`]** — the generic KLV triplet (§6.3) everything else in an MXF
  file rides on; [`walk_klv_items`]/[`collect_klv_items`] walk a sequence.
- **[`PartitionPack`]** — the Header/Body/Footer Partition Pack (§7.1-§7.4,
  Tables 4-8): [`PartitionKind`] + [`PartitionStatus`] plus every Table 5
  field (KAG size, byte offsets, Operational Pattern UL, Essence Container
  UL batch).
- **[`PrimerPack`]** — the per-Partition local-tag → UL/UUID lookup table
  (§9.2).
- **[`LocalSet`]** — the generic "local set" KLV-lite framing (§9.3) every
  Header Metadata Set uses; [`StructuralSetKind`] identifies which Set a
  given instance is (Table 17) even for Sets this crate doesn't deeply type.
- **[`Preface`]**, **[`Identification`]**, **[`ContentStorage`]**,
  **[`EssenceContainerData`]** — the four Root Metadata Sets (Annex A) every
  real MXF file has, decoded field-by-field.
- **[`MaterialPackage`]**, **[`SourcePackage`]** — the two concrete Package
  kinds (Annex E / B.1): Package UID, name, dates, and Track references.
- **[`TimelineTrack`]**, **[`EventTrack`]**, **[`StaticTrack`]** — the three
  Track kinds (B.12/B.13/B.14).
- **[`Sequence`]**, **[`SourceClip`]**, **[`TimecodeComponent`]**,
  **[`FillerComponent`]** — the Track-Sequence-Component chain's parts
  (B.9/B.10/B.11/B.17).
- **[`op1a`]** — OP1a Operational Pattern UL identification/qualifier
  helpers (SMPTE ST 378M).
- **[`RandomIndexPack`]** — the optional file-trailer Partition index (§12).

See `docs/st377-1.md` for the curated ST 377-1 transcription this crate
implements field-for-field, including this crate's scope decision (what's
fully typed vs. identified-but-generic, with spec citations for each call),
and `docs/st378-op1a.md` for the OP1a transcription and this crate's OP1a
implementation status.

## Scope

MXF is a huge ecosystem spec: Operational Patterns, Essence Container
mappings, DM/Application Metadata plug-ins, and per-essence-kind Descriptors
all live in sibling documents this crate does not anticipate. This crate
**fully types** the format's own backbone (KLV/BER framing, the Partition
Pack, the Primer Pack, "local set" framing), the four Root Metadata Sets
every real file has, and the OP1a Operational Pattern's structural metadata
Sets (Packages, Tracks, Sequences, SourceClip, TimecodeComponent,
FillerComponent — Annex B/E); everything else (Essence Descriptors, DM
Segments/Source Clips, Application Metadata Sets) is **identified but
generic** — parsed as a [`LocalSet`] tagged with its [`StructuralSetKind`],
preserved byte-for-byte, not individually decoded. **Essence Container
payload bytes are out of scope entirely** — carried opaquely via
[`KlvItem`], never decoded, the same boundary as `st337`'s `burst_payload`/
`rdd29`'s `AudioDataDLC`.

### OP1a limitations (issue #937)

Typing the OP1a structural metadata Sets is not the same as being able to
build or fully parse a complete OP1a *file*. Two gaps, by design, not by
oversight:

- **No `EssenceDescriptor` type.** [`SourcePackage::descriptor`] is a bare
  16-byte [`StrongRef`] this crate can neither resolve nor build a target
  for. A real OP1a file's actual Essence Descriptors (e.g. an MPEG Video
  Descriptor, a Wave Audio Descriptor) are registered by essence-container-
  mapping specs outside ST 377-1 itself, so this isn't a small gap to close.
- **No file assembler.** Nothing in this crate computes cross-Partition
  byte offsets, `HeaderByteCount`/`IndexByteCount`, or emits a
  [`RandomIndexPack`] pointing at real Partition offsets. Each piece
  ([`PartitionPack`], [`PrimerPack`], the typed Header Metadata Sets,
  [`RandomIndexPack`]) parses and serializes correctly on its own; nothing
  stitches them into one valid file.

See the crate root docs (`cargo doc -p st377-1 --open`) for the full
breakdown, including what a complete implementation would additionally
require.

`#![no_std]` + `alloc`; depends only on `broadcast-common`.

## Quick start

```rust
use broadcast_common::{Parse, Serialize};
use st377_1::{PartitionKind, PartitionPack, PartitionStatus};

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
};
let bytes = pack.to_bytes();
assert_eq!(PartitionPack::parse(&bytes).unwrap(), pack);
```

## Examples

```sh
cargo run -p st377-1 --example parse_partition
cargo run -p st377-1 --example build_preface
```

## Features

| Feature   | Default | Description |
|-----------|---------|-------------|
| `std`     | yes     | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde`   | no      | `serde::Serialize`/`Deserialize` derives on public types. |

## Minimum Supported Rust Version

1.86

## License

MIT OR Apache-2.0
