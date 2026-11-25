//! Pin the element-id table to the private transcription, loaded at runtime.
//!
//! `test_fixtures::spec` returns `None` when the private checkout is absent, so
//! this test skips loudly there and is a hard gate whenever it is present.

use broadcast_matroska::ebml::document::{
    DOC_TYPE_ID, DOC_TYPE_READ_VERSION_ID, DOC_TYPE_VERSION_ID, EBML_ID, EBML_MAX_ID_LENGTH_ID,
    EBML_MAX_SIZE_LENGTH_ID, EBML_READ_VERSION_ID, EBML_VERSION_ID,
};
use broadcast_matroska::matroska::ids::{self, ElementKind};

const TRANSCRIPTION: &str = "mpeg/transmux/webm/ebml-matroska.md";

#[test]
fn element_ids_match_the_transcription() {
    let Some(path) = test_fixtures::spec(TRANSCRIPTION, "broadcast-matroska") else {
        eprintln!(
            "SKIP: private/ is not checked out, so {TRANSCRIPTION} cannot pin the element ids"
        );
        return;
    };
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let mut rows = 0usize;
    for line in text.lines() {
        let Some(row) = parse_row(line) else {
            continue;
        };
        rows += 1;
        let (raw_id, name, kind) = row;
        let declared = match raw_id {
            EBML_ID => Some(("EBML", ElementKind::Master)),
            DOC_TYPE_ID => Some(("DocType", ElementKind::String)),
            EBML_VERSION_ID => Some(("EBMLVersion", ElementKind::Unsigned)),
            EBML_READ_VERSION_ID => Some(("EBMLReadVersion", ElementKind::Unsigned)),
            EBML_MAX_ID_LENGTH_ID => Some(("EBMLMaxIDLength", ElementKind::Unsigned)),
            EBML_MAX_SIZE_LENGTH_ID => Some(("EBMLMaxSizeLength", ElementKind::Unsigned)),
            DOC_TYPE_VERSION_ID => Some(("DocTypeVersion", ElementKind::Unsigned)),
            DOC_TYPE_READ_VERSION_ID => Some(("DocTypeReadVersion", ElementKind::Unsigned)),
            _ => ids::lookup(raw_id).map(|def| (def.name, def.kind)),
        };
        let Some((declared_name, declared_kind)) = declared else {
            panic!("0x{raw_id:X} ({name}) is in the transcription but not typed");
        };
        assert_eq!(
            declared_name, name,
            "0x{raw_id:X}: name differs from the transcription"
        );
        assert_eq!(
            declared_kind, kind,
            "0x{raw_id:X} ({name}): type differs from the transcription"
        );
    }
    assert!(rows >= 27, "parsed only {rows} transcription rows");
}

#[test]
fn element_table_ids_and_names_are_unique() {
    let mut ids: Vec<u32> = ids::ALL.iter().map(|def| def.id).collect();
    ids.sort_unstable();
    let mut unique = ids.clone();
    unique.dedup();
    assert_eq!(ids, unique, "duplicate element id in ids::ALL");

    let mut names: Vec<&str> = ids::ALL.iter().map(|def| def.name).collect();
    names.sort_unstable();
    let mut unique_names = names.clone();
    unique_names.dedup();
    assert_eq!(names, unique_names, "duplicate element name in ids::ALL");
}

/// Parse one `| 0x… | Name | parent | type |` row of the demux table.
fn parse_row(line: &str) -> Option<(u32, &str, ElementKind)> {
    let rest = line.trim().strip_prefix("| `0x")?;
    let (hex, tail) = rest.split_once('`')?;
    let raw_id = u32::from_str_radix(hex, 16).ok()?;
    let mut columns = tail.trim_start().strip_prefix('|')?.split('|');
    let name = columns.next()?.trim();
    let _parent = columns.next()?;
    let payload = columns.next()?.trim();
    let kind = payload_kind(payload)?;
    let name = name.strip_suffix(" (header)").unwrap_or(name);
    Some((raw_id, name, kind))
}

/// Map a transcription Payload cell to an EBML type.
fn payload_kind(payload: &str) -> Option<ElementKind> {
    let first = payload.split_whitespace().next()?;
    match first {
        "master" => Some(ElementKind::Master),
        "uint" => Some(ElementKind::Unsigned),
        "float" => Some(ElementKind::Float),
        "string" => Some(ElementKind::String),
        "binary" => Some(ElementKind::Binary),
        _ => None,
    }
}

#[test]
fn every_master_has_a_typed_variant() {
    use broadcast_matroska::Limits;
    use broadcast_matroska::ebml::{ElementId, VintSize};
    use broadcast_matroska::matroska::Element;

    for def in ids::ALL {
        if def.kind != ElementKind::Master {
            continue;
        }
        let id = ElementId::try_from_raw(def.id).unwrap();
        let mut bytes = vec![0u8; 12];
        let id_len = id.write(&mut bytes).unwrap();
        let size_len = VintSize::minimal(0)
            .unwrap()
            .write(&mut bytes[id_len..])
            .unwrap();
        bytes.truncate(id_len + size_len);
        let (element, used) = Element::parse_at(&bytes, &Limits::new())
            .unwrap_or_else(|error| panic!("{} (0x{:X}): {error}", def.name, def.id));
        assert_eq!(used, bytes.len());
        assert_eq!(element.id(), def.id, "{}", def.name);
    }
}
