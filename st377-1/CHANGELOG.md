# Changelog

All notable changes to `st377-1` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-07-12

### Added

- New crate: SMPTE ST 377-1:2019 "Material Exchange Format (MXF) — File
  Format Specification" — the workspace's first file-based-interchange
  crate (every other crate here is live-stream-shaped). Closes #672.
- `KlvItem` — the generic KLV (Key-Length-Value) triplet (§6.3): 16-byte
  Key + BER-encoded Length + Value, the framing primitive every other
  structure in an MXF file rides on. Zero-copy (`value: &'a [u8]`), so
  walking a huge essence-carrying file never copies sample bytes.
  `walk_klv_items`/`collect_klv_items` walk a sequence of them.
- `ber` module (BER length codec, ISO/IEC 8825-1 as constrained by §6.3.4):
  short form (`0x00`-`0x7F`) and long form (up to 8 following length
  bytes), rejecting the reserved "unspecified length" `0x80` token and
  MXF's 9-byte total cap.
- `PartitionPack` — the Header/Body/Footer Partition Pack (§7.1-§7.4,
  Tables 4-8), **fully typed**: `PartitionKind`/`PartitionStatus` (the
  Key's byte 14/15) plus every Table 5 field (KAG size, `ThisPartition`/
  `PreviousPartition`/`FooterPartition` byte offsets, `HeaderByteCount`/
  `IndexByteCount`, `BodySID`/`IndexSID`, Operational Pattern UL, the
  `EssenceContainers` UL batch). Rejects an Open Footer Partition (§7.4.1
  — "Open Footer Partitions are not permitted") on both parse and
  serialize.
- `PrimerPack` — the per-Partition local-tag → UL/UUID lookup table
  (§9.2, Tables 13-15), including `resolve_tag`/`resolve_ul` for
  decoding "dyn" (dynamically-allocated, no fixed static tag) optional
  properties by consulting a real file's own Primer Pack.
- `LocalSet`/`LocalSetItem` — the generic "local set" KLV-lite framing
  (§9.3/§9.6.1) every Header Metadata Set uses: `{local_tag: u16,
  length: u16 | BER, value}` items, with the Set Key's byte 6 selecting
  which length encoding applies (§9.3 Note 1). `StructuralSetKind`
  identifies which Set a given instance is (Table 17's full 27-entry list,
  covering every Set the spec defines, not just the four this crate
  deeply types) plus `Unknown([u8; 2])` for private/dark extensions.
- `Preface`, `Identification`, `ContentStorage`, `EssenceContainerData` —
  the four Root Metadata Sets (Annex A.2-A.5) every real MXF file has,
  decoded field-by-field from their static local tags (Interchange
  Object's `Instance UID`/`Generation UID`/`Object Class` folded into
  each). "dyn"-tagged optional properties with no fixed static tag
  (`ApplicationSchemes`/`IsRIPPresent` on `Preface`; `PrecedingIndexTable`/
  `SingularPartitionUsage`/`FollowingIndexTable`/`IsSparse` on
  `EssenceContainerData`) are preserved byte-for-byte in a `dark`
  catch-all rather than individually typed in this first pass — see
  `docs/st377-1.md`'s Scope section for the full typed-vs-generic
  breakdown with citations.
- `RandomIndexPack`/`PartitionLocation` — the optional file-trailer
  Partition index (§12, Tables 29-30): one `{BodySID, ByteOffset}` pair
  per Partition plus the trailing self-describing overall-length field
  (§12.2 Note 2), validated on parse.
- `types` module: `MxfTimestamp` (§4.3 Timestamp), `ProductVersion` +
  `ReleaseType` (§4.3's `major*256+minor` Version Type and 5-field
  ProductVersion, `ReleaseType` carrying the #204 label pair), `Auid`
  (§4.2.1's UL/UUID storage-order-swap distinguishing top bit),
  `PackageId` (opaque 32-byte UMID/Package ID, §4.2 — SMPTE ST 330's own
  internal layout is a separate normative reference, out of scope here),
  `decode_utf16_be`/`encode_utf16_be` (§4.3 String, big-endian UTF-16 via
  `char::decode_utf16`), `parse_uid_batch`/`serialize_uid_batch` (§4.3
  Batch/Array of 16-byte UL/StrongRef elements).
- `docs/st377-1.md` — curated transcription of SMPTE ST 377-1:2019
  (fetched directly from `pub.smpte.org`, free per SMPTE's 2026-06-17
  catalog release), including this crate's scope decision in full: what
  is fully typed (the format's own backbone + the four Root Metadata
  Sets) vs. identified-but-generic (every other Header Metadata Set) vs.
  wholly out of scope (Essence Container payload, Index Table contents,
  Operational-Pattern-specific constraints) — each judgment cited against
  the spec section it narrows.
- Spec-derived synthetic fixture (`tests/fixtures/synthetic_minimal.mxf`,
  `tests/fixture_synthetic.rs`): no real captured `.mxf` file exists
  anywhere in this workspace (this is the first MXF-touching crate here),
  so a minimal valid Header Partition (Primer Pack + all four typed Root
  Metadata Sets) + Body Partition (a KLV Fill item standing in for
  essence, since essence payload is out of scope) + Footer Partition +
  Random Index Pack was assembled by a standalone Python script,
  independent of this crate's own `Serialize`, directly from the spec's
  own tables — documented per `docs/CRATE-ACCEPTANCE.md`'s fallback
  provenance rule (the same precedent as `st12-1`/`rdd29` this session).
  This fixture test caught a real bug during development: an over-loose
  `PartitionPack::is_partition_key` that misidentified the Primer Pack
  and Random Index Pack keys as Partition Pack keys (they share the same
  13-byte "Defined-Length Pack, Set/Pack Registry" prefix, differing only
  in byte 14) — fixed by also checking byte 14 is a valid `PartitionKind`.
- Two runnable examples: `parse_partition` (build a Header Partition
  Pack from typed fields, serialize, parse back, and inspect its
  Operational Pattern/Essence Container inventory) and `build_preface`
  (build a `Preface` Set from typed fields, serialize, and parse back).
- `#![no_std]` (via `#![cfg_attr(not(feature = "std"), no_std)]`) +
  `alloc`; builds standalone with `--no-default-features` and on a
  bare-metal (`thumbv7em-none-eabi`) target.
- `serde` support (`Serialize`/`Deserialize` derives) behind the `serde`
  feature.
- `tests/label_coverage.rs` — the workspace's issue #204 label-convention
  drift-guard (`PartitionKind`, `PartitionStatus`, `ItemLengthMode`,
  `ReleaseType` via `impl_spec_display!`; `StructuralSetKind` via a
  hand-written `Display` impl, since its `Unknown([u8; 2])` catch-all
  carries a 2-byte payload the macro's single-byte-payload form doesn't
  fit).
- New fuzz target `st377_1`: parses the leading KLV item of arbitrary
  bytes, dispatches by Key to whichever typed parser applies
  (`PartitionPack`/`PrimerPack`/`RandomIndexPack`/`LocalSet`/the four
  Root Metadata Sets), and byte-identical round-trips each.
- Wired into the workspace: root `Cargo.toml` members, `ci.yml`'s
  `no_std` (`thumbv7em-none-eabi`) build loop, and a new
  `release-st377-1.yml` (own `st377-1-v*` tag, independent of the
  lockstep `v*` release — in the spirit of `mpeg-ts`/`st12-1`).
