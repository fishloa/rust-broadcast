//! Parses `tests/fixtures/op1a_mpeg2_pcm.mxf` — a real OP1a MXF file
//! (MPEG-2 video + PCM audio) — and validates every top-level KLV item:
//!
//! - Partition Packs, the Primer Pack, and the Random Index Pack — all
//!   spec-fixed positional layouts, not "sets" — round-trip
//!   **byte-identically** through their typed parsers.
//! - Every Header Metadata Set (a "local set" key, §9.3) round-trips
//!   **byte-identically** through the generic [`LocalSet`] (an order-
//!   preserving passthrough over the item list it parsed, so this is a
//!   genuine byte-fidelity check against whatever a real encoder wrote).
//! - If the set's [`StructuralSetKind`] additionally has a typed
//!   representation in this crate ([`Preface`]/[`Identification`]/
//!   [`ContentStorage`]/[`EssenceContainerData`]/[`MaterialPackage`]/
//!   [`SourcePackage`]/[`TimelineTrack`]/[`EventTrack`]/[`StaticTrack`]/
//!   [`Sequence`]/[`SourceClip`]/[`TimecodeComponent`]/[`FillerComponent`]),
//!   it must ALSO parse through that typed struct and round-trip
//!   **losslessly**: parse -> serialize -> parse gives back an equal value.
//!   This is deliberately *not* a byte-identical check against the
//!   original captured bytes: a Local Set is an unordered bag of
//!   `{tag, value}` items (§9.3) — property order is an encoder's own
//!   choice, not spec-mandated — and this fixture proves it (a real
//!   `Identification` Set from `ffmpeg`/`Lavf` writes `Platform` (`0x3C08`)
//!   before `ProductUID`/`ModificationDate`/`ToolkitVersion`, ahead of this
//!   crate's Annex-A.3-declaration-order canonicalization). Demanding
//!   byte-identical reserialize from the typed struct would mean chasing
//!   one specific encoder's ordering rather than testing this crate's own
//!   parser; the `LocalSet` byte-fidelity check above already proves this
//!   crate CAN reproduce a real encoder's exact bytes when it doesn't
//!   re-order anything.
//! - Kinds this crate only *identifies* (Essence Descriptors, DM/Application
//!   Metadata, private/dark extensions — `StructuralSetKind::Unknown` and
//!   friends) get only the generic `LocalSet` byte-fidelity round-trip
//!   above; there is no typed struct for them to additionally check (see
//!   crate root docs on scope).
//!
//! Every branch **asserts**. Nothing is silently skipped: a local-set key
//! that fails to parse, or a typed parser that fails or loses/changes data,
//! fails the test loudly, naming the offending item index, key, and type.
//! The only KLVs skipped without asserting are genuine non-local-set items —
//! Essence Container elements and Index Table segments — which are
//! confirmed *not* to be local-set keys at all before being skipped, and are
//! out of scope for this crate regardless (never decoded).

use broadcast_common::{Parse, Serialize};
use st377_1::{
    ContentStorage, Error, EssenceContainerData, EventTrack, FillerComponent, Identification,
    LocalSet, MaterialPackage, PartitionPack, Preface, PrimerPack, RandomIndexPack, Sequence,
    SourceClip, SourcePackage, StaticTrack, StructuralSetKind, TimecodeComponent, TimelineTrack,
    collect_klv_items, is_fill_item_key, is_local_set_key,
};

fn fixture_bytes() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/op1a_mpeg2_pcm.mxf"
    ))
    .expect("read tests/fixtures/op1a_mpeg2_pcm.mxf")
}

/// Parse `klv` as `T`, then serialize -> parse again and assert the second
/// parse equals the first (a lossless round-trip). Panics naming `name` and
/// item index `i` on any failure — parse failure, serialize failure, the
/// reserialized bytes failing to reparse, or a value mismatch. See the
/// module doc for why this is a value round-trip rather than a byte-for-byte
/// comparison against the original captured bytes.
fn assert_typed_round_trip<T>(klv: &[u8], i: usize, name: &str)
where
    T: for<'a> Parse<'a, Error = Error> + Serialize<Error = Error> + PartialEq + core::fmt::Debug,
{
    let parsed = T::parse(klv).unwrap_or_else(|e| panic!("item {i}: {name} parse failed: {e}"));
    let mut out = vec![0u8; parsed.serialized_len()];
    parsed
        .serialize_into(&mut out)
        .unwrap_or_else(|e| panic!("item {i}: {name} serialize failed: {e}"));
    let reparsed = T::parse(&out)
        .unwrap_or_else(|e| panic!("item {i}: {name} reserialized bytes failed to reparse: {e}"));
    assert_eq!(
        reparsed, parsed,
        "item {i}: {name} lost or changed data across a parse -> serialize -> parse round-trip"
    );
}

