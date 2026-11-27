//! The item reader degrades a box it cannot reproduce instead of failing the
//! file: the property stays opaque, byte-exact, and a typed finding names it.
//!
//! Each case takes a committed still-image fixture and breaks one byte of one
//! typed item property, so the property's typed parse fails where it used to
//! fail the whole read.

use isobmff::error::OpaqueReason;
use isobmff::image_brands::ImageFinding;
use isobmff::item_reader::{ItemReader, ItemReaderConfig, ItemStep, StillImage};
use test_fixtures as fixtures;

/// A property this crate types, carrying `pixel_information` (a channel count
/// followed by that many bit depths).
const PIXI: [u8; 4] = *b"pixi";
/// Body offset of `pixi`'s channel count, after its version/flags.
const PIXI_COUNT_OFFSET: usize = 4;
/// Offset of a box four-CC within its header.
const TYPE_OFFSET: usize = 4;
/// Width of one box header.
const HEADER_LEN: usize = 8;

fn load(bytes: &[u8]) -> StillImage {
    let config = ItemReaderConfig::default().with_total_len(Some(bytes.len() as u64));
    let mut reader = ItemReader::new(config);
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
                return image;
            }
            ItemStep::Done => panic!("done without a model"),
            _ => panic!("unexpected step"),
        }
    }
}

/// Byte offset of a box with `fourcc` among the children of `region`.
fn find_child(region: &[u8], fourcc: &[u8; 4]) -> Option<usize> {
    let mut at = 0usize;
    while at + HEADER_LEN <= region.len() {
        let size = u32::from_be_bytes(region[at..at + 4].try_into().ok()?) as usize;
        if size < HEADER_LEN || at + size > region.len() {
            return None;
        }
        if &region[at + TYPE_OFFSET..at + HEADER_LEN] == fourcc {
            return Some(at);
        }
        at += size;
    }
    None
}

/// The absolute offset of the first `pixi` property in a still-image file.
fn pixi_property_offset(bytes: &[u8]) -> usize {
    let meta = find_child(bytes, b"meta").expect("meta");
    let meta_body = meta + HEADER_LEN + 4;
    let iprp = meta_body + find_child(&bytes[meta_body..], b"iprp").expect("iprp");
    let iprp_body = iprp + HEADER_LEN;
    let ipco = iprp_body + find_child(&bytes[iprp_body..], b"ipco").expect("ipco");
    let ipco_body = ipco + HEADER_LEN;
    ipco_body + find_child(&bytes[ipco_body..], &PIXI).expect("pixi")
}

/// Every fixture here carries one `av1C` property; breaking its marker bit is a
/// form the crate's typed parse refuses.
#[test]
fn an_unrepresentable_property_stays_opaque_and_is_reported() {
    let mut cases = 0usize;
    for fixture in [
        "avif/plain.avif",
        "avif/alpha.avif",
        "avif/grid.avif",
        "avif/hdr10.avif",
        "heif/plain.heic",
        "heif/thumb.heic",
    ] {
        let original = fixtures::read(fixture);
        let mut bytes = original.clone();
        let count_at = pixi_property_offset(&bytes) + HEADER_LEN + PIXI_COUNT_OFFSET;
        bytes[count_at] = 0xFF;
        if bytes == original {
            continue;
        }
        let image = load(&bytes);
        let findings = image.structure_findings();
        let reported: Vec<&ImageFinding> = findings
            .iter()
            .filter(|finding| matches!(finding, ImageFinding::UnrepresentableBox { .. }))
            .collect();
        assert_eq!(
            reported.len(),
            1,
            "{fixture}: one finding, got {findings:?}"
        );
        let ImageFinding::UnrepresentableBox {
            box_type,
            offset,
            reason,
        } = *reported[0]
        else {
            unreachable!("filtered above")
        };
        assert_eq!(box_type, PIXI, "{fixture}: the finding names the property");
        assert_eq!(
            usize::try_from(offset).expect("offset"),
            count_at - HEADER_LEN - PIXI_COUNT_OFFSET,
            "{fixture}: the finding names the property's offset"
        );
        assert_eq!(
            reason,
            OpaqueReason::BufferTooShort,
            "{fixture}: the reason"
        );
        assert_eq!(
            image.reemit_into(&bytes).expect("re-emit"),
            bytes,
            "{fixture}: an opaque property re-emits byte-exact"
        );
        // The property is carried, not dropped.
        let carried = ImageFinding::UnrepresentableBox {
            box_type,
            offset,
            reason,
        };
        assert_eq!(carried.name(), "unrepresentable-box");
        cases += 1;
    }
    assert_eq!(cases, 6, "every listed fixture exercised the degradation");
}

/// A file whose properties all parse keeps its empty finding list, so the
/// degradation path changes nothing for it.
#[test]
fn a_readable_file_reports_no_degradation() {
    let bytes = fixtures::read("avif/plain.avif");
    let image = load(&bytes);
    assert!(
        !image
            .structure_findings()
            .iter()
            .any(|finding| matches!(finding, ImageFinding::UnrepresentableBox { .. })),
        "no property was degraded"
    );
    assert_eq!(image.reemit_into(&bytes).expect("re-emit"), bytes);
}
