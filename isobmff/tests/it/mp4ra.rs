//! MP4RA registration coverage: every code in the 30 tables behind mp4ra.org,
//! partitioned against what this crate types.
//!
//! The CSVs are read through [`test_fixtures::spec_string`], so the suite skips
//! loudly without the private checkout and hard-fails when it is present but an
//! entry is missing. The partition lives in [`isobmff::coverage::mp4ra`]; this
//! module proves each typed row and checks the partition is exact.
//!
//! A typed row proves itself: a structured code parses a minimal (or real)
//! instance through the crate's parser, an enumerated identifier maps through
//! its typed enum and back. Moving a code into a typed table without a working
//! parse fails here; leaving it in two lists fails; a README count that drifts
//! fails.

use std::collections::BTreeSet;

use broadcast_core::{Parse, Serialize};
use isobmff::coverage::TYPED_BOXES;
use isobmff::coverage::mp4ra::{
    self, NotApplicable, TYPED_NOT_REGISTERED, Table, TypedKind, TypedRow,
};
use test_fixtures as fixtures;

/// Where the MP4RA tables live in the private checkout.
const MP4RA_DIR: &str = "mpeg/isobmff/mp4ra";
/// Who the fixture loader should name in its skip message.
const WHO: &str = "isobmff MP4RA coverage";

/// The four-CC a code names, space-padded to four bytes.
fn fourcc(code: &str) -> [u8; 4] {
    let mut out = [b' '; 4];
    for (slot, byte) in out.iter_mut().zip(code.bytes()) {
        *slot = byte;
    }
    out
}

fn boxed(code: &str, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    let size = u32::try_from(8 + body.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(&fourcc(code));
    out.extend_from_slice(body);
    out
}

fn full(version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + body.len());
    out.push(version);
    out.extend_from_slice(&flags.to_be_bytes()[1..]);
    out.extend_from_slice(body);
    out
}

/// The distinct codes of one CSV, trailing spaces trimmed.
fn csv_codes(name: &str) -> Option<BTreeSet<String>> {
    let entry = format!("{MP4RA_DIR}/{name}.csv");
    let text = fixtures::spec_string(&entry, WHO)?;
    let mut out = BTreeSet::new();
    let mut lines = text.lines();
    let header = lines.next().unwrap_or_default();
    if !header.starts_with("code") {
        return Some(out);
    }
    for line in lines {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            continue;
        }
        let code = line
            .split(',')
            .next()
            .unwrap_or_default()
            .replace("$20", " ");
        out.insert(code.trim_end().to_string());
    }
    Some(out)
}

fn table(name: &str) -> &'static Table {
    mp4ra::TABLES
        .iter()
        .find(|t| t.csv == name)
        .unwrap_or_else(|| panic!("no table {name}"))
}

/// The typed codes a table claims: its typed kinds' rows, filtered to its CSV.
fn typed_in(table: &Table, csv: &BTreeSet<String>) -> BTreeSet<String> {
    let mut rows = BTreeSet::new();
    for kind in table.typed {
        for row in kind.rows() {
            if csv.contains(row.code) {
                rows.insert(row.code.to_string());
            }
        }
    }
    rows
}