#[test]
fn real_fixture_every_klv_item_round_trips_byte_identical() {
    let bytes = fixture_bytes();
    let items = collect_klv_items(&bytes).expect("collect KLV items from real fixture");
    assert!(
        !items.is_empty(),
        "fixture should contain at least one KLV item"
    );

    let mut header_metadata_sets_seen = 0usize;

    for (i, (_offset, item)) in items.iter().enumerate() {
        if is_fill_item_key(&item.key) {
            continue;
        }

        let klv = item.to_bytes();

        if PartitionPack::is_partition_key(&item.key) {
            let pp = PartitionPack::parse(&klv)
                .unwrap_or_else(|e| panic!("item {i}: PartitionPack parse failed: {e}"));
            let mut out = vec![0u8; pp.serialized_len()];
            pp.serialize_into(&mut out)
                .unwrap_or_else(|e| panic!("item {i}: PartitionPack serialize failed: {e}"));
            assert_eq!(
                out, klv,
                "item {i}: PartitionPack not byte-identical after round-trip"
            );
            continue;
        }

        if PrimerPack::is_primer_key(&item.key) {
            let primer = PrimerPack::parse(&klv)
                .unwrap_or_else(|e| panic!("item {i}: PrimerPack parse failed: {e}"));
            let mut out = vec![0u8; primer.serialized_len()];
            primer
                .serialize_into(&mut out)
                .unwrap_or_else(|e| panic!("item {i}: PrimerPack serialize failed: {e}"));
            assert_eq!(
                out, klv,
                "item {i}: PrimerPack not byte-identical after round-trip"
            );
            continue;
        }

        if RandomIndexPack::is_rip_key(&item.key) {
            let rip = RandomIndexPack::parse(&klv)
                .unwrap_or_else(|e| panic!("item {i}: RandomIndexPack parse failed: {e}"));
            let mut out = vec![0u8; rip.serialized_len()];
            rip.serialize_into(&mut out)
                .unwrap_or_else(|e| panic!("item {i}: RandomIndexPack serialize failed: {e}"));
            assert_eq!(
                out, klv,
                "item {i}: RandomIndexPack not byte-identical after round-trip"
            );
            continue;
        }

        if !is_local_set_key(&item.key) {
            // Essence Container element or Index Table segment (confirmed
            // NOT a local-set key, not merely assumed) — opaque, out of
            // scope for this crate. Nothing to assert.
            continue;
        }

        // Every local-set key MUST parse as a LocalSet and round-trip
        // byte-identically. No `if let ... else { skip }` — a failure here
        // fails the test.
        header_metadata_sets_seen += 1;
        let set = LocalSet::parse(&klv).unwrap_or_else(|e| {
            panic!(
                "item {i}: local-set key {:02x?} failed to parse as LocalSet: {e}",
                item.key
            )
        });
        let mut out = vec![0u8; set.serialized_len()];
        set.serialize_into(&mut out)
            .unwrap_or_else(|e| panic!("item {i}: LocalSet serialize failed: {e}"));
        assert_eq!(
            out,
            klv,
            "item {i}: LocalSet not byte-identical after round-trip (key {:02x?})",
            &item.key[..4]
        );

        // Sets with a typed representation in this crate must ALSO parse
        // and round-trip through that typed struct. This is the real
        // validation the previous version of this test never performed:
        // it only ever exercised the generic LocalSet path, so the typed
        // OP1a parsers (MaterialPackage/SourcePackage/Track/Sequence/
        // SourceClip/TimecodeComponent/FillerComponent) had zero real-world
        // byte coverage.
        match set.kind() {
            StructuralSetKind::Preface => assert_typed_round_trip::<Preface>(&klv, i, "Preface"),
            StructuralSetKind::Identification => {
                assert_typed_round_trip::<Identification>(&klv, i, "Identification");
            }
            StructuralSetKind::ContentStorage => {
                assert_typed_round_trip::<ContentStorage>(&klv, i, "ContentStorage");
            }
            StructuralSetKind::EssenceContainerData => {
                assert_typed_round_trip::<EssenceContainerData>(&klv, i, "EssenceContainerData");
            }
            StructuralSetKind::MaterialPackage => {
                assert_typed_round_trip::<MaterialPackage>(&klv, i, "MaterialPackage");
            }
            StructuralSetKind::SourcePackage => {
                assert_typed_round_trip::<SourcePackage>(&klv, i, "SourcePackage");
            }
            StructuralSetKind::TimelineTrack => {
                assert_typed_round_trip::<TimelineTrack>(&klv, i, "TimelineTrack");
            }
            StructuralSetKind::EventTrackDm => {
                assert_typed_round_trip::<EventTrack>(&klv, i, "EventTrack");
            }
            StructuralSetKind::StaticTrackDm => {
                assert_typed_round_trip::<StaticTrack>(&klv, i, "StaticTrack");
            }
            StructuralSetKind::Sequence => {
                assert_typed_round_trip::<Sequence>(&klv, i, "Sequence");
            }
            StructuralSetKind::SourceClip => {
                assert_typed_round_trip::<SourceClip>(&klv, i, "SourceClip");
            }
            StructuralSetKind::TimecodeComponent => {
                assert_typed_round_trip::<TimecodeComponent>(&klv, i, "TimecodeComponent");
            }
            StructuralSetKind::Filler => {
                assert_typed_round_trip::<FillerComponent>(&klv, i, "FillerComponent");
            }
            // Essence Descriptors (F.*), DM/Application Metadata, and
            // private/dark extensions are identified-but-generic by design
            // (see crate root docs) — the LocalSet round-trip above is the
            // whole contract for these kinds.
            _ => {}
        }
    }

    // Sanity: the fixture must actually exercise the local-set (Header
    // Metadata) path in depth, or this whole test would be vacuous. The
    // real fixture carries 27 Header Metadata Sets as of this writing;
    // guard with margin so an unrelated future fixture edit doesn't need
    // to touch this test, while still catching a collapse to near-zero.
    assert!(
        header_metadata_sets_seen >= 20,
        "expected at least 20 Header Metadata Sets in the real fixture, saw {header_metadata_sets_seen}"
    );
}
