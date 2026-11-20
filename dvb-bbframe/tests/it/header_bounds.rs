//! DFL ceiling edge and NM stride validity boundaries.
//!
//! DFL's accepted ceiling is the DVB-S2 normal-FECFRAME bound (EN 302 307-1
//! §5.1.4); the NM per-UP stride comes from UPL/ISSYI/NPD (EN 302 755 §5.1.8).

use broadcast_common::Serialize;
use dvb_bbframe::crc::crc8;
use dvb_bbframe::error::Error;
use dvb_bbframe::header::{BBHEADER_LEN, Bbheader, DFL_MAX_BITS, Matype, Mode, TsGs};
use dvb_bbframe::packet::nm_stride_bytes;

fn nm_header(dfl: u16) -> Bbheader {
    Bbheader {
        matype: Matype {
            ts_gs: TsGs::Ts,
            sis: true,
            ccm: true,
            issyi: false,
            npd: false,
            ext: 0,
            isi: 0,
        },
        upl: 1504,
        sync: 0x47,
        dfl,
        syncd: 0,
        mode: Mode::Normal,
        issy_in_header: None,
    }
}

/// The loosest DFL bound this crate accepts, in bits (DVB-S2 FECFRAME).
#[test]
fn dfl_ceiling_is_64800_bits() {
    assert_eq!(DFL_MAX_BITS, 64800);
}

#[test]
fn dfl_at_the_ceiling_is_accepted_and_round_trips() {
    let hdr = nm_header(DFL_MAX_BITS);
    let bytes: [u8; BBHEADER_LEN] = hdr.serialize().expect("64800 bits is a valid DFL");
    let parsed = Bbheader::parse(&bytes).expect("the parser accepts its own ceiling");
    assert_eq!(parsed.dfl, DFL_MAX_BITS);
    assert_eq!(parsed.dfl, 64800);
}

#[test]
fn dfl_above_the_ceiling_is_rejected_on_parse() {
    for dfl in [DFL_MAX_BITS + 1, 64801, u16::MAX] {
        let mut bytes: [u8; BBHEADER_LEN] = nm_header(0).serialize().unwrap();
        bytes[4..6].copy_from_slice(&dfl.to_be_bytes());
        bytes[9] = crc8(&bytes[..9]);
        assert_eq!(
            Bbheader::parse(&bytes).err(),
            Some(Error::DflOutOfRange {
                dfl,
                max: DFL_MAX_BITS
            }),
            "DFL {dfl} must be rejected"
        );
    }
}

#[test]
fn dfl_above_the_ceiling_is_rejected_on_serialize() {
    for dfl in [DFL_MAX_BITS + 1, 64801, u16::MAX] {
        let mut buf = [0u8; BBHEADER_LEN];
        assert_eq!(
            nm_header(dfl).serialize_into(&mut buf).err(),
            Some(Error::DflOutOfRange {
                dfl,
                max: DFL_MAX_BITS
            }),
            "DFL {dfl} must not serialize"
        );
    }
}

#[test]
fn nm_stride_rejects_non_byte_aligned_upl() {
    // 1505 bits is not a whole number of bytes: framing at floor(1505/8)
    // would mis-frame every user packet in the frame.
    let mut hdr = nm_header(0);
    hdr.upl = 1505;
    assert_eq!(nm_stride_bytes(&hdr), None);

    hdr.upl = 0;
    assert_eq!(nm_stride_bytes(&hdr), None);

    hdr.upl = 1504;
    assert_eq!(nm_stride_bytes(&hdr), Some(188));
}
