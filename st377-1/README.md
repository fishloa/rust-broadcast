# st377-1

[![Crates.io](https://img.shields.io/crates/v/st377-1.svg)](https://crates.io/crates/st377-1)
[![docs.rs](https://img.shields.io/docsrs/st377-1)](https://docs.rs/st377-1)

SMPTE ST 377-1:2019 "Material Exchange Format (MXF) — File Format
Specification": KLV (Key-Length-Value) framing, the Partition Pack, the
Primer Pack, and the four Root Metadata Sets every MXF file has, `no_std`.

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
- **[`RandomIndexPack`]** — the optional file-trailer Partition index (§12).

See `docs/st377-1.md` for the curated ST 377-1 transcription this crate
implements field-for-field, including this crate's scope decision (what's
fully typed vs. identified-but-generic, with spec citations for each call).

## Scope

MXF is a huge ecosystem spec: Operational Patterns, Essence Container
mappings, DM/Application Metadata plug-ins, and per-essence-kind Descriptors
all live in sibling documents this crate does not anticipate. This first
pass **fully types** the format's own backbone (KLV/BER framing, the
Partition Pack, the Primer Pack, "local set" framing) and the four Root
Metadata Sets every real file has; everything else (Packages, Tracks,
Sequences, Descriptors, DM/Application Metadata) is **identified but
generic** — parsed as a [`LocalSet`] tagged with its [`StructuralSetKind`],
preserved byte-for-byte, not individually decoded. **Essence Container
payload bytes are out of scope entirely** — carried opaquely via
[`KlvItem`], never decoded, the same boundary as `st337`'s `burst_payload`/
`rdd29`'s `AudioDataDLC`.

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
