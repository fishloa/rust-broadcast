//! A box that is not needed to locate samples degrades to raw carriage
//! instead of refusing the file: one finding names it, its bytes stay exactly
//! where they were, and reading continues.
//!
//! Each case takes a committed fixture and breaks one byte of one
//! non-structural box, so its typed parse fails where it used to fail the whole
//! read (or, for a track-level metadata atom, lose the metadata silently).

use broadcast_core::damage::Damage;
use isobmff::error::OpaqueReason;
use isobmff::image_brands::ImageFinding;
use isobmff::item_reader::{ItemReader, ItemReaderConfig, ItemStep, StillImage};
use isobmff::reader::{Mode, Reader, ReaderConfig, Source, Step};
use test_fixtures as fixtures;

/// Offset of a box four-CC within its header.
const TYPE_OFFSET: usize = 4;
/// Width of a box header.
const HEADER: usize = 8;
/// Width of a FullBox's version and flags.
const FULLBOX: usize = 4;
/// A value past every count a fixture states.
const HUGE_COUNT: [u8; 2] = [0xFF, 0xFF];
/// Width of the 16-bit language field a 3GPP metadata atom carries after its
/// version/flags word.
const TWO_BYTES: usize = 2;

fn load_item(bytes: &[u8]) -> (StillImage, Vec<Damage>) {
    let config = ItemReaderConfig::default().with_total_len(Some(bytes.len() as u64));
    let mut reader = ItemReader::new(config);
    let mut damages = Vec::new();
    loop {
        match reader.step().expect("step") {
            ItemStep::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let len = usize::try_from(range.len).expect("length");
                reader
                    .supply(range.offset, &bytes[start..start + len])
                    .expect("supply");
            }
            ItemStep::Loaded(image) => {
                let image = image.clone();
                assert!(matches!(reader.step().expect("step"), ItemStep::Done));
                return (image, damages);
            }
            ItemStep::Damage(damage) => damages.push(damage),
            ItemStep::Done => panic!("done without a model"),
            _ => panic!("unexpected step"),
        }
    }
}

/// The first `size|type` box whose four-CC is `fourcc`, anywhere in `bytes`.
fn find_box(bytes: &[u8], fourcc: &[u8; 4]) -> usize {
    let mut at = 0usize;
    while at + HEADER <= bytes.len() {
        if &bytes[at + TYPE_OFFSET..at + HEADER] == fourcc {
            let size = u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
                as usize;
            if size >= HEADER && at + size <= bytes.len() {
                return at;
            }
        }
        at += 1;
    }
    panic!("no {:?} box", core::str::from_utf8(fourcc));
}

fn read_movie(bytes: &[u8], mode: Mode) -> Result<Vec<Damage>, isobmff::Error> {
    let config = ReaderConfig::default()
        .with_total_len(Some(bytes.len() as u64))
        .with_source(Source::Seekable)
        .with_mode(mode);
    let mut reader = Reader::new(config);
    let mut warnings = Vec::new();
    loop {
        match reader.step()? {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let len = usize::try_from(range.len).expect("length");
                reader.supply(range.offset, &bytes[start..start + len])?;
            }
            Step::Damage(damage) => warnings.push(damage),
            Step::Done => break,
            _ => {}
        }
    }
    Ok(warnings)
}

fn one_unrepresentable(name: &str, warnings: &[Damage], box_type: [u8; 4]) -> String {
    let reported: Vec<&Damage> = warnings
        .iter()
        .filter(|damage| {
            damage.kind() == broadcast_core::damage::DamageKind::MetadataUninterpreted
                && damage.recoverable_bytes()
        })
        .collect();
    assert_eq!(reported.len(), 1, "{name}: one report, got {warnings:?}");
    let damage = reported[0];
    assert_eq!(
        core::str::from_utf8(&damage.fourcc().expect("a four-character code")).expect("fourcc"),
        core::str::from_utf8(&box_type).expect("fourcc"),
        "{name}: the report names the box"
    );
    assert!(damage.location().byte_range().is_some(), "{name}: located");
    damage.what().to_owned()
}

