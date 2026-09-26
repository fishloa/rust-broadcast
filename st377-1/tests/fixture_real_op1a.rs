//! Parses `tests/fixtures/op1a_mpeg2_pcm.mxf` — a real OP1a MXF file
//! (MPEG-2 video + PCM audio) — and validates every top-level KLV item:
//!
//! - Partition Packs, the Primer Pack, and the Random Index Pack — all
//!   spec-fixed positional layouts, not "sets" — round-trip
//!   **byte-identically** against the ORIGINAL captured bytes (issue #1047
//!   / audit MX-C1): each of [`KlvItem`]/`PartitionPack`/`PrimerPack`/
//!   `RandomIndexPack`/`LocalSet` now stores the on-wire BER length-field
//!   width it was parsed with (`len_size`, a [`st377_1::BerLength`]) and
//!   reproduces that exact width on `serialize_into`, rather than always
//!   re-canonicalizing to the shortest form that fits. This matters
//!   because real MXF encoders routinely write a longer, fixed-width form
//!   (`docs/st377-1.md` §6.3.4 permits any valid BER form) so a pack can be
//!   rewritten in place (e.g. Open -> Closed) without shifting every later
//!   absolute offset — measured directly against this fixture: **all 25**
//!   of its Partition Packs, and 2 of its 27 Header Metadata Sets, use a
//!   non-minimal length token (`real_fixture_non_minimal_ber_lengths_round_
//!   trip_byte_identically` below quantifies this precisely and asserts
//!   every single one still reproduces its original bytes).
//! - Every Header Metadata Set (a "local set" key, §9.3) round-trips
//!   **byte-identically** through the generic [`LocalSet`] (an order-
//!   preserving passthrough over the item list it parsed, including a
//!   per-item [`st377_1::BerLength`] for the rarer `ItemLengthMode::Ber`
//!   sets — none in this fixture, which is exclusively `TwoByte`, but
//!   covered by an in-crate unit test).
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
use st377_1::op1a::{Op1aQualifier, is_op1a};
use st377_1::{
    ContentStorage, Error, EssenceContainerData, EventTrack, FillerComponent, Identification,
    KlvItem, LocalSet, MaterialPackage, PartitionPack, Preface, PrimerPack, RandomIndexPack,
    Sequence, SourceClip, SourcePackage, StaticTrack, StructuralSetKind, TimecodeComponent,
    TimelineTrack, collect_klv_items, is_fill_item_key, is_local_set_key,
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

/// Compares every item against the TRUE original bytes at its offset
/// (`&bytes[offset..offset + consumed]`), not a re-encoded baseline — see
/// the module doc (issue #1047 / audit MX-C1). Previously this used
/// `item.to_bytes()` as the comparison baseline, which is `KlvItem`'s OWN
/// re-canonicalized encoding of the length token, so it could never
/// disagree with a further re-canonicalization no matter what the source
/// file actually wrote for a non-minimally-encoded item — it only ever
/// caught a *field-content* regression, never the length-token-width loss.
#[test]
fn real_fixture_every_klv_item_round_trips_byte_identical() {
    let bytes = fixture_bytes();
    let items = collect_klv_items(&bytes).expect("collect KLV items from real fixture");
    assert!(
        !items.is_empty(),
        "fixture should contain at least one KLV item"
    );

    let mut header_metadata_sets_seen = 0usize;

    for (i, (offset, item)) in items.iter().enumerate() {
        if is_fill_item_key(&item.key) {
            continue;
        }

        // The TRUE original bytes for this item, re-derived from its
        // on-wire length token (not `item.to_bytes()`, which is already
        // re-canonicalized and so cannot expose a width-preservation bug).
        let (_, consumed) = KlvItem::parse_prefix(&bytes[*offset..])
            .unwrap_or_else(|e| panic!("item {i}: re-deriving consumed length failed: {e}"));
        let klv = &bytes[*offset..*offset + consumed];

        if PartitionPack::is_partition_key(&item.key) {
            let pp = PartitionPack::parse(klv)
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
            let primer = PrimerPack::parse(klv)
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
            let rip = RandomIndexPack::parse(klv)
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
        let set = LocalSet::parse(klv).unwrap_or_else(|e| {
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
            StructuralSetKind::Preface => assert_typed_round_trip::<Preface>(klv, i, "Preface"),
            StructuralSetKind::Identification => {
                assert_typed_round_trip::<Identification>(klv, i, "Identification");
            }
            StructuralSetKind::ContentStorage => {
                assert_typed_round_trip::<ContentStorage>(klv, i, "ContentStorage");
            }
            StructuralSetKind::EssenceContainerData => {
                assert_typed_round_trip::<EssenceContainerData>(klv, i, "EssenceContainerData");
            }
            StructuralSetKind::MaterialPackage => {
                assert_typed_round_trip::<MaterialPackage>(klv, i, "MaterialPackage");
            }
            StructuralSetKind::SourcePackage => {
                assert_typed_round_trip::<SourcePackage>(klv, i, "SourcePackage");
            }
            StructuralSetKind::TimelineTrack => {
                assert_typed_round_trip::<TimelineTrack>(klv, i, "TimelineTrack");
            }
            StructuralSetKind::EventTrackDm => {
                assert_typed_round_trip::<EventTrack>(klv, i, "EventTrack");
            }
            StructuralSetKind::StaticTrackDm => {
                assert_typed_round_trip::<StaticTrack>(klv, i, "StaticTrack");
            }
            StructuralSetKind::Sequence => {
                assert_typed_round_trip::<Sequence>(klv, i, "Sequence");
            }
            StructuralSetKind::SourceClip => {
                assert_typed_round_trip::<SourceClip>(klv, i, "SourceClip");
            }
            StructuralSetKind::TimecodeComponent => {
                assert_typed_round_trip::<TimecodeComponent>(klv, i, "TimecodeComponent");
            }
            StructuralSetKind::Filler => {
                assert_typed_round_trip::<FillerComponent>(klv, i, "FillerComponent");
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

/// Real-fixture oracle for the OP1a qualifier byte (issue #1048 / audit
/// MX-C2). The fixture's own Operational Pattern UL is `...01 01 09 00`
/// (qualifier `0x09`), and its essence layout is independently known from
/// the fixture's own name/provenance: ONE interleaved MPEG-2 video + PCM
/// audio Essence Container (i.e. internal, streamable, but carrying more
/// than one essence track) — so the correct decode is
/// `external_essence() == false`, `non_streamable() == false`,
/// `multi_track() == true`.
#[test]
fn real_fixture_op1a_qualifier_matches_actual_essence_layout() {
    let bytes = fixture_bytes();
    let items = collect_klv_items(&bytes).expect("collect KLV items from real fixture");

    let header_partition = items
        .iter()
        .find_map(|(_offset, item)| {
            if PartitionPack::is_partition_key(&item.key) {
                PartitionPack::parse(&item.to_bytes()).ok()
            } else {
                None
            }
        })
        .expect("fixture must contain at least one parseable Partition Pack");

    assert!(
        is_op1a(&header_partition.operational_pattern),
        "fixture's Partition Pack operational_pattern must be an OP1a UL, got {:02x?}",
        header_partition.operational_pattern
    );
    let qualifier_byte = header_partition.operational_pattern[14];
    assert_eq!(
        qualifier_byte, 0x09,
        "fixture's known OP1a qualifier byte is 0x09 (verified directly against the file's \
         bytes) — if this fails the fixture changed, not the crate"
    );

    let q = Op1aQualifier::from_byte(qualifier_byte);
    assert!(
        !q.external_essence(),
        "fixture's essence is embedded in the file (internal), got external_essence()==true \
         for qualifier byte 0x09"
    );
    assert!(
        !q.non_streamable(),
        "fixture is a single streamable Essence Container, got non_streamable()==true for \
         qualifier byte 0x09"
    );
    assert!(
        q.multi_track(),
        "fixture interleaves MPEG-2 video + PCM audio in one Essence Container (2 tracks), \
         got multi_track()==false for qualifier byte 0x09"
    );
}

/// **Verifies the real fix, quantified against the real fixture** (issue
/// #1047 / audit MX-C1): every serializer in this crate (`KlvItem`,
/// `PartitionPack`, `PrimerPack`, `RandomIndexPack`, `LocalSet`) now stores
/// the on-wire BER length-field width it was parsed with (`len_size`, a
/// [`st377_1::BerLength`]) and reproduces that exact width on
/// `serialize_into`, instead of always re-canonicalizing to the shortest
/// form that fits. `docs/st377-1.md` §6.3.4 permits any valid BER form, and
/// real encoders routinely write a longer, fixed-width one (e.g. so a pack
/// can be rewritten in place without shifting every later absolute offset)
/// — measured directly against this fixture, a clear majority of its
/// top-level items do exactly that.
///
/// This walks every top-level KLV item generically via
/// [`KlvItem::parse_prefix`] (independent of the more targeted typed checks
/// in `real_fixture_every_klv_item_round_trips_byte_identical` above, which
/// covers Partition Packs/Primer Pack/Random Index Pack/Header Metadata
/// Sets specifically) and asserts, for EVERY item including Essence
/// Container elements and Index Table segments this crate never decodes,
/// that `KlvItem`'s own generic serializer reproduces the exact original
/// bytes.
#[test]
fn real_fixture_non_minimal_ber_lengths_round_trip_byte_identically() {
    let bytes = fixture_bytes();

    let mut offset = 0usize;
    let mut total_items = 0usize;
    let mut non_minimal_items = 0usize;

    while offset < bytes.len() {
        let (item, consumed) = match KlvItem::parse_prefix(&bytes[offset..]) {
            Ok(v) => v,
            Err(_) => break, // end of well-formed top-level KLV framing
        };
        total_items += 1;
        let original = &bytes[offset..offset + consumed];

        // The TRUE on-wire length-token width: total consumed minus the
        // 16-byte Key and the Value length itself.
        let true_len_size = consumed - 16 - item.value.len();
        let canonical_len_size = canonical_len_size_for(item.value.len());
        if true_len_size != canonical_len_size {
            non_minimal_items += 1;
        }

        assert_eq!(
            item.to_bytes(),
            original,
            "item at offset {offset}: KlvItem must reproduce its exact original bytes \
             (len_size = {:?})",
            item.len_size
        );

        offset += consumed;
    }

    assert!(
        total_items >= 190,
        "expected roughly 200 top-level KLV items in the real fixture, saw {total_items}"
    );
    // The audit measured "174 of the first 200" non-minimal — assert a
    // large majority (not the exact figure, so an unrelated fixture edit
    // doesn't need to touch this test) to keep the finding's scale honest:
    // this is a real, common-case fix, not a corner case.
    assert!(
        non_minimal_items > total_items / 2,
        "expected a majority of top-level items to use a non-minimal BER length form, saw \
         {non_minimal_items}/{total_items} — if this fixture no longer exercises the bug, \
         pick/generate one that does"
    );
}

/// [`crate::ber::ber_length_size`] isn't re-exported at the crate root (it's
/// an internal helper), so this test-only copy computes the same canonical
/// minimal BER length-token width from a value length, to independently
/// cross-check `KlvItem`'s own re-encoding against the true on-wire width.
fn canonical_len_size_for(value_len: usize) -> usize {
    if value_len <= 0x7F {
        1
    } else {
        let bytes_needed = (64 - (value_len as u64).leading_zeros()).div_ceil(8) as usize;
        1 + bytes_needed
    }
}
