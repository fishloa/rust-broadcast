//! Typed metadata on real files: the iTunes tags of three `.m4a` fixtures, the
//! QuickTime `udta` of a `.mov`, Apple's keyed `mdta` metadata, the GoPro
//! vendor atoms, byte-exact box round trips, mutations and hostile input.

use std::path::PathBuf;

use broadcast_common::{Parse, Serialize};
use isobmff::metadata::{Chpl, Ilst, ItemKey, Keys, Meta, Metadata, MetadataItem, Udta};
use isobmff::{Limits, MovieBox, Reader, ReaderConfig, SampleOrder, Step};

/// Read a private fixture, skipping only when the private repo is absent.
fn fixture(rel: &str) -> Option<Vec<u8>> {
    let path: PathBuf = test_fixtures::require(&format!("mp4/{rel}"), "metadata")?;
    Some(std::fs::read(path).expect("fixture readable"))
}

fn movie(bytes: &[u8]) -> MovieBox {
    let mut found = None;
    for item in isobmff::BoxIter::new(bytes) {
        let item = item.expect("top-level box");
        if item.four_cc() == *b"moov" {
            found = Some(MovieBox::parse(item.bytes).expect("moov parses"));
        }
    }
    found.expect("fixture has a moov")
}

fn text(metadata: &Metadata, fourcc: &[u8; 4]) -> String {
    metadata
        .text(&ItemKey::FourCc(*fourcc))
        .unwrap_or_else(|| panic!("missing text item {fourcc:?}"))
        .to_string()
}

fn pair(metadata: &Metadata, fourcc: &[u8; 4]) -> (u16, u16) {
    for item in &metadata.items {
        if let MetadataItem::Pair { key, index, total } = item
            && key == &ItemKey::FourCc(*fourcc)
        {
            return (*index, *total);
        }
    }
    panic!("missing pair item {fourcc:?}");
}

/// Every `udta` and `meta` box in a file, header included, at any depth a
/// movie or track uses.
fn metadata_boxes(bytes: &[u8]) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let mut udtas = Vec::new();
    let mut metas = Vec::new();
    let mut visit = |item: isobmff::BoxItem<'_>| {
        collect_children(&item, &mut udtas, &mut metas);
    };
    for item in isobmff::BoxIter::new(bytes) {
        let Ok(item) = item else { continue };
        if item.four_cc() == *b"moov" {
            for child in item.children() {
                let Ok(child) = child else { continue };
                visit(child);
            }
        }
    }
    (udtas, metas)
}

fn collect_children(
    item: &isobmff::BoxItem<'_>,
    udtas: &mut Vec<Vec<u8>>,
    metas: &mut Vec<Vec<u8>>,
) {
    match &item.four_cc() {
        b"udta" => {
            udtas.push(item.bytes.to_vec());
            for child in item.children() {
                let Ok(child) = child else { continue };
                if child.four_cc() == *b"meta" {
                    metas.push(child.bytes.to_vec());
                }
            }
        }
        b"meta" => metas.push(item.bytes.to_vec()),
        b"trak" => {
            for child in item.children() {
                let Ok(child) = child else { continue };
                collect_children(&child, udtas, metas);
            }
        }
        _ => {}
    }
}

#[test]
fn lavf_tagged_reads_every_itunes_tag() {
    let Some(bytes) = fixture("tags/lavf-tagged.m4a") else {
        return;
    };
    let movie = movie(&bytes);
    let metadata = movie
        .metadata(&Limits::default())
        .expect("metadata parses")
        .expect("the file has metadata");
    assert_eq!(text(&metadata, b"\xa9nam"), "Test Tone");
    assert_eq!(text(&metadata, b"\xa9ART"), "rust-broadcast");
    assert_eq!(text(&metadata, b"\xa9alb"), "Fixtures");
    assert_eq!(text(&metadata, b"\xa9day"), "2026");
    assert_eq!(text(&metadata, b"\xa9gen"), "Test");
    assert_eq!(text(&metadata, b"\xa9cmt"), "generated");
    assert!(!text(&metadata, b"\xa9too").is_empty());
    assert_eq!(pair(&metadata, b"trkn"), (3, 12));
    assert_eq!(pair(&metadata, b"disk"), (1, 2));
}