fn pinned(codes: &[&'static str]) -> BTreeSet<String> {
    codes.iter().map(|c| (*c).to_string()).collect()
}

fn na_codes(items: &[NotApplicable]) -> BTreeSet<String> {
    items.iter().map(|i| i.code.to_string()).collect()
}

pub fn registry_tables() -> Option<Vec<(String, BTreeSet<String>)>> {
    let mut out = Vec::new();
    for table in mp4ra::TABLES {
        out.push((table.csv.to_string(), csv_codes(table.csv)?));
    }
    Some(out)
}

#[test]
fn every_table_partitions_its_codes_exactly_once() {
    let Some(loaded) = registry_tables() else {
        eprintln!("SKIP isobmff MP4RA coverage: private checkout absent");
        return;
    };
    let mut problems = Vec::new();
    for (name, csv) in &loaded {
        let table = table(name);
        if csv.len() != table.rows {
            problems.push(format!(
                "{name}: CSV has {} distinct codes, table pins {}",
                csv.len(),
                table.rows
            ));
        }
        let typed = typed_in(table, csv);
        let held = pinned(table.untyped_held);
        let unheld = pinned(table.untyped_unheld);
        let na = na_codes(table.not_applicable);

        for (left_name, left) in [
            ("typed", &typed),
            ("untyped_held", &held),
            ("untyped_unheld", &unheld),
            ("not_applicable", &na),
        ] {
            for other in left.difference(csv) {
                problems.push(format!(
                    "{name}: {left_name} code {other:?} is not in the CSV"
                ));
            }
        }
        let groups = [
            ("typed", &typed),
            ("untyped_held", &held),
            ("untyped_unheld", &unheld),
            ("not_applicable", &na),
        ];
        for (index, (left_name, left)) in groups.iter().enumerate() {
            for (right_name, right) in groups.iter().skip(index + 1) {
                for both in left.intersection(right) {
                    problems.push(format!(
                        "{name}: {both:?} is in both {left_name} and {right_name}"
                    ));
                }
            }
        }
        let union: BTreeSet<&String> = groups.iter().flat_map(|(_, set)| set.iter()).collect();
        for missing in csv.iter().filter(|c| !union.contains(c)) {
            problems.push(format!("{name}: {missing:?} is in no list"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn every_typed_code_is_registered_or_explained() {
    let Some(loaded) = registry_tables() else {
        eprintln!("SKIP isobmff MP4RA coverage: private checkout absent");
        return;
    };
    let all: BTreeSet<String> = loaded.into_iter().flat_map(|(_, codes)| codes).collect();
    let mut unexplained = Vec::new();
    let mut explained = BTreeSet::new();
    for kind in [
        TypedKind::Box,
        TypedKind::QtAtom,
        TypedKind::UdtaAtom,
        TypedKind::SampleEntry,
        TypedKind::SampleEntryBox,
        TypedKind::ItemProperty,
        TypedKind::ItemType,
        TypedKind::Reference,
        TypedKind::EntityGroup,
        TypedKind::SampleGroup,
        TypedKind::TrackGroup,
        TypedKind::TrackReference,
        TypedKind::TrackSelection,
        TypedKind::KeyNamespace,
        TypedKind::Scheme,
        TypedKind::AuxInfoType,
        TypedKind::ColourType,
        TypedKind::DataReference,
        TypedKind::Brand,
        TypedKind::Handler,
    ] {
        for row in kind.rows() {
            if all.contains(row.code) {
                continue;
            }
            let key = (format!("{kind:?}"), row.code.to_string());
            if TYPED_NOT_REGISTERED
                .iter()
                .any(|r| format!("{:?}", r.kind) == key.0 && r.code == row.code)
            {
                explained.insert(key);
            } else {
                unexplained.push(format!("{:?} {}", kind, row.code));
            }
        }
    }
    assert!(
        unexplained.is_empty(),
        "typed codes no MP4RA table lists and no reason explains: {unexplained:?}"
    );
    for entry in TYPED_NOT_REGISTERED {
        let key = (format!("{:?}", entry.kind), entry.code.to_string());
        assert!(
            explained.contains(&key),
            "stale TYPED_NOT_REGISTERED entry {:?} {}",
            entry.kind,
            entry.code
        );
        assert!(!entry.reason.is_empty(), "{} has no reason", entry.code);
    }
}

// ---------------------------------------------------------------------------
// Proofs
// ---------------------------------------------------------------------------

fn check_code(what: &str, code: &str, expected: &str) -> std::result::Result<(), String> {
    if code.trim_end() == expected {
        Ok(())
    } else {
        Err(format!("{what}: {code:?} != {expected:?}"))
    }
}

fn prove_enum(kind: TypedKind, code: &str) -> std::result::Result<(), String> {
    use isobmff::image_brands::ImageBrand;
    use isobmff::item::{EntityGroupType, ItemType, ReferenceType};
    use isobmff::registered::{
        Brand, HandlerType, KeyNamespace, RestrictedSchemeType, SampleGroupType, TrackGroupType,
        TrackReferenceType, TrackSelectionType,
    };
    use isobmff::visual_ext::ColourType;
    let bytes = fourcc(code);
    match kind {
        TypedKind::Brand => {
            let image = ImageBrand::from_fourcc(bytes);
            if !matches!(image, ImageBrand::Other(_)) {
                check_code(
                    "ImageBrand::fourcc",
                    &String::from_utf8_lossy(&image.fourcc()),
                    code,
                )?;
                return check_code("ImageBrand::name", image.name(), code);
            }
            let brand = Brand::from_fourcc(bytes);
            if matches!(brand, Brand::Other(_)) {
                return Err(format!("Brand::from_fourcc({code:?}) is Other"));
            }
            check_code(
                "Brand::fourcc",
                &String::from_utf8_lossy(&brand.fourcc()),
                code,
            )?;
            check_code("Brand::name", brand.name(), code)
        }
        TypedKind::Handler => {
            let value = HandlerType::from_fourcc(bytes);
            if matches!(value, HandlerType::Other(_)) {
                return Err(format!("HandlerType::from_fourcc({code:?}) is Other"));
            }
            check_code(
                "HandlerType::fourcc",
                &String::from_utf8_lossy(&value.fourcc()),
                code,
            )?;
            check_code("HandlerType::name", value.name(), code)
        }
        TypedKind::SampleGroup => {
            let value = SampleGroupType::from_fourcc(bytes);
            if matches!(value, SampleGroupType::Other(_)) {
                return Err(format!("SampleGroupType::from_fourcc({code:?}) is Other"));
            }
            check_code(
                "SampleGroupType::fourcc",
                &String::from_utf8_lossy(&value.fourcc()),
                code,
            )?;
            check_code("SampleGroupType::name", value.name(), code)
        }
        TypedKind::TrackGroup => {
            let value = TrackGroupType::from_fourcc(bytes);
            if matches!(value, TrackGroupType::Other(_)) {
                return Err(format!("TrackGroupType::from_fourcc({code:?}) is Other"));
            }
            check_code(
                "TrackGroupType::fourcc",
                &String::from_utf8_lossy(&value.fourcc()),
                code,
            )?;
            check_code("TrackGroupType::name", value.name(), code)
        }
        TypedKind::TrackReference => {
            let value = TrackReferenceType::from_fourcc(bytes);
            if matches!(value, TrackReferenceType::Other(_)) {
                return Err(format!(
                    "TrackReferenceType::from_fourcc({code:?}) is Other"
                ));
            }
            check_code(
                "TrackReferenceType::fourcc",
                &String::from_utf8_lossy(&value.fourcc()),
                code,
            )?;
            check_code("TrackReferenceType::name", value.name(), code)
        }
        TypedKind::TrackSelection => {
            let value = TrackSelectionType::from_fourcc(bytes);
            if matches!(value, TrackSelectionType::Other(_)) {
                return Err(format!(
                    "TrackSelectionType::from_fourcc({code:?}) is Other"
                ));
            }
            check_code(
                "TrackSelectionType::fourcc",
                &String::from_utf8_lossy(&value.fourcc()),
                code,
            )?;
            check_code("TrackSelectionType::name", value.name(), code)
        }
        TypedKind::KeyNamespace => {
            let value = KeyNamespace::from_fourcc(bytes);
            if matches!(value, KeyNamespace::Other(_)) {
                return Err(format!("KeyNamespace::from_fourcc({code:?}) is Other"));
            }
            check_code(
                "KeyNamespace::fourcc",
                &String::from_utf8_lossy(&value.fourcc()),
                code,
            )?;
            check_code("KeyNamespace::name", value.name(), code)
        }
        TypedKind::ItemType => {
            let value = ItemType::from_fourcc(bytes);
            if matches!(value, ItemType::Other(_)) {
                return Err(format!("ItemType::from_fourcc({code:?}) is Other"));
            }
            check_code(
                "ItemType::fourcc",
                &String::from_utf8_lossy(&value.fourcc()),
                code,
            )
        }
        TypedKind::Reference => {
            let value = ReferenceType::from_fourcc(bytes);
            if matches!(value, ReferenceType::Other(_)) {
                return Err(format!("ReferenceType::from_fourcc({code:?}) is Other"));
            }
            check_code(
                "ReferenceType::fourcc",
                &String::from_utf8_lossy(&value.fourcc()),
                code,
            )?;
            check_code("ReferenceType::name", value.name(), code)
        }
        TypedKind::EntityGroup => {
            let value = EntityGroupType::from_fourcc(bytes);
            if matches!(value, EntityGroupType::Other(_)) {
                return Err(format!("EntityGroupType::from_fourcc({code:?}) is Other"));
            }
            check_code(
                "EntityGroupType::fourcc",
                &String::from_utf8_lossy(&value.fourcc()),
                code,
            )?;
            check_code("EntityGroupType::name", value.name(), code)
        }
        TypedKind::ColourType => {
            let value = ColourType::from_fourcc(bytes);
            if matches!(value, ColourType::Unknown(_)) {
                return Err(format!("ColourType::from_fourcc({code:?}) is Unknown"));
            }
            check_code(
                "ColourType::fourcc",
                &String::from_utf8_lossy(&value.fourcc()),
                code,
            )?;
            check_code("ColourType::name", value.name(), code)
        }
        TypedKind::Scheme => {
            if let Some(scheme) = isobmff::cenc::CencScheme::from_four_cc(&bytes) {
                return check_code("CencScheme::name", scheme.name(), code);
            }
            let value = RestrictedSchemeType::from_fourcc(bytes);
            if matches!(value, RestrictedSchemeType::Other(_)) {
                return Err(format!("no scheme enum names {code:?}"));
            }
            check_code(
                "RestrictedSchemeType::fourcc",
                &String::from_utf8_lossy(&value.fourcc()),
                code,
            )?;
            check_code("RestrictedSchemeType::name", value.name(), code)
        }
        TypedKind::AuxInfoType => {
            let Some(scheme) = isobmff::cenc::CencScheme::from_four_cc(&bytes) else {
                return Err(format!("no CencScheme names {code:?}"));
            };
            check_code("CencScheme::name", scheme.name(), code)
        }
        TypedKind::DataReference => prove_data_reference(code),
        other => Err(format!("{other:?} is not an enumerated identifier kind")),
    }
}

fn prove_data_reference(code: &str) -> std::result::Result<(), String> {
    let entry = match code {
        "url" => boxed("url ", &full(0, 1, &[])),
        "urn" => boxed("urn ", &full(0, 0, b"name\0location\0")),
        "imdt" => boxed("imdt", &full(0, 0, &[0, 0, 0, 0])),
        "snim" => boxed("snim", &full(0, 0, &[])),
        other => return Err(format!("no dref entry builder for {other:?}")),
    };
    let mut body = full(0, 0, &[]);
    body.extend_from_slice(&1u32.to_be_bytes());
    body.extend_from_slice(&entry);
    let parsed = isobmff::init_segment::DataReferenceBox::parse(&boxed("dref", &body))
        .map_err(|e| format!("dref {code:?}: {e:?}"))?;
    match (code, parsed.entries.first()) {
        ("url", Some(isobmff::init_segment::DataEntry::Url(_))) => Ok(()),
        ("urn", Some(isobmff::init_segment::DataEntry::Urn(_))) => Ok(()),
        ("imdt", Some(isobmff::init_segment::DataEntry::Imda(_))) => Ok(()),
        ("snim", Some(isobmff::init_segment::DataEntry::SeqNumImda(_))) => Ok(()),
        (_, Some(isobmff::init_segment::DataEntry::Other(_))) => {
            Err(format!("dref entry {code:?} parsed opaque"))
        }
        (_, other) => Err(format!("dref {code:?} parsed as {other:?}")),
    }
}

fn prove_box(code: &str) -> std::result::Result<(), String> {
    if code == "tran" {
        let bytes = boxed("tran", &[0]);
        let parsed = isobmff::avc_config::TranscodingInfoBox::parse(&bytes)
            .map_err(|e| format!("tran: {e:?}"))?;
        let again = parsed.try_to_bytes().map_err(|e| format!("tran: {e:?}"))?;
        if again != bytes {
            return Err("tran round trip differs".to_string());
        }
        return Ok(());
    }
    if let Some(row) = TYPED_BOXES.iter().find(|r| r.name.trim_end() == code) {
        return (row.proves)().map_err(|e| format!("{}: {e:?}", row.name));
    }
    let bytes = match code {
        "ftyp" | "styp" => boxed(code, b"isom\0\0\0\0"),
        "mvex" => boxed("mvex", &[]),
        "emsg" => {
            let mut body = full(0, 0, &[0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
            body.extend_from_slice(&[0, 0, 0, 0]);
            body.extend_from_slice(b"urn:x\0value\0");
            boxed("emsg", &body)
        }
        "senc" => boxed("senc", &full(0, 0, &[0, 0, 0, 0])),
        "pssh" => {
            let mut body = full(0, 0, &[0u8; 16]);
            body.extend_from_slice(&[0, 0, 0, 0]);
            boxed("pssh", &body)
        }
        "tenc" => {
            let entry = vec![0u8; 20];
            boxed("tenc", &full(0, 0, &entry))
        }
        "sthd" => boxed("sthd", &full(0, 0, &[])),
        "ccst" => {
            let value = isobmff::image_sequence::CodingConstraintsBox::new(false, false, 0);
            let mut out = vec![0u8; value.serialized_len()];
            value
                .serialize_into(&mut out)
                .map_err(|e| format!("ccst: {e:?}"))?;
            out
        }
        "md5i" => {
            let value = isobmff::integrity::Md5IntegrityBox::new([0u8; 16], *b"mdat");
            let mut out = vec![0u8; value.serialized_len()];
            value
                .serialize_into(&mut out)
                .map_err(|e| format!("md5i: {e:?}"))?;
            out
        }
        "url" => boxed("url ", &full(0, 1, &[])),
        "wide" => boxed("wide", &[]),
        "tran" => boxed("tran", &[0]),
        "!mov" | "!mof" | "!six" | "!ssx" => boxed(code, &[1, 2, 3]),
        "ttyp" => boxed("ttyp", b"isom\0\0\0\0"),
        "tfad" => boxed("tfad", &[]),
        "tfma" => boxed("tfma", &full(0, 0, &[0, 0, 0, 0])),
        "spdb" => boxed("spdb", &full(0, 0, b"<spd/>")),
        "3gau" => boxed(
            "3gau",
            &[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ),
        other => return Err(format!("no box proof for {other:?}")),
    };
    match code {
        "ftyp" => isobmff::segments::FileTypeBox::parse_box(&bytes).map(|_| ()),
        "styp" => isobmff::segments::SegmentTypeBox::parse_box(&bytes).map(|_| ()),
        "mvex" => isobmff::init_segment::MovieExtendsBox::parse(&bytes).map(|_| ()),
        "emsg" => isobmff::emsg::EmsgBox::parse(&bytes).map(|_| ()),
        "senc" => isobmff::cenc::SampleEncryptionBox::parse_box(&bytes, 0).map(|_| ()),
        "pssh" => isobmff::cenc::ProtectionSystemSpecificHeaderBox::parse_box(&bytes).map(|_| ()),
        "tenc" => isobmff::cenc::TrackEncryptionBox::parse_box(&bytes).map(|_| ()),
        "sthd" => isobmff::init_segment::SubtitleMediaHeaderBox::parse(&bytes).map(|_| ()),
        "ccst" => isobmff::image_sequence::CodingConstraintsBox::parse(&bytes).map(|_| ()),
        "md5i" => isobmff::integrity::Md5IntegrityBox::parse(&bytes).map(|_| ()),
        "url" => isobmff::init_segment::DataEntryUrlBox::parse(&bytes).map(|_| ()),
        "wide" => isobmff::segments::WideAtom::parse(&bytes).map(|_| ()),
        "!mov" | "!mof" | "!six" | "!ssx" => {
            isobmff::compressed::CompressedBox::parse_box(&bytes).map(|_| ())
        }
        "ttyp" => isobmff::segments::TrackTypeBox::parse_box(&bytes).map(|_| ()),
        "tfad" => isobmff::threegpp::TrackFragmentAdjustmentBox::parse_box(&bytes).map(|_| ()),
        "tfma" => isobmff::threegpp::TrackFragmentMediaAdjustmentBox::parse_box(&bytes).map(|_| ()),
        "spdb" => isobmff::threegpp::ServiceProtectionDescriptionBox::parse_box(&bytes).map(|_| ()),
        "3gau" => isobmff::threegpp::ApplicationDataUnitInfoBox::parse_box(&bytes).map(|_| ()),
        other => return Err(format!("no box proof for {other:?}")),
    }
    .map(|_| ())
    .map_err(|e| format!("{code}: {e:?}"))
}

const VISUAL_ENTRIES: &[&str] = &[
    "avc1", "avc2", "avc3", "avc4", "avcp", "hvc1", "hev1", "dvh1", "dvhe", "vvc1", "vvi1", "mp4v",
    "av01", "vp09", "rgan", "avs3", "lav3", "PNTG", "SVQ1", "SVQ3", "WRLE", "ap4h", "ap4x", "apch",
    "apcn", "apco", "apcs", "aprh", "aprn", "civd", "dvh5", "dvh6", "dvhp", "dvi", "flic", "gif",
    "h261", "h263", "jpeg", "mjpa", "mjpb", "png", "rle", "rpza", "tga", "tiff", "s263",
];
const AUDIO_ENTRIES: &[&str] = &[
    "mp4a", "enca", "ac-3", "ec-3", "ac-4", "Opus", "fLaC", "mha1", "mha2", "mhm1", "mhm2", "dtsc",
    "dtsh", "dtsl", "dtse", "lpcm", "in24", "in32", "sowt", "twos", "raw", "fl32", "fl64", "ima4",
    "ulaw", "alaw", "NONE", "QDM2", "QDMC", "Qclp", "agsm", "vdva", "samr", "sawb", "sawp", "sevs",
];
const HINT_ENTRIES: &[&str] = &[
    "rtp", "srtp", "rrtp", "rsrp", "rtcp", "stcp", "prtp", "rm2t", "sm2t", "pm2t", "fdp",
];

fn sample_entry_bytes(code: &str) -> Result<Vec<u8>, String> {
    let fixed = if code == "rgan" {
        8
    } else if VISUAL_ENTRIES.contains(&code) {
        78
    } else if AUDIO_ENTRIES.contains(&code) {
        28
    } else if HINT_ENTRIES.contains(&code) {
        64
    } else {
        8
    };
    let extra: &[u8] = match code {
        "stpp" => b"\0\0\0",
        "wvtt" => {
            return Ok(boxed(
                code,
                &[&[0u8; 8][..], &boxed("vttC", b"\0")].concat(),
            ));
        }
        "metx" => b"enc\0ns\0schema\0",
        "mett" | "stxt" => b"enc\0mime\0",
        "urim" => {
            return Ok(boxed(
                code,
                &[
                    &[0u8; 8][..],
                    &bytes_of(&isobmff::meta_track::UriBox::new("urn:x")),
                ]
                .concat(),
            ));
        }
        "tmcd" => &[0u8; 25],
        "3gvo" => &[0u8; 1],
        "c608" => &[],
        "mebx" => {
            return Ok(boxed(
                code,
                &[&[0u8; 8][..], &boxed("keys", &full(0, 0, &[0, 0, 0, 0]))].concat(),
            ));
        }
        _ => &[],
    };
    let mut body = vec![0u8; fixed];
    if matches!(code, "stpp" | "wvtt" | "metx" | "mett" | "stxt" | "urim") {
        body = vec![0u8; 8];
    }
    body.extend_from_slice(extra);
    if VISUAL_ENTRIES.contains(&code) {
        let config = match code {
            "avc1" | "avc2" | "avc3" | "avc4" | "avcp" => {
                boxed("avcC", &[1, 66, 0, 30, 0xFF, 0xE0, 0])
            }
            "hvc1" | "hev1" | "dvh1" | "dvhe" => boxed("hvcC", &[0u8; 23]),
            "vvc1" | "vvi1" => boxed("vvcC", &[0u8; 32]),
            "av01" => boxed("av1C", &[0x80, 0, 0, 0]),
            "vp09" => boxed("vpcC", &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            "rgan" => bytes_of(&isobmff::region::RegionTrackConfigBox::new(1, 1)),
            _ => Vec::new(),
        };
        body.extend_from_slice(&config);
    }
    if matches!(code, "avs3" | "lav3") {
        let cfg = if code == "avs3" { "av3c" } else { "lavc" };
        let payload: &[u8] = if code == "avs3" {
            &[1, 0, 0, 0]
        } else {
            &[1, 0]
        };
        body.extend_from_slice(&boxed(cfg, payload));
    }
    Ok(boxed(code, &body))
}

fn prove_sample_entry(code: &str) -> std::result::Result<(), String> {
    if code == "encv" {
        let sinf = boxed(
            "sinf",
            &[
                &boxed("frma", b"avc1")[..],
                &boxed("schm", &full(0, 0, &[0u8; 12]))[..],
            ]
            .concat(),
        );
        let entry = boxed("encv", &[&[0u8; 78][..], &sinf[..]].concat());
        let found = isobmff::cenc::sinf_of_sample_entry(&entry)
            .ok_or_else(|| "encv sinf not found".to_string())?;
        return isobmff::cenc::ProtectionSchemeInfoBox::parse(found)
            .map(|_| ())
            .map_err(|e| format!("encv: {e:?}"));
    }
    let bytes = sample_entry_bytes(code)?;
    if matches!(code, "avs3" | "lav3") {
        return isobmff::avs3::AvsVideoSampleEntry::parse_entry(&bytes)
            .map(|_| ())
            .map_err(|e| format!("{code}: {e:?}"));
    }
    let variant = isobmff::init_segment::sample_entry_variant(&bytes)
        .map_err(|e| format!("{code}: {e:?}"))?;
    if matches!(
        variant,
        isobmff::init_segment::SampleEntryVariant::Unknown(_)
    ) {
        return Err(format!("{code:?} parsed opaque"));
    }
    Ok(())
}

fn bytes_of<T>(value: &T) -> Vec<u8>
where
    T: broadcast_core::Serialize<Error = isobmff::error::Error>,
{
    let mut out = vec![0u8; value.serialized_len()];
    value
        .serialize_into(&mut out)
        .expect("serialized length matches");
    out
}

fn property_bytes(code: &str) -> Option<Vec<u8>> {
    use isobmff::image::{
        Av1LayeredImageIndexingProperty, ImageMirror, ImageMirrorAxis, ImageRotation,
        ImageSpatialExtentsProperty, LayerSelectorProperty, OperatingPointSelectorProperty,
        PixelInformationProperty, RelativeLocationProperty,
    };
    use isobmff::image_props as props;
    let bytes = match code {
        "ispe" => bytes_of(&ImageSpatialExtentsProperty::new(1, 1)),
        "pixi" => bytes_of(&PixelInformationProperty::new(vec![8])),
        "rloc" => bytes_of(&RelativeLocationProperty::new(0, 0)),
        "auxC" => bytes_of(&isobmff::image::AuxiliaryTypeProperty::new(b"urn:x")),
        "irot" => bytes_of(&ImageRotation::new(0)),
        "lsel" => bytes_of(&LayerSelectorProperty::new(0)),
        "imir" => bytes_of(&ImageMirror::new(ImageMirrorAxis::Vertical)),
        "a1op" => bytes_of(&OperatingPointSelectorProperty::new(0)),
        "a1lx" => bytes_of(&Av1LayeredImageIndexingProperty::new([0; 3])),
        "iscl" => bytes_of(&props::ImageScaling::new(1, 1, 1, 1)),
        "rref" => bytes_of(&props::RequiredReferenceTypesProperty::new(vec![])),
        "crtt" => bytes_of(&props::CreationTimeProperty::new(0)),
        "mdft" => bytes_of(&props::ModificationTimeProperty::new(0)),
        "udes" => bytes_of(&props::UserDescriptionProperty::new(
            b"en", b"n", b"d", b"t",
        )),
        "altt" => bytes_of(&props::AccessibilityTextProperty::new(b"x", b"en")),
        "aebr" => bytes_of(&props::AutoExposureProperty::new(0, 0)),
        "wbbr" => bytes_of(&props::WhiteBalanceProperty::new(0, 0)),
        "fobr" => bytes_of(&props::FocusProperty::new(0, 0)),
        "afbr" => bytes_of(&props::FlashExposureProperty::new(0, 0)),
        "dobr" => bytes_of(&props::DepthOfFieldProperty::new(0, 0)),
        "pano" => bytes_of(&props::PanoramaProperty::new(Default::default())),
        "tols" => bytes_of(&props::TargetOutputLayerSetProperty::new(0)),
        "wipe" => bytes_of(&props::WipeTransitionEffectProperty::new(Default::default())),
        "zoom" => bytes_of(&props::ZoomTransitionEffectProperty::new(
            Default::default(),
            Default::default(),
        )),
        "fade" => bytes_of(&props::FadeTransitionEffectProperty::new(Default::default())),
        "splt" => bytes_of(&props::SplitTransitionEffectProperty::new(
            Default::default(),
        )),
        "stpe" => bytes_of(&props::SuggestedTransitionPeriodProperty::new(0)),
        "ssld" => bytes_of(&props::SuggestedTimeDisplayDurationProperty::new(0)),
        "prdi" => bytes_of(&props::ProgressiveDerivedImageItemInformationProperty::new(
            Some(0),
            vec![],
        )),
        "sstr" => bytes_of(&props::SingleStreamProperty::new()),
        "cmex" => bytes_of(&props::CameraExtrinsicMatrixProperty::new(
            None, None, None, None, None,
        )),
        "cmin" => bytes_of(&props::CameraIntrinsicMatrixProperty::simplified(1, 0, 0)),
        "cexg" => bytes_of(&props::ConstrainedExtentsGridProperty::new(0, 0, 1, 1)),
        "dadj" => bytes_of(&props::DisparityAdjustmentProperty::new(0)),
        "stag" => bytes_of(&props::StereoAggressorsProperty::new(vec![
            props::StereoAggressor::new(Default::default(), 0),
        ])),
        "reve" => bytes_of(&props::ReferenceViewingEnvironmentProperty::new(
            0, 0, 0, 0, 0, 0,
        )),
        "ndwt" => bytes_of(&props::NominalDiffuseWhiteProperty::new(1)),
        "mskC" => bytes_of(&props::MaskConfigurationProperty::new(8)),
        "pasp" => boxed("pasp", &[0u8; 8]),
        "clap" => boxed("clap", &[0u8; 32]),
        "colr" => {
            let mut body = b"nclx".to_vec();
            body.extend_from_slice(&[0u8; 7]);
            boxed("colr", &body)
        }
        "mdcv" => boxed("mdcv", &[0u8; 24]),
        "clli" => boxed("clli", &[0u8; 4]),
        "amve" => bytes_of(&isobmff::visual_ext::AmbientViewingEnvironmentBox::new(
            0, 0, 0,
        )),
        "cclv" => bytes_of(&isobmff::visual_ext::ContentColourVolumeBox::default()),
        "brnd" => bytes_of(&isobmff::item::BrandProperty::new(
            isobmff::segments::FileTypeBox {
                major_brand: *b"isom",
                minor_version: 0,
                compatible_brands: vec![],
            },
        )),
        "hdlp" => bytes_of(&isobmff::item::HandlerProperty::new(*b"vide")),
        "scrb" => bytes_of(&isobmff::scheme::ScrambleSchemeInfoBox::new(
            isobmff::cenc::SchemeTypeBox {
                version: 0,
                flags: 0,
                scheme_type: *b"cenc",
                scheme_version: 0,
                scheme_uri: None,
            },
        )),
        "stvi" => bytes_of(&isobmff::track_desc::StereoVideoBox::new(0, vec![])),
        "avcC" => boxed("avcC", &[1, 66, 0, 30, 0xFF, 0xE0, 0]),
        "hvcC" => boxed("hvcC", &[0u8; 23]),
        "vvcC" => boxed("vvcC", &[0u8; 32]),
        "av1C" => boxed("av1C", &[0x80, 0, 0, 0]),
        "subs" => boxed("subs", &[0u8; 8]),
        _ => return None,
    };
    Some(bytes)
}

fn prove_item_property(code: &str) -> std::result::Result<(), String> {
    let bytes = property_bytes(code).ok_or_else(|| format!("no property builder for {code:?}"))?;
    let parsed = match isobmff::image::ItemProperty::parse_box(&bytes) {
        Ok(value) if !matches!(value, isobmff::image::ItemProperty::Opaque(_)) => value,
        _ => {
            let wrapped = boxed(code, &bytes);
            isobmff::image::ItemProperty::parse_box(&wrapped)
                .map_err(|e| format!("{code}: {e:?}"))?
        }
    };
    if matches!(parsed, isobmff::image::ItemProperty::Opaque(_)) {
        return Err(format!("{code:?} parsed opaque"));
    }
    Ok(())
}

fn config_body(bytes: &[u8]) -> &[u8] {
    bytes.get(8..).unwrap_or_default()
}

fn config_bytes(code: &str) -> std::result::Result<Vec<u8>, String> {
    let bytes = match code {
        "av1C" => boxed("av1C", &[0x80, 0, 0, 0]),
        "vpcC" => {
            let mut body = vec![1, 0, 0, 0];
            body.extend_from_slice(&[0u8; 8]);
            boxed("vpcC", &body)
        }
        "av2c" => boxed("av2c", &[1, 0, 0]),
        "la2c" => boxed("la2c", &[1, 0, 0]),
        "av3c" => boxed("av3c", &[1, 0, 0, 0]),
        "lavc" => boxed("lavc", &[1, 0]),
        "ccst" => bytes_of(&isobmff::image_sequence::CodingConstraintsBox::new(
            false, false, 0,
        )),
        "chnl" => bytes_of(&isobmff::audio_boxes::ChannelLayoutBox::new(0)),
        "dmix" => bytes_of(&isobmff::audio_boxes::DownMixInstructionsBox::new(
            isobmff::audio_boxes::DownMixInstruction::new(0, 0, false, 0),
        )),
        "dfLa" => boxed("dfLa", &full(0, 0, &[0x80, 0, 0, 0])),
        "dca3" => boxed("dca3", &[0xC0]),
        "mhaC" => boxed(
            "mhaC",
            &bytes_of(&isobmff::mpegh::MHADecoderConfigurationRecord::new(
                0,
                0,
                vec![],
            )),
        ),
        "esds" => bytes_of(&isobmff::mp4esds::EsdsBox::new(
            isobmff::mp4esds::ESDescriptor::new(1, 0, None, None),
        )),
        "scrb" => bytes_of(&isobmff::scheme::ScrambleSchemeInfoBox::new(
            isobmff::cenc::SchemeTypeBox {
                version: 0,
                flags: 0,
                scheme_type: *b"cenc",
                scheme_version: 0,
                scheme_uri: None,
            },
        )),
        "dvcC" | "dvvC" => boxed(code, &[0u8; 24]),
        "clli" => boxed("clli", &[0u8; 4]),
        "mdcv" => boxed("mdcv", &[0u8; 24]),
        "damr" => boxed("damr", &[0u8; 9]),
        "d263" => boxed("d263", &[0u8; 7]),
        "dawp" => boxed("dawp", &[0u8; 5]),
        _ => boxed(code, &[0u8; 256]),
    };
    Ok(bytes)
}

fn prove_sample_entry_box(code: &str) -> std::result::Result<(), String> {
    let bytes = config_bytes(code)?;
    if let Ok(parsed) = isobmff::image::ItemProperty::parse_box(&bytes)
        && !matches!(parsed, isobmff::image::ItemProperty::Opaque(_))
    {
        return Ok(());
    }
    match code {
        "av1C" => isobmff::av1::Av1ConfigurationBox::parse(&bytes).map(|_| ()),
        "vpcC" => isobmff::vp9::Vp9ConfigurationBox::parse(config_body(&bytes)).map(|_| ()),
        "av2c" | "la2c" => {
            isobmff::avs2::Avs2ConfigurationBox::parse(config_body(&bytes)).map(|_| ())
        }
        "av3c" => isobmff::avs3::Avs3ConfigurationBox::parse(config_body(&bytes)).map(|_| ()),
        "lavc" => isobmff::avs3::LAvsConfigurationBox::parse(config_body(&bytes)).map(|_| ()),
        "ccst" => isobmff::image_sequence::CodingConstraintsBox::parse(&bytes).map(|_| ()),
        "chnl" => isobmff::audio_boxes::ChannelLayoutBox::parse(&bytes).map(|_| ()),
        "dmix" => isobmff::audio_boxes::DownMixInstructionsBox::parse(&bytes).map(|_| ()),
        "btrt" => isobmff::sample_entries::BitRateBox::parse(&bytes).map(|_| ()),
        "dOps" => isobmff::opus::OpusSpecificBox::parse(&bytes).map(|_| ()),
        "dac3" => isobmff::ac3::Ac3SpecificBox::parse(&bytes).map(|_| ()),
        "dec3" => isobmff::ac3::Ec3SpecificBox::parse(&bytes).map(|_| ()),
        "dac4" => isobmff::ac4::Ac4SpecificBox::parse(&bytes).map(|_| ()),
        "ddts" => isobmff::dts::DtsSpecificBox::parse(&bytes).map(|_| ()),
        "srat" => isobmff::init_segment::SamplingRateBox::parse(&bytes).map(|_| ()),
        "txtC" => isobmff::meta_track::TextConfigBox::parse(&bytes).map(|_| ()),
        "uriI" => isobmff::meta_track::UriInitBox::parse(&bytes).map(|_| ()),
        "vttC" => isobmff::subtitle_entries::WebVttConfigurationBox::parse(&bytes).map(|_| ()),
        "dfLa" => isobmff::flac::FlacSpecificBox::parse(config_body(&bytes)).map(|_| ()),
        "dca3" => isobmff::avs3_audio::Ca3SpecificBox::parse(config_body(&bytes)).map(|_| ()),
        "mhaC" => {
            isobmff::mpegh::MHADecoderConfigurationRecord::parse(config_body(&bytes)).map(|_| ())
        }
        "esds" => isobmff::mp4esds::EsdsBox::parse_box(&bytes).map(|_| ()),
        "scrb" => isobmff::scheme::ScrambleSchemeInfoBox::parse(&bytes).map(|_| ()),
        "dvcC" | "dvvC" => {
            isobmff::dovi::DoviDecoderConfigurationRecord::parse(config_body(&bytes)).map(|_| ())
        }
        "damr" => {
            isobmff::threegpp::AmrDecoderSpecificBox::parse_body(config_body(&bytes)).map(|_| ())
        }
        "d263" => {
            isobmff::threegpp::H263DecoderSpecificBox::parse_body(config_body(&bytes)).map(|_| ())
        }
        "dawp" => isobmff::threegpp::AmrwbPlusDecoderSpecificBox::parse_body(config_body(&bytes))
            .map(|_| ()),
        "clli" => isobmff::hdr_boxes::ContentLightLevel::parse(config_body(&bytes)).map(|_| ()),
        "mdcv" => {
            isobmff::hdr_boxes::MasteringDisplayColourVolume::parse(config_body(&bytes)).map(|_| ())
        }
        _ => Err(isobmff::error::Error::InvalidInput(
            "no sample-entry-box proof",
        )),
    }
    .map_err(|e| format!("{code:?}: {e:?}"))
}

fn prove_atom(code: &str) -> std::result::Result<(), String> {
    if code == "load" {
        return isobmff::quicktime::PreloadBox::parse_box(&boxed("load", &[0u8; 12]))
            .map(|_| ())
            .map_err(|e| format!("load: {e:?}"));
    }
    if code == "elng" {
        return isobmff::track_desc::ExtendedLanguageBox::parse(&boxed(
            "elng",
            &full(0, 0, b"en\0"),
        ))
        .map(|_| ())
        .map_err(|e| format!("elng: {e:?}"));
    }
    if code == "tlou" {
        let value = isobmff::audio_boxes::TrackLoudnessInfo::from_base(
            isobmff::audio_boxes::LoudnessBaseBox::new(isobmff::audio_boxes::LoudnessKind::Track)
                .with_entries(vec![
                    isobmff::audio_boxes::LoudnessEntry::new(0, 0, 0, 0, 0, 0).with_measurements(
                        vec![isobmff::audio_boxes::LoudnessMeasurement::new(3, 9, 4, 5)],
                    ),
                ]),
        );
        return isobmff::audio_boxes::TrackLoudnessInfo::parse(&bytes_of(&value))
            .map(|_| ())
            .map_err(|e| format!("tlou: {e:?}"));
    }
    if code == "alou" {
        let value = isobmff::audio_boxes::AlbumLoudnessInfo::from_base(
            isobmff::audio_boxes::LoudnessBaseBox::new(isobmff::audio_boxes::LoudnessKind::Album)
                .with_entries(vec![
                    isobmff::audio_boxes::LoudnessEntry::new(0, 0, 0, 0, 0, 0).with_measurements(
                        vec![isobmff::audio_boxes::LoudnessMeasurement::new(3, 9, 4, 5)],
                    ),
                ]),
        );
        return isobmff::audio_boxes::AlbumLoudnessInfo::parse(&bytes_of(&value))
            .map(|_| ())
            .map_err(|e| format!("alou: {e:?}"));
    }
    if code == "ludt" {
        let track = isobmff::audio_boxes::TrackLoudnessInfo::from_base(
            isobmff::audio_boxes::LoudnessBaseBox::new(isobmff::audio_boxes::LoudnessKind::Track)
                .with_entries(vec![
                    isobmff::audio_boxes::LoudnessEntry::new(0, 0, 0, 0, 0, 0).with_measurements(
                        vec![isobmff::audio_boxes::LoudnessMeasurement::new(3, 9, 4, 5)],
                    ),
                ]),
        );
        let value = isobmff::audio_boxes::LoudnessBox::new().with_track(track);
        return isobmff::audio_boxes::LoudnessBox::parse(&bytes_of(&value))
            .map(|_| ())
            .map_err(|e| format!("ludt: {e:?}"));
    }
    let child = match code {
        "loci" => boxed(
            "loci",
            &full(
                0,
                0,
                &[0, 0, b'n', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            ),
        ),
        "strk" => boxed("strk", &boxed("stri", &full(0, 0, &[0u8; 8]))),
        "albm" | "auth" | "cprt" | "titl" | "dscp" | "perf" | "gnre" | "yrrc" | "name" | "hinf"
        | "hnti" | "tsel" | "kind" => boxed(code, &[0u8; 64]),
        "clsf" => bytes_of(&isobmff::threegpp::MediaClassificationBox::new(
            *b"BBFC",
            1,
            b"PG".to_vec(),
        )),
        "coll" => bytes_of(&isobmff::threegpp::CollectionNameBox::new(b"Set".to_vec())),
        "kywd" => bytes_of(&isobmff::threegpp::KeywordsBox::new(vec![b"one".to_vec()])),
        "rtng" => bytes_of(&isobmff::threegpp::MediaRatingBox::new(
            *b"BBFC",
            *b"PG13",
            b"12A".to_vec(),
        )),
        "urat" => bytes_of(&isobmff::threegpp::UserRatingBox::new(50)),
        "thmb" => bytes_of(&isobmff::threegpp::ThumbnailBox::new(
            *b"jpeg",
            vec![0xFF, 0xD8],
        )),
        "orie" => bytes_of(&isobmff::threegpp::OrientationInformationBox::new(
            1, 1, true, 0, 0, 0,
        )),
        other => return Err(format!("no atom proof for {other:?}")),
    };
    let parsed = isobmff::metadata::Udta::parse(&boxed("udta", &child))
        .map_err(|e| format!("{code}: {e:?}"))?;
    match parsed.children.first() {
        Some(isobmff::metadata::UdtaChild::Opaque(_)) => Err(format!("{code:?} parsed opaque")),
        Some(_) => Ok(()),
        None => Err(format!("{code:?} produced no child")),
    }
}

fn prove_row(kind: TypedKind, code: &str) -> std::result::Result<(), String> {
    match kind {
        TypedKind::Box => prove_box(code),
        TypedKind::SampleEntry => prove_sample_entry(code),
        TypedKind::SampleEntryBox => prove_sample_entry_box(code),
        TypedKind::ItemProperty => prove_item_property(code),
        TypedKind::QtAtom | TypedKind::UdtaAtom => prove_atom(code),
        TypedKind::ItemType
        | TypedKind::Reference
        | TypedKind::EntityGroup
        | TypedKind::SampleGroup
        | TypedKind::TrackGroup
        | TypedKind::TrackReference
        | TypedKind::TrackSelection
        | TypedKind::KeyNamespace
        | TypedKind::Scheme
        | TypedKind::AuxInfoType
        | TypedKind::ColourType
        | TypedKind::DataReference
        | TypedKind::Brand
        | TypedKind::Handler => prove_enum(kind, code),
    }
}

#[test]
fn every_typed_row_proves_itself() {
    let mut failures = Vec::new();
    for kind in [
        TypedKind::Box,
        TypedKind::QtAtom,
        TypedKind::UdtaAtom,
        TypedKind::SampleEntry,
        TypedKind::SampleEntryBox,
        TypedKind::ItemProperty,
        TypedKind::ItemType,
        TypedKind::Reference,
        TypedKind::EntityGroup,
        TypedKind::SampleGroup,
        TypedKind::TrackGroup,
        TypedKind::TrackReference,
        TypedKind::TrackSelection,
        TypedKind::KeyNamespace,
        TypedKind::Scheme,
        TypedKind::AuxInfoType,
        TypedKind::ColourType,
        TypedKind::DataReference,
        TypedKind::Brand,
        TypedKind::Handler,
    ] {
        for TypedRow { code } in kind.rows() {
            if let Err(problem) = prove_row(kind, code) {
                failures.push(format!("{kind:?} {code:?}: {problem}"));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn readme_states_the_mp4ra_counts() {
    let Some(loaded) = registry_tables() else {
        eprintln!("SKIP isobmff MP4RA coverage: private checkout absent");
        return;
    };
    let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"))
        .expect("README.md is committed");
    let mut totals = [0usize; 5];
    let mut seen = BTreeSet::new();
    for line in readme.lines() {
        let Some(rest) = line.strip_prefix("| `") else {
            continue;
        };
        let Some((name, rest)) = rest.split_once(".csv` | ") else {
            continue;
        };
        if !mp4ra::TABLES.iter().any(|t| t.csv == name) {
            continue;
        }
        seen.insert(name.to_string());
        let cells: Vec<&str> = rest.split('|').map(str::trim).collect();
        let numbers: Vec<usize> = cells
            .iter()
            .take(5)
            .map(|cell| {
                cell.parse()
                    .unwrap_or_else(|_| panic!("{name}: {cell:?} is not a count"))
            })
            .collect();
        let (codes, typed, held, unheld, na) =
            (numbers[0], numbers[1], numbers[2], numbers[3], numbers[4]);
        let table = table(name);
        let Some(csv) = csv_codes(name) else {
            return;
        };
        let typed_codes = typed_in(table, &csv);
        let held_codes = pinned(table.untyped_held);
        let unheld_codes = pinned(table.untyped_unheld);
        let na_codes_set = na_codes(table.not_applicable);
        assert_eq!(table.rows, codes, "{name} code count");
        assert_eq!(typed_codes.len(), typed, "{name} typed count");
        assert_eq!(held_codes.len(), held, "{name} held-untyped count");
        assert_eq!(unheld_codes.len(), unheld, "{name} not-held count");
        assert_eq!(na_codes_set.len(), na, "{name} not-applicable count");
        totals[0] += codes;
        totals[1] += typed;
        totals[2] += held;
        totals[3] += unheld;
        totals[4] += na;
    }
    assert_eq!(seen.len(), mp4ra::TABLES.len(), "README lists every table");
    let total_line = readme
        .lines()
        .find(|line| line.starts_with("| **Total** |"))
        .expect("README states the MP4RA totals");
    let numbers: Vec<usize> = total_line
        .split('|')
        .skip(2)
        .take(5)
        .map(|cell| cell.trim().parse().unwrap_or(0))
        .collect();
    assert_eq!(numbers, totals, "README MP4RA totals");
    assert_eq!(
        totals[1] + totals[2] + totals[3] + totals[4],
        totals[0],
        "README columns partition the code count"
    );
    let _ = loaded;
}
