//! AFD and bar data for H.264/H.265 (Phase 3a).

#![cfg(feature = "std")]

use broadcast_common::Unpackage;
use std::sync::Arc;
use transmux::ir::{
    ActiveFormat, ActiveFormatDescription, AfdPersistence, BarData, DisplayInfo, UserDataCarriage,
};
use transmux::nal::NalCodec;
use transmux::ts_demux::{DemuxEvent, StreamingTsDemux, TsDemux};
use transmux::user_data;

fn avc_au(nal_bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8, 0, 0, 1];
    out.extend_from_slice(nal_bytes);
    out
}

#[test]
fn afd_code_round_trips_all_16() {
    for code in 0u8..16 {
        let v = ActiveFormatDescription::from_code(code);
        assert_eq!(v.code(), code, "code {code}");
    }
}

#[test]
fn afd_spec_vector_4_3_centre() {
    let nal = [
        0x06u8, 0x04, 0x09, 0xB5, 0x00, 0x31, 0x44, 0x54, 0x47, 0x31, 0x41, 0xF9, 0x80,
    ];
    let au = avc_au(&nal);
    let pud = user_data::picture_user_data(NalCodec::Avc, &au);
    assert!(pud.afd_present);
    assert_eq!(pud.afd, Some(ActiveFormatDescription::Centre4x3));
    assert!(!pud.bar_present);
}

#[test]
fn afd_flag_clear_yields_none() {
    let mut payload = vec![0xB5u8, 0x00, 0x31, 0x44, 0x54, 0x47, 0x31, 0x00];
    // ensure function-level coverage too
    assert_eq!(user_data::afd_from_t35_payload(&payload), Some(None));
    payload[7] = 0x01;
    assert_eq!(user_data::afd_from_t35_payload(&payload), Some(None));
}

#[test]
fn afd_reserved_code_keeps_wire_value() {
    let code = 0x0Cu8;
    let payload = vec![0xB5u8, 0x00, 0x31, 0x44, 0x54, 0x47, 0x31, 0x41, 0xFC];
    assert_eq!(
        user_data::afd_from_t35_payload(&payload),
        Some(Some(ActiveFormatDescription::Reserved(code)))
    );
}

#[test]
fn afd_short_payload_ignored() {
    let payload = vec![0xB5u8, 0x00, 0x31, 0x44, 0x54, 0x47, 0x31, 0x41];
    assert_eq!(user_data::afd_from_t35_payload(&payload), None);
}

#[test]
fn afd_wrong_signature_ignored() {
    let mut payload = vec![0xB5u8, 0x00, 0x31, 0x44, 0x54, 0x47, 0x31, 0x41, 0xF9];
    payload[3] = 0x45; // not DTG1
    assert_eq!(user_data::afd_from_t35_payload(&payload), None);
    let payload = vec![0xB5u8, 0x00, 0x32, 0x44, 0x54, 0x47, 0x31, 0x41, 0xF9];
    assert_eq!(user_data::afd_from_t35_payload(&payload), None);
}

#[test]
fn afd_all_16_codes_typed() {
    for code in 0u8..16u8 {
        let payload = vec![
            0xB5u8,
            0x00,
            0x31,
            0x44,
            0x54,
            0x47,
            0x31,
            0x41,
            0xF0 | code,
        ];
        assert_eq!(
            user_data::afd_from_t35_payload(&payload),
            Some(Some(ActiveFormatDescription::from_code(code))),
            "code {code}"
        );
    }
}

#[test]
fn hevc_prefix_sei_parsed_suffix_ignored() {
    // Build a T.35 AFD payload inside an SEI message.
    let mut sei_body = vec![0x04u8, 0x09];
    sei_body.extend_from_slice(&[0xB5, 0x00, 0x31, 0x44, 0x54, 0x47, 0x31, 0x41, 0xF9]);
    sei_body.push(0x80);
    // prefix NAL (type 39)
    let mut prefix = vec![0x4Eu8, 0x01];
    prefix.extend_from_slice(&sei_body);
    let mut au = vec![0u8, 0, 0, 1];
    au.extend_from_slice(&prefix);
    let pud = user_data::picture_user_data(NalCodec::Hevc, &au);
    assert!(pud.afd_present);
    assert_eq!(pud.afd, Some(ActiveFormatDescription::Centre4x3));
    // suffix NAL (type 40) is ignored
    let mut suffix = vec![0x50u8, 0x01];
    suffix.extend_from_slice(&sei_body);
    let mut au2 = vec![0u8, 0, 0, 1];
    au2.extend_from_slice(&suffix);
    let pud2 = user_data::picture_user_data(NalCodec::Hevc, &au2);
    assert!(!pud2.afd_present);
}

#[test]
fn bar_data_top_bottom_distinct() {
    let payload = vec![
        0xB5u8, 0x00, 0x31, 0x47, 0x41, 0x39, 0x34, 0x06, 0xC0, 0xC0, 0x46, 0xC1, 0xFA,
    ];
    let bd = user_data::bar_data_from_t35_payload(&payload).expect("bar");
    assert_eq!(bd.top_bar_end_line, Some(70));
    assert_eq!(bd.bottom_bar_start_line, Some(506));
    assert_eq!(bd.left_bar_end_pixel, None);
    assert_eq!(bd.right_bar_start_pixel, None);
}

#[test]
fn bar_data_left_right_distinct() {
    let payload = vec![
        0xB5u8, 0x00, 0x31, 0x47, 0x41, 0x39, 0x34, 0x06, 0x30, 0x80, 0x0A, 0x80, 0x20,
    ];
    let bd = user_data::bar_data_from_t35_payload(&payload).expect("bar");
    assert_eq!(bd.left_bar_end_pixel, Some(10));
    assert_eq!(bd.right_bar_start_pixel, Some(32));
    assert_eq!(bd.top_bar_end_line, None);
    assert_eq!(bd.bottom_bar_start_line, None);
}