#[test]
fn afconvert_freeform_item_keeps_its_namespace_and_name() {
    let Some(bytes) = fixture("tags/afconvert-aac.m4a") else {
        return;
    };
    let movie = movie(&bytes);
    let metadata = movie
        .metadata(&Limits::default())
        .expect("metadata parses")
        .expect("the file has metadata");
    let freeform = metadata
        .items
        .iter()
        .find_map(|item| match item {
            MetadataItem::Freeform { mean, name, values } => Some((mean, name, values)),
            _ => None,
        })
        .expect("a freeform item");
    assert_eq!(freeform.0, "com.apple.iTunes");
    assert_eq!(freeform.1, "iTunSMPB");
    let value = freeform.2.first().expect("one value").text().expect("text");
    assert!(value.starts_with(" 00000000"));
}

#[test]
fn mp4box_chapters_read_two_starts() {
    let Some(bytes) = fixture("tags/mp4box-chapters-nero.m4a") else {
        return;
    };
    let movie = movie(&bytes);
    let metadata = movie
        .metadata(&Limits::default())
        .expect("metadata parses")
        .expect("the file has metadata");
    let chapters = metadata
        .items
        .iter()
        .find_map(|item| match item {
            MetadataItem::Chapters(chapters) => Some(chapters),
            _ => None,
        })
        .expect("a chapter list");
    assert_eq!(chapters.len(), 2);
    assert_eq!(chapters[0].start_100ns, 0);
    assert_eq!(chapters[0].title, "Intro");
    assert_eq!(chapters[1].start_100ns, 15_000_000);
    assert!((chapters[1].start_seconds() - 1.5).abs() < f64::EPSILON);
}

#[test]
fn quicktime_udta_title_reads() {
    let Some(bytes) = fixture("quicktime/lavf-aac-tmcd.mov") else {
        return;
    };
    let movie = movie(&bytes);
    let metadata = movie
        .metadata(&Limits::default())
        .expect("metadata parses")
        .expect("the file has metadata");
    assert_eq!(text(&metadata, b"\xa9nam"), "Test Movie");
}

#[test]
fn iphone_keyed_metadata_is_mdta_and_parses_location() {
    let Some(bytes) = fixture("quicktime/iphone-x-live-photo.mov") else {
        return;
    };
    let movie = movie(&bytes);
    let (_, metas) = metadata_boxes(&bytes);
    let meta = Meta::parse(metas.first().expect("a meta box")).expect("meta parses");
    assert_eq!(meta.handler(), Some(*b"mdta"));
    let metadata = movie
        .metadata(&Limits::default())
        .expect("metadata parses")
        .expect("the file has metadata");
    let mdta = |name: &str| ItemKey::Mdta(name.into());
    assert_eq!(
        metadata.text(&mdta("com.apple.quicktime.make")),
        Some("Apple")
    );
    assert_eq!(
        metadata.text(&mdta("com.apple.quicktime.model")),
        Some("iPhone X")
    );
    assert!(
        metadata
            .text(&mdta("com.apple.quicktime.software"))
            .is_some_and(|value| !value.is_empty())
    );
    assert!(
        metadata
            .text(&mdta("com.apple.quicktime.creationdate"))
            .is_some_and(|value| value.starts_with("2018-"))
    );
    assert!(
        metadata
            .text(&mdta("com.apple.quicktime.content.identifier"))
            .is_some_and(|value| value.len() > 8)
    );
    let location = metadata
        .items
        .iter()
        .find_map(|item| match item {
            MetadataItem::Text { key, value, .. }
                if key == &mdta("com.apple.quicktime.location.ISO6709") =>
            {
                Some(isobmff::metadata::Iso6709::new(value.clone()))
            }
            _ => None,
        })
        .expect("a location string");
    assert!((location.latitude().expect("latitude") - 24.0742).abs() < 0.001);
    assert!((location.longitude().expect("longitude") + 76.3873).abs() < 0.001);
}

#[test]
fn gopro_vendor_atoms_stay_opaque() {
    for name in [
        "camera/gopro-hero6-ble.mp4",
        "camera/gopro-karma.mp4",
        "camera/gopro-max-heromode.mp4",
    ] {
        let Some(bytes) = fixture(name) else {
            continue;
        };
        let movie = movie(&bytes);
        let metadata = movie
            .metadata(&Limits::default())
            .expect("metadata parses")
            .expect("the camera file has udta");
        assert!(
            metadata.items.iter().any(
                |item| matches!(item, MetadataItem::Opaque(boxed) if boxed.box_type == *b"FIRM")
            ),
            "{name}: FIRM must stay an opaque item"
        );
    }
}