/// The reports of a box carried uninterpreted, as the step channel published
/// them.
fn carried_damages(damages: &[Damage]) -> Vec<&Damage> {
    damages
        .iter()
        .filter(|damage| {
            damage.kind() == broadcast_core::damage::DamageKind::MetadataUninterpreted
                && damage.recoverable_bytes()
        })
        .collect()
}

/// The first child of a FullBox container starting at `box`.
fn first_child(box_start: usize) -> usize {
    box_start + HEADER + FULLBOX
}

/// The `iref` child's `reference_count`: its `from_item_ID` is 16 bits in a
/// version-0 `iref` and 32 in a version-1 one (§8.11.12.2).
fn iref_count_at(bytes: &[u8], iref: usize) -> usize {
    let child = first_child(iref);
    let version = bytes[iref + HEADER];
    child + HEADER + if version == 1 { 4 } else { 2 }
}

#[test]
fn a_derived_item_body_that_cannot_be_typed_keeps_the_item() {
    let name = "avif/gainmap-swapped.avif";
    let original = fixtures::read(name);
    let (image, _) = load_item(&original);
    let tmap = image
        .items()
        .iter()
        .find(|item| item.item_type() == isobmff::item::ItemType::Tmap)
        .expect("a tone-map item");
    let offset = tmap.data_ranges()[0].offset as usize;
    let mut bytes = original.clone();
    bytes[offset] = 0xFF;
    let (image, damages) = load_item(&bytes);
    let findings = image.structure_findings();
    let reported: Vec<&ImageFinding> = findings
        .iter()
        .filter(|finding| matches!(finding, ImageFinding::UnrepresentableBox { .. }))
        .collect();
    assert_eq!(reported.len(), 1, "{name}: one finding, got {findings:?}");
    assert_eq!(
        carried_damages(&damages).len(),
        reported.len(),
        "{name}: the step channel publishes the same fault: {damages:?}"
    );
    assert_eq!(
        image.reemit_into(&bytes).expect("re-emit"),
        bytes,
        "{name}: the body is still carried"
    );
}

#[test]
fn an_entity_group_that_cannot_be_typed_is_carried() {
    let name = "avif/gainmap-multichannel.avif";
    let original = fixtures::read(name);
    let mut bytes = original.clone();
    let grpl = find_box(&bytes, b"grpl");
    // `grpl` is a plain container: its children start right after its header.
    let child = grpl + HEADER;
    // `num_entities_in_group` is the second field of the group's body.
    let count_at = child + HEADER + FULLBOX + 4;
    bytes[count_at..count_at + 2].copy_from_slice(&HUGE_COUNT);
    let (image, damages) = load_item(&bytes);
    let findings = image.structure_findings();
    let reported: Vec<&ImageFinding> = findings
        .iter()
        .filter(|finding| matches!(finding, ImageFinding::UnrepresentableBox { .. }))
        .collect();
    assert_eq!(reported.len(), 1, "{name}: one finding, got {findings:?}");
    assert_eq!(
        carried_damages(&damages).len(),
        reported.len(),
        "{name}: the step channel publishes the same fault: {damages:?}"
    );
    let ImageFinding::UnrepresentableBox { offset, .. } = *reported[0] else {
        unreachable!("filtered above")
    };
    assert_eq!(
        usize::try_from(offset).expect("offset"),
        child,
        "{name}: offset"
    );
    assert_eq!(
        image.reemit_into(&bytes).expect("re-emit"),
        bytes,
        "{name}: the group is carried byte-exact"
    );
}