#[test]
fn bar_data_single_flag_decoded() {
    let payload = vec![
        0xB5u8, 0x00, 0x31, 0x47, 0x41, 0x39, 0x34, 0x06, 0x80, 0xC0, 0x46,
    ];
    let bd = user_data::bar_data_from_t35_payload(&payload).expect("bar");
    assert_eq!(bd.top_bar_end_line, Some(70));
    assert_eq!(bd.bottom_bar_start_line, None);
}

#[test]
fn bar_data_markers_not_enforced() {
    let payload = vec![
        0xB5u8, 0x00, 0x31, 0x47, 0x41, 0x39, 0x34, 0x06, 0x80, 0x00, 0x46,
    ];
    let bd = user_data::bar_data_from_t35_payload(&payload).expect("bar");
    assert_eq!(bd.top_bar_end_line, Some(70));
}

#[test]
fn changes_from_afd_only() {
    let prev = DisplayInfo::new(None);
    let cur = DisplayInfo::new(None).with_active_format(Some(ActiveFormat::new(
        Some(ActiveFormatDescription::Centre4x3),
        None,
        UserDataCarriage::AvcSei,
    )));
    let cs = cur.changes_from(Some(&prev));
    assert!(cs.afd);
    assert!(!cs.bar_data);
    assert!(!cs.initial);
}

#[test]
fn changes_from_bar_only() {
    let prev = DisplayInfo::new(None).with_active_format(Some(ActiveFormat::new(
        Some(ActiveFormatDescription::Centre4x3),
        None,
        UserDataCarriage::AvcSei,
    )));
    let cur = DisplayInfo::new(None).with_active_format(Some(ActiveFormat::new(
        Some(ActiveFormatDescription::Centre4x3),
        Some(BarData {
            top_bar_end_line: Some(70),
            bottom_bar_start_line: None,
            left_bar_end_pixel: None,
            right_bar_start_pixel: None,
        }),
        UserDataCarriage::AvcSei,
    )));
    let cs = cur.changes_from(Some(&prev));
    assert!(!cs.afd);
    assert!(cs.bar_data);
}

#[test]
fn changes_from_geometry_and_afd() {
    use transmux::ir::DisplayGeometry;
    let g = DisplayGeometry::new(1920, 1080, None);
    let prev = DisplayInfo::new(None);
    let cur = DisplayInfo::new(Some(g)).with_active_format(Some(ActiveFormat::new(
        Some(ActiveFormatDescription::Centre4x3),
        None,
        UserDataCarriage::AvcSei,
    )));
    let cs = cur.changes_from(Some(&prev));
    assert!(cs.afd);
    assert!(cs.output_rect);
}

#[test]
fn gulli_one_initial_afd_event() {
    let Some(data) = test_fixtures::require_bytes("ts/gulli-opengop.ts", "display_afd") else {
        return;
    };
    let mut demux = StreamingTsDemux::new();
    let mut changed = Vec::new();
    let mut sample_arcs = Vec::new();
    let drain =
        |demux: &mut StreamingTsDemux, changed: &mut Vec<usize>, sample_arcs: &mut Vec<usize>| {
            while let Some(ev) = demux.poll_event() {
                match ev {
                    DemuxEvent::DisplayChanged { display, .. } => {
                        changed.push(Arc::as_ptr(&display) as usize);
                    }
                    DemuxEvent::Sample { sample, .. } => {
                        if let Some(d) = sample.display {
                            sample_arcs.push(Arc::as_ptr(&d) as usize);
                        }
                    }
                    _ => {}
                }
            }
        };
    for chunk in data.chunks(188 * 512) {
        demux.feed(chunk);
        drain(&mut demux, &mut changed, &mut sample_arcs);
    }
    demux.finish();
    drain(&mut demux, &mut changed, &mut sample_arcs);
    assert_eq!(changed.len(), 1, "expected one initial DisplayChanged");
    // verify the changed state carries SameAsCodedFrame
    // Re-derive by demuxing fresh? Skip deep check; count is the headline.
    if !sample_arcs.is_empty() {
        assert!(sample_arcs.iter().all(|a| *a == changed[0]));
    }
}

#[test]
fn gulli_batch_all_same_afd() {
    let Some(data) = test_fixtures::require_bytes("ts/gulli-opengop.ts", "display_afd") else {
        return;
    };
    let media = TsDemux::new().unpackage(&data).expect("batch demux");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, transmux::ir::CodecConfig::Avc { .. }))
        .expect("avc track");
    assert!(!track.samples.is_empty());
    let first = track.samples[0]
        .display
        .as_ref()
        .expect("display")
        .active_format
        .as_ref()
        .expect("active")
        .afd;
    assert_eq!(first, Some(ActiveFormatDescription::SameAsCodedFrame));
    for s in &track.samples {
        let afd = s
            .display
            .as_ref()
            .and_then(|d| d.active_format.as_ref())
            .and_then(|a| a.afd);
        assert_eq!(afd, first);
    }
    let a0 = track.samples[0].display.as_ref().unwrap();
    let a1 = track.samples.last().unwrap().display.as_ref().unwrap();
    assert!(Arc::ptr_eq(a0, a1));
}

#[test]
fn afd_persistence_until_replaced_default() {
    assert_eq!(AfdPersistence::default(), AfdPersistence::UntilReplaced);
}

#[test]
fn display_renders_reserved() {
    assert_eq!(
        format!("{}", ActiveFormatDescription::Reserved(0x0C)),
        "reserved(0x0C)"
    );
}