#[test]
fn gopro_xyz_location_parses() {
    let Some(bytes) = fixture("camera/gopro-max-heromode.mp4") else {
        return;
    };
    let movie = movie(&bytes);
    let metadata = movie
        .metadata(&Limits::default())
        .expect("metadata parses")
        .expect("the camera file has udta");
    let location = metadata
        .items
        .iter()
        .find_map(|item| match item {
            MetadataItem::Location(location) => Some(location),
            _ => None,
        })
        .expect("a ©xyz item");
    assert!((location.latitude().expect("latitude") - 33.1268).abs() < 0.001);
    assert!((location.longitude().expect("longitude") + 117.3273).abs() < 0.001);
}

#[test]
fn every_udta_and_meta_box_round_trips_byte_exactly() {
    for name in [
        "tags/lavf-tagged.m4a",
        "tags/afconvert-aac.m4a",
        "tags/mp4box-chapters-nero.m4a",
        "quicktime/lavf-aac-tmcd.mov",
        "quicktime/iphone-x-live-photo.mov",
        "camera/gopro-hero6-ble.mp4",
        "camera/gopro-karma.mp4",
        "camera/gopro-max-heromode.mp4",
    ] {
        let Some(bytes) = fixture(name) else {
            continue;
        };
        let (udtas, metas) = metadata_boxes(&bytes);
        assert!(
            !udtas.is_empty() || !metas.is_empty(),
            "{name}: no metadata box"
        );
        for udta in udtas {
            let parsed = Udta::parse(&udta).expect("udta parses");
            assert_eq!(
                parsed.try_to_bytes().expect("udta writes"),
                udta,
                "{name}: udta"
            );
        }
        for meta in metas {
            let parsed = Meta::parse(&meta).expect("meta parses");
            assert_eq!(
                parsed.try_to_bytes().expect("meta writes"),
                meta,
                "{name}: meta"
            );
        }
    }
}

#[test]
fn the_reader_surfaces_metadata_once_after_the_tracks() {
    let Some(bytes) = fixture("tags/lavf-tagged.m4a") else {
        return;
    };
    let mut reader = Reader::new(
        ReaderConfig::default()
            .with_total_len(Some(bytes.len() as u64))
            .with_order(SampleOrder::File),
    );
    let mut saw_tracks = false;
    let mut metadata: Option<isobmff::metadata::MovieMetadata> = None;
    loop {
        match reader.step().expect("reader step") {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let end = usize::try_from(range.end()).expect("end");
                reader
                    .supply(range.offset, &bytes[start..end])
                    .expect("supply");
            }
            Step::Tracks(_) => saw_tracks = true,
            Step::Metadata(found) => {
                assert!(saw_tracks, "metadata must follow Tracks");
                metadata = Some(found.clone());
            }
            Step::Done => break,
            _ => {}
        }
    }
    let metadata = metadata.expect("Step::Metadata");
    assert_eq!(text(&metadata.movie, b"\xa9nam"), "Test Tone");
}

fn boxed(fourcc: [u8; 4], body: &[u8]) -> Vec<u8> {
    let size = u32::try_from(8 + body.len()).expect("fits");
    let mut out = size.to_be_bytes().to_vec();
    out.extend_from_slice(&fourcc);
    out.extend_from_slice(body);
    out
}

fn top_box(bytes: &[u8], fourcc: [u8; 4]) -> (usize, usize) {
    for item in isobmff::BoxIter::new(bytes) {
        let item = item.expect("top box");
        if item.four_cc() == fourcc {
            return (item.start, item.end - item.start);
        }
    }
    panic!("top-level {fourcc:?} not found");
}

fn child_box(
    bytes: &[u8],
    start: usize,
    size: usize,
    fourcc: [u8; 4],
    skip: usize,
) -> (usize, usize) {
    let body_start = start + 8 + skip;
    let body = &bytes[body_start..start + size];
    for item in isobmff::BoxIter::new(body) {
        let item = item.expect("child box");
        if item.four_cc() == fourcc {
            return (body_start + item.start, item.end - item.start);
        }
    }
    panic!("child {fourcc:?} not found");
}

fn meta_skip(bytes: &[u8], start: usize) -> usize {
    let body = start + 8;
    if &bytes[body + 4..body + 8] == b"hdlr" {
        0
    } else {
        4
    }
}

#[test]
fn flipping_the_trkn_total_changes_the_parsed_pair() {
    let Some(mut bytes) = fixture("tags/lavf-tagged.m4a") else {
        return;
    };
    let moov = top_box(&bytes, *b"moov");
    let udta = child_box(&bytes, moov.0, moov.1, *b"udta", 0);
    let meta = child_box(&bytes, udta.0, udta.1, *b"meta", 0);
    let skip = meta_skip(&bytes, meta.0);
    let ilst = child_box(&bytes, meta.0, meta.1, *b"ilst", skip);
    let trkn = child_box(&bytes, ilst.0, ilst.1, *b"trkn", 0);
    let data = child_box(&bytes, trkn.0, trkn.1, *b"data", 0);
    let total = data.0 + 8 + 8 + 4;
    assert_eq!(&bytes[total..total + 2], &[0, 12]);
    bytes[total + 1] = 11;
    let metadata = movie(&bytes)
        .metadata(&Limits::default())
        .expect("metadata parses")
        .expect("metadata present");
    assert_eq!(pair(&metadata, b"trkn"), (3, 11));
}

