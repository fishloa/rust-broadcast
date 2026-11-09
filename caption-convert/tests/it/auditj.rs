use caption_convert::parse_webvtt;

#[test]
fn cue_identifier_starting_with_note_is_not_a_note_block() {
    let doc = "WEBVTT\n\nNOTES1\n00:00:01.000 --> 00:00:02.000\nhello\n";
    let parsed = parse_webvtt(doc).unwrap();
    assert_eq!(parsed.cues.len(), 1);
    assert_eq!(parsed.cues[0].text, "hello");
}

fn cc_bytes(process: bool, b1: u8, b2: u8) -> Vec<u8> {
    use broadcast_common::Serialize;
    use cc_data::{CcData, CcTriplet, CcType};
    let cc = CcData {
        process_em_data_flag: true,
        process_cc_data_flag: process,
        additional_data_flag: false,
        reserved_byte1: 0xFF,
        triplets: vec![CcTriplet {
            cc_valid: true,
            cc_type: CcType::Ntsc608Field1,
            cc_data_1: b1,
            cc_data_2: b2,
        }],
    };
    cc.try_to_bytes().unwrap()
}

#[test]
fn cc_data_with_process_cc_data_flag_clear_is_discarded() {
    let mut conv = caption_convert::Cea608ToWebVtt::new(cc_data::decode::Cea608Channel::Cc1);
    let frames: [(u8, u8); 4] = [(0x14, 0x20), (0x14, 0x70), (b'H', b'I'), (0x14, 0x2F)];
    for (i, (a, b)) in frames.into_iter().enumerate() {
        conv.push_cc_data(i as u64, &cc_bytes(false, a, b)).unwrap();
    }
    conv.finalize(10);
    assert!(conv.cues().is_empty(), "{:?}", conv.cues());
}
