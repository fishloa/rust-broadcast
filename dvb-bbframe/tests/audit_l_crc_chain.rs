use broadcast_common::Serialize;
use dvb_bbframe::crc::crc8;
use dvb_bbframe::header::{Bbheader, Matype, Mode, TsGs};
use dvb_bbframe::packet::{CarryOverExtractor, NM_UP_SIZE};

fn nm_header(dfl_bytes: usize) -> [u8; 10] {
    let h = Bbheader {
        matype: Matype { ts_gs: TsGs::Ts, sis: true, ccm: true, issyi: false, npd: false, ext: 0, isi: 0 },
        upl: (NM_UP_SIZE * 8) as u16,
        sync: 0x47,
        dfl: (dfl_bytes * 8) as u16,
        syncd: 0,
        mode: Mode::Normal,
        issy_in_header: None,
    };
    h.serialize().unwrap()
}

fn frame(prev_crc: u8, body: u8) -> (Vec<u8>, u8) {
    let up = [body; NM_UP_SIZE - 1];
    let mut d = vec![prev_crc];
    d.extend_from_slice(&up);
    (d, crc8(&up))
}

#[test]
fn good_packet_after_a_lost_frame_is_not_flagged_transport_error() {
    let mut ex = CarryOverExtractor::new();
    let (a, crc_a) = frame(0, 0x11);
    let out = ex.feed_nm(&nm_header(a.len()), &a);
    assert_eq!(out.len(), 1);

    let (b, crc_b) = frame(crc_a, 0x22);
    let mut bad_hdr = nm_header(b.len());
    bad_hdr[9] ^= 0x55;
    assert!(ex.feed_nm(&bad_hdr, &b).is_empty());
    assert_eq!(ex.stats().header_parse_failures, 1);

    let (c, _) = frame(crc_b, 0x33);
    let out = ex.feed_nm(&nm_header(c.len()), &c);
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0][1] & 0x80,
        0,
        "UP is intact (its CRC-8 byte is valid for the lost predecessor) yet TEI was set; stats {:?}",
        ex.stats()
    );
}
