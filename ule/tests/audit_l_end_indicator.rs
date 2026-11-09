use ule::Sndu;

#[test]
fn d1_length_7fff_is_the_end_indicator_not_a_parseable_sndu() {
    let total = 4 + 0x7FFF;
    let mut d = vec![0u8; total];
    d[0] = 0xFF;
    d[1] = 0xFF;
    d[2] = 0x08;
    d[3] = 0x00;
    let crc_at = total - 4;
    let crc = broadcast_common::crc32_mpeg2::compute(&d[..crc_at]);
    d[crc_at..].copy_from_slice(&crc.to_be_bytes());
    if let Ok(p) = Sndu::parse(&d) {
        let mut o = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut o)
            .expect("parse accepted D=1/Length=0x7FFF (End Indicator) but serialize refuses it");
    }
}
