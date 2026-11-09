//! `hvcc-array-nal-type` and `hvc1-array-completeness` on real `hvcC` boxes
//! (ISO/IEC 14496-15:2017 §8.3.3.1.1 and §8.4.1.1.1).

use media_doctor::{Report, Severity, check_container_codec};

const NAL_TYPE_ID: &str = "hvcc-array-nal-type";
const COMPLETENESS_ID: &str = "hvc1-array-completeness";
const HVC1_CONFORMANT: &str = "mp4/hevc-frag.mp4";
const HEV1_DEFAULT: &str = "mp4/hevc-main.mp4";
const GPAC_HVC1_INCOMPLETE: &str = "video/dolby-vision/sollevante-dv-p8.1-1080p24-10s.mp4";

const HVCC_FIXED_LEN: usize = 23;
const ARRAY_HEADER_LEN: usize = 3;
const NAL_TYPE_MASK: u8 = 0x3F;
const AUD_NUT: u8 = 35;

fn ids(bytes: &[u8]) -> Vec<(String, Severity)> {
    let mut report = Report::new();
    check_container_codec(bytes, &mut report);
    report
        .findings()
        .iter()
        .filter(|f| f.rule_id == NAL_TYPE_ID || f.rule_id == COMPLETENESS_ID)
        .map(|f| (f.rule_id.clone(), f.severity))
        .collect()
}

fn hvcc_body_start(bytes: &[u8]) -> usize {
    bytes.windows(4).position(|w| w == b"hvcC").expect("hvcC") + 4
}

fn array_header_offsets(bytes: &[u8]) -> Vec<usize> {
    let body = hvcc_body_start(bytes);
    let mut at = body + HVCC_FIXED_LEN - 1;
    let count = usize::from(bytes[at]);
    at += 1;
    let mut offsets = Vec::new();
    for _ in 0..count {
        offsets.push(at);
        let nalus = usize::from(u16::from_be_bytes([bytes[at + 1], bytes[at + 2]]));
        at += ARRAY_HEADER_LEN;
        for _ in 0..nalus {
            let len = usize::from(u16::from_be_bytes([bytes[at], bytes[at + 1]]));
            at += 2 + len;
        }
    }
    offsets
}

#[test]
fn conformant_hvc1_and_default_hev1_report_nothing() {
    assert_eq!(ids(&test_fixtures::read(HVC1_CONFORMANT)), []);
    assert_eq!(ids(&test_fixtures::read(HEV1_DEFAULT)), []);
}

#[test]
fn gpac_hvc1_with_incomplete_parameter_sets_warns_per_array() {
    let Some(bytes) = test_fixtures::require_bytes(GPAC_HVC1_INCOMPLETE, "hevc_array_findings")
    else {
        return;
    };
    assert_eq!(
        ids(&bytes),
        vec![(COMPLETENESS_ID.to_owned(), Severity::Warning); 3]
    );
}

#[test]
fn an_unpermitted_array_type_warns() {
    let mut bytes = test_fixtures::read(HVC1_CONFORMANT);
    let sei = *array_header_offsets(&bytes).last().unwrap();
    bytes[sei] = (bytes[sei] & !NAL_TYPE_MASK) | AUD_NUT;
    assert_eq!(
        ids(&bytes),
        vec![(NAL_TYPE_ID.to_owned(), Severity::Warning)]
    );
}