#[test]
fn flipping_a_keys_index_changes_the_resolved_key() {
    let Some(mut bytes) = fixture("quicktime/iphone-x-live-photo.mov") else {
        return;
    };
    let moov = top_box(&bytes, *b"moov");
    let meta = child_box(&bytes, moov.0, moov.1, *b"meta", 0);
    let skip = meta_skip(&bytes, meta.0);
    let ilst = child_box(&bytes, meta.0, meta.1, *b"ilst", skip);
    let first_item = ilst.0 + 8;
    assert_eq!(&bytes[first_item + 4..first_item + 8], &[0, 0, 0, 1]);
    bytes[first_item + 7] = 2;
    let metadata = movie(&bytes)
        .metadata(&Limits::default())
        .expect("metadata parses")
        .expect("metadata present");
    let first = metadata.items.first().expect("an item");
    assert_eq!(
        first.key(),
        Some(&ItemKey::Mdta(
            "com.apple.quicktime.location.ISO6709".into()
        ))
    );
}

#[test]
fn a_data_atom_claiming_four_gib_is_a_typed_error() {
    let mut data = Vec::new();
    data.extend_from_slice(&1u32.to_be_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&0x1_0000_0010u64.to_be_bytes());
    data.extend_from_slice(&1u32.to_be_bytes());
    data.extend_from_slice(&0u32.to_be_bytes());
    let item = boxed(*b"\xa9nam", &data);
    let ilst = boxed(*b"ilst", &item);
    assert!(Ilst::parse(&ilst).is_err());
}

#[test]
fn a_keys_table_claiming_two_to_the_32_entries_is_a_typed_error() {
    let mut body = vec![0u8; 4];
    body.extend_from_slice(&u32::MAX.to_be_bytes());
    let keys = boxed(*b"keys", &body);
    assert!(Keys::parse(&keys).is_err());
}

#[test]
fn a_chpl_claiming_255_chapters_in_ten_bytes_is_a_typed_error() {
    let mut body = vec![1u8, 0, 0, 0, 0, 0, 0, 0, 255];
    body.extend_from_slice(&[0u8; 10]);
    let chpl = boxed(*b"chpl", &body);
    assert!(Chpl::parse(&chpl).is_err());
}

#[test]
fn iso6709_parses_signed_components_and_keeps_the_string() {
    let location = isobmff::metadata::Iso6709::new("+24.0742-076.3873+002.009/");
    assert_eq!(location.as_str(), "+24.0742-076.3873+002.009/");
    assert!((location.latitude().expect("lat") - 24.0742).abs() < 0.001);
    assert!((location.longitude().expect("lon") + 76.3873).abs() < 0.001);
    assert!((location.altitude().expect("alt") - 2.009).abs() < 0.001);
    assert_eq!(isobmff::metadata::Iso6709::new("garbage").latitude(), None);
}

#[test]
fn metadata_writes_back_to_a_udta_box() {
    let Some(bytes) = fixture("tags/lavf-tagged.m4a") else {
        return;
    };
    let movie = movie(&bytes);
    let mut metadata = movie
        .metadata(&Limits::default())
        .expect("metadata parses")
        .expect("metadata present");
    metadata
        .items
        .retain(|item| item.key() != Some(&ItemKey::FourCc(*b"\xa9nam")));
    metadata.items.push(MetadataItem::Text {
        key: ItemKey::FourCc(*b"\xa9nam"),
        value: "Written".into(),
        locale: isobmff::metadata::TextLocale::Itunes {
            data_type: isobmff::metadata::ilst::DATA_TYPE_UTF8,
            country: 0,
            language: 0,
        },
    });
    let written = metadata.to_udta_bytes().expect("metadata writes");
    let udta = Udta::parse(&written).expect("the written udta parses");
    let read = Metadata::from_udta(&udta);
    assert_eq!(text(&read, b"\xa9nam"), "Written");
    assert_eq!(pair(&read, b"trkn"), (3, 12));
    assert_eq!(text(&read, b"\xa9ART"), "rust-broadcast");
}
