use mpeg_pes::PesPacket;

fn accepted_input_must_round_trip(b: &[u8]) {
    if let Ok(p) = PesPacket::parse(b) {
        let mut o = vec![0u8; p.serialized_len()];
        let n = p.serialize_into(&mut o).unwrap();
        assert_eq!(&o[..n], b, "parsed OK but serialize rewrote the bytes");
    }
}

#[test]
fn escr_with_cleared_marker_bit_is_rejected_or_preserved() {
    let mut b = vec![0x00, 0x00, 0x01, 0xE0, 0x00, 0x0B, 0x80, 0x20, 0x06];
    let mut escr = mpeg_pes::Escr::from_27mhz(12345).to_field_bytes();
    escr[2] &= !0x04;
    b.extend_from_slice(&escr);
    b.extend_from_slice(&[0xAA, 0xBB]);
    accepted_input_must_round_trip(&b);
}

#[test]
fn p_std_buffer_with_wrong_fixed_prefix_is_rejected_or_preserved() {
    let mut b = vec![0x00, 0x00, 0x01, 0xE0, 0x00, 0x0A, 0x80, 0x01, 0x03, 0x10, 0x97, 0x04, 0xAA, 0xBB];
    b[5] = (b.len() - 6) as u8;
    accepted_input_must_round_trip(&b);
}

#[test]
fn program_packet_sequence_counter_with_cleared_markers_is_rejected_or_preserved() {
    let mut b = vec![0x00, 0x00, 0x01, 0xE0, 0x00, 0x0A, 0x80, 0x01, 0x03, 0x20, 0x05, 0x01, 0xAA, 0xBB];
    b[5] = (b.len() - 6) as u8;
    accepted_input_must_round_trip(&b);
}
