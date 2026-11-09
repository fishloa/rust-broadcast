use broadcast_common::Serialize;
use compliance_probe::scte35::{Scte35Sanity, check_section};
use scte35_splice::SpliceInfoSection;
use scte35_splice::commands::{AnyCommand, SpliceInsert, SpliceInsertComponent};
use scte35_splice::time::SpliceTime;

fn bytes(section: &SpliceInfoSection<'_>) -> Vec<u8> {
    let mut buf = vec![0u8; section.serialized_len()];
    section.serialize_into(&mut buf).unwrap();
    buf
}

#[test]
fn cancelled_splice_insert_is_not_an_immediate_cue() {
    let si = SpliceInsert {
        splice_event_id: 7,
        splice_event_cancel_indicator: true,
        ..SpliceInsert::default()
    };
    let b = bytes(&SpliceInfoSection::new_clear(
        AnyCommand::SpliceInsert(si),
        &[],
    ));
    assert_ne!(
        check_section(&b, 90_000),
        Scte35Sanity::Immediate,
        "a splice_event_cancel_indicator=1 message carries no splice_time and no splice_immediate_flag; it is not an immediate splice"
    );
}

#[test]
fn component_mode_splice_insert_with_per_component_times_is_not_immediate() {
    let si = SpliceInsert {
        splice_event_id: 8,
        out_of_network_indicator: true,
        program_splice_flag: false,
        splice_immediate_flag: false,
        components: vec![SpliceInsertComponent {
            component_tag: 1,
            splice_time: Some(SpliceTime::with_pts(90_000 * 3)),
        }],
        ..SpliceInsert::default()
    };
    let b = bytes(&SpliceInfoSection::new_clear(
        AnyCommand::SpliceInsert(si),
        &[],
    ));
    assert_eq!(
        check_section(&b, 90_000 * 10),
        Scte35Sanity::InPast,
        "component-mode cue with splice_immediate_flag=0 and a 7 s stale component splice_time"
    );
}

#[test]
fn encrypted_section_is_not_malformed() {
    let mut s =
        SpliceInfoSection::new_clear(AnyCommand::SpliceInsert(SpliceInsert::default()), &[]);
    s.encrypted_packet = true;
    s.clear = None;
    let payload = [0x5Au8; 24];
    s.encrypted_payload = Some(&payload);
    s.encrypted_splice_command_length = Some(20);
    let b = bytes(&s);
    assert!(SpliceInfoSection::parse_ok(&b), "fixture must itself parse");
    assert_ne!(
        check_section(&b, 0),
        Scte35Sanity::Malformed,
        "a well-formed encrypted splice_info_section is valid (SCTE 35 9.6.1)"
    );
}

trait ParseOk {
    fn parse_ok(b: &[u8]) -> bool;
}
impl ParseOk for SpliceInfoSection<'_> {
    fn parse_ok(b: &[u8]) -> bool {
        use broadcast_common::Parse;
        SpliceInfoSection::parse(b).is_ok()
    }
}