#[test]
fn an_item_reference_that_cannot_be_typed_is_carried() {
    for name in ["avif/alpha.avif", "heif/thumb.heic", "heif/tiled.heic"] {
        let original = fixtures::read(name);
        let mut bytes = original.clone();
        let iref = find_box(&bytes, b"iref");
        let count_at = iref_count_at(&bytes, iref);
        bytes[count_at..count_at + 2].copy_from_slice(&HUGE_COUNT);
        let (image, damages) = load_item(&bytes);
        let findings = image.structure_findings();
        let reported: Vec<&ImageFinding> = findings
            .iter()
            .filter(|finding| matches!(finding, ImageFinding::UnrepresentableBox { .. }))
            .collect();
        assert_eq!(reported.len(), 1, "{name}: one finding, got {findings:?}");
        assert_eq!(
            carried_damages(&damages).len(),
            reported.len(),
            "{name}: the step channel publishes the same fault: {damages:?}"
        );
        let ImageFinding::UnrepresentableBox { offset, .. } = *reported[0] else {
            unreachable!("filtered above")
        };
        assert_eq!(
            usize::try_from(offset).expect("offset"),
            first_child(iref),
            "{name}: the finding names the reference"
        );
        assert_eq!(
            image.reemit_into(&bytes).expect("re-emit"),
            bytes,
            "{name}: the reference is carried byte-exact"
        );
    }
}

#[test]
fn a_track_group_that_cannot_be_typed_is_carried() {
    let name = "mp4/boxes/track-group-ster.mp4";
    let original = fixtures::read(name);
    let mut bytes = original.clone();
    let trgr = find_box(&bytes, b"trgr");
    // `trgr` is a plain container: its children start right after its header.
    let group = trgr + HEADER;
    // A `ster` group's declared size, cut below its own fixed fields.
    bytes[group..group + 4].copy_from_slice(&12u32.to_be_bytes());
    let warnings = read_movie(&bytes, Mode::Permissive).expect("permissive read");
    let reason = one_unrepresentable(name, &warnings, *b"trgr");
    assert_eq!(
        reason,
        OpaqueReason::BufferTooShort.name(),
        "{name}: the reason"
    );
    assert!(
        read_movie(&bytes, Mode::Strict).is_err(),
        "{name}: strict still refuses it"
    );
}

#[test]
fn an_edit_list_that_cannot_be_typed_is_carried() {
    let name = "mp4/progressive/av-prog.mp4";
    let original = fixtures::read(name);
    let mut bytes = original.clone();
    let elst = find_box(&bytes, b"elst");
    let count_at = elst + HEADER + FULLBOX;
    bytes[count_at..count_at + 2].copy_from_slice(&HUGE_COUNT);
    let warnings = read_movie(&bytes, Mode::Permissive).expect("permissive read");
    one_unrepresentable(name, &warnings, *b"edts");
    assert!(
        read_movie(&bytes, Mode::Strict).is_err(),
        "{name}: strict still refuses it"
    );
}

/// A 3GPP user-data atom whose body is not the syntax its clause gives it is
/// carried like any other: the atoms this crate types reach `udta` through the
/// same tolerant walk, so one that cannot be typed costs that atom.
#[test]
fn a_threegpp_metadata_atom_that_cannot_be_typed_is_carried() {
    let name = "mp4/boxes/loudness.m4a";
    let mut bytes = fixtures::read(name);
    let ludt = find_box(&bytes, b"ludt");
    // Re-label the loudness box as 3GPP keywords and give it a keyword count
    // its body cannot hold, so the typed parse fails.
    bytes[ludt + TYPE_OFFSET..ludt + HEADER].copy_from_slice(b"kywd");
    bytes[ludt + HEADER + FULLBOX + TWO_BYTES] = HUGE_COUNT[0];
    let warnings = read_movie(&bytes, Mode::Permissive).expect("permissive read");
    one_unrepresentable(name, &warnings, *b"kywd");
    assert!(
        read_movie(&bytes, Mode::Strict).is_err(),
        "{name}: strict still refuses it"
    );
}

#[test]
fn a_metadata_atom_that_cannot_be_typed_is_carried() {
    let name = "mp4/boxes/loudness.m4a";
    let mut bytes = fixtures::read(name);
    let ludt = find_box(&bytes, b"ludt");
    // A version the clause does not model, read as the version-2 form.
    bytes[ludt + HEADER] = 0x7F;
    let warnings = read_movie(&bytes, Mode::Permissive).expect("permissive read");
    one_unrepresentable(name, &warnings, *b"ludt");
    assert!(
        read_movie(&bytes, Mode::Strict).is_err(),
        "{name}: strict still refuses it"
    );
}
