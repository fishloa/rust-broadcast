//! Each table's `MIN_SECTION_LEN` floor is a hard lower bound on the declared
//! `section_length`: every value from one below the floor down to zero must be
//! rejected with `SectionLengthOverflow` while the buffer is at full length,
//! and the exact floor must be accepted.
//!
//! The floors are the spec layouts (common header + the table's fixed fields +
//! CRC), written out as literals: a table that lowers its floor would otherwise
//! parse a hostile section past its declared end with every other test green.

use broadcast_core::{Parse, Serialize};
use dvb_si::Error;
use dvb_si::descriptors::{DescriptorLoop, RntDescriptorLoop, UntDescriptorLoop};
use dvb_si::tables::SectionVersion;
use dvb_si::tables::ait::{AitSection, ApplicationType};
use dvb_si::tables::bat::BatSection;
use dvb_si::tables::cat::CatSection;
use dvb_si::tables::mpe_fec::{MpeFecSection, RealTimeParameters};
use dvb_si::tables::mpe_ifec::{MpeIfecSection, RealTimeParameters as IfecRealTimeParameters};
use dvb_si::tables::nit::{NitKind, NitSection};
use dvb_si::tables::pat::PatSection;
use dvb_si::tables::pmt::PmtSection;
use dvb_si::tables::protection_message::{ProtectionMessageBody, ProtectionMessageSection};
use dvb_si::tables::rct::RctSection;
use dvb_si::tables::rnt::{ContextIdType, RntSection};
use dvb_si::tables::sdt::{SdtKind, SdtSection};
use dvb_si::tables::sit::SitSection;
use dvb_si::tables::tot::TotSection;
use dvb_si::tables::tsdt::TsdtSection;
use dvb_si::tables::unt::{UntActionType, UntSection};
use mpeg_ts::pid::Pid;

fn set_section_length(bytes: &mut [u8], value: u16) {
    bytes[1] = (bytes[1] & 0xF0) | ((value >> 8) as u8 & 0x0F);
    bytes[2] = value as u8;
}

macro_rules! assert_floor {
    ($ty:ty, $bytes:expr, $min_total:expr, $check_floor:expr) => {{
        let bytes = $bytes;
        let min_total: usize = $min_total;
        let check_floor: usize = $check_floor;
        assert_eq!(bytes.len(), min_total, "minimal section size");
        assert_eq!(
            u16::from(bytes[1] & 0x0F) << 8 | u16::from(bytes[2]),
            (min_total - 3) as u16,
            "floor section_length"
        );

        let at_floor = <$ty>::parse(&bytes).expect("minimum-length section is accepted");
        let mut again = vec![0u8; at_floor.serialized_len()];
        at_floor.serialize_into(&mut again).unwrap();
        assert_eq!(again, bytes);

        for declared in 0..(check_floor - 3) {
            let mut short = bytes.clone();
            set_section_length(&mut short, declared as u16);
            assert_eq!(
                <$ty>::parse(&short).unwrap_err(),
                Error::SectionLengthOverflow {
                    declared,
                    available: bytes.len() - 3,
                },
                "section_length {declared} (below the floor)"
            );
        }

        for declared in (check_floor - 3)..(min_total - 3) {
            let mut short = bytes.clone();
            set_section_length(&mut short, declared as u16);
            assert!(
                <$ty>::parse(&short).is_err(),
                "section_length {declared} leaves no room for the fixed fields"
            );
        }
    }};
}

fn version() -> SectionVersion {
    SectionVersion::new(0, true, 0, 0)
}

#[test]
fn cat_floor_is_twelve_bytes() {
    let cat = CatSection::new(0, true, 0, 0, DescriptorLoop::new(&[]));
    let mut bytes = vec![0u8; cat.serialized_len()];
    cat.serialize_into(&mut bytes).unwrap();
    assert_floor!(CatSection<'_>, bytes, 12, 12);
}

#[test]
fn tsdt_floor_is_twelve_bytes() {
    let tsdt = TsdtSection::new(0, 0, true, 0, 0, DescriptorLoop::new(&[]));
    let mut bytes = vec![0u8; tsdt.serialized_len()];
    tsdt.serialize_into(&mut bytes).unwrap();
    assert_floor!(TsdtSection<'_>, bytes, 12, 12);
}

#[test]
fn pat_floor_is_twelve_bytes() {
    let pat = PatSection::new(1, 0, true, 0, 0, vec![]);
    let mut bytes = vec![0u8; pat.serialized_len()];
    pat.serialize_into(&mut bytes).unwrap();
    assert_floor!(PatSection, bytes, 12, 12);
}

#[test]
fn pmt_floor_is_sixteen_bytes() {
    let pmt = PmtSection::new(
        1,
        version(),
        Pid::from_wire_masked(0x1FFF),
        DescriptorLoop::new(&[]),
        vec![],
    );
    let mut bytes = vec![0u8; pmt.serialized_len()];
    pmt.serialize_into(&mut bytes).unwrap();
    assert_floor!(PmtSection<'_>, bytes, 16, 16);
}

#[test]
fn nit_floor_is_sixteen_bytes() {
    let nit = NitSection::new(
        NitKind::Actual,
        1,
        version(),
        DescriptorLoop::new(&[]),
        vec![],
    );
    let mut bytes = vec![0u8; nit.serialized_len()];
    nit.serialize_into(&mut bytes).unwrap();
    assert_floor!(NitSection<'_>, bytes, 16, 14);
}

#[test]
fn bat_floor_is_sixteen_bytes() {
    let bat = BatSection::new(1, 0, true, 0, 0, DescriptorLoop::new(&[]), vec![]);
    let mut bytes = vec![0u8; bat.serialized_len()];
    bat.serialize_into(&mut bytes).unwrap();
    assert_floor!(BatSection<'_>, bytes, 16, 14);
}

#[test]
fn ait_floor_is_sixteen_bytes() {
    let ait = AitSection::new(
        ApplicationType::HbbTv,
        false,
        version(),
        DescriptorLoop::new(&[]),
        vec![],
    );
    let mut bytes = vec![0u8; ait.serialized_len()];
    ait.serialize_into(&mut bytes).unwrap();
    assert_floor!(AitSection<'_>, bytes, 16, 16);
}

#[test]
fn mpe_fec_floor_is_sixteen_bytes() {
    let rtp = RealTimeParameters {
        delta_t: 0,
        table_boundary: false,
        frame_boundary: false,
        address: 0,
    };
    let fec = MpeFecSection::new(true, 0, true, 0, 0, rtp, &[]);
    let mut bytes = vec![0u8; fec.serialized_len()];
    fec.serialize_into(&mut bytes).unwrap();
    assert_floor!(MpeFecSection<'_>, bytes, 16, 16);
}

#[test]
fn mpe_ifec_floor_is_sixteen_bytes() {
    let rtp = IfecRealTimeParameters {
        delta_t: 0,
        mpe_boundary: false,
        frame_boundary: false,
        prev_burst_size: 0,
    };
    let ifec = MpeIfecSection::new(true, 0, 0, version(), rtp, &[]);
    let mut bytes = vec![0u8; ifec.serialized_len()];
    ifec.serialize_into(&mut bytes).unwrap();
    assert_floor!(MpeIfecSection<'_>, bytes, 16, 16);
}

#[test]
fn rct_floor_is_seventeen_bytes() {
    let rct = RctSection::new(false, 1, version(), 0, vec![], DescriptorLoop::new(&[]));
    let mut bytes = vec![0u8; rct.serialized_len()];
    rct.serialize_into(&mut bytes).unwrap();
    assert_floor!(RctSection<'_>, bytes, 17, 17);
}

#[test]
fn rnt_floor_is_fifteen_bytes() {
    let rnt = RntSection::new(
        1,
        version(),
        ContextIdType::BouquetId,
        RntDescriptorLoop::new(&[]),
        vec![],
    );
    let mut bytes = vec![0u8; rnt.serialized_len()];
    rnt.serialize_into(&mut bytes).unwrap();
    assert_floor!(RntSection<'_>, bytes, 15, 15);
}

#[test]
fn sdt_floor_is_fifteen_bytes() {
    let sdt = SdtSection::new(SdtKind::Actual, 1, version(), 1, vec![]);
    let mut bytes = vec![0u8; sdt.serialized_len()];
    sdt.serialize_into(&mut bytes).unwrap();
    assert_floor!(SdtSection<'_>, bytes, 15, 15);
}

#[test]
fn sit_floor_is_fourteen_bytes() {
    let sit = SitSection::new(1, 0, true, 0, 0, DescriptorLoop::new(&[]), vec![]);
    let mut bytes = vec![0u8; sit.serialized_len()];
    sit.serialize_into(&mut bytes).unwrap();
    assert_floor!(SitSection<'_>, bytes, 14, 14);
}

#[test]
fn tot_floor_is_fourteen_bytes() {
    let tot = TotSection::new([0xE4, 0x09, 0x12, 0x34, 0x56], DescriptorLoop::new(&[]));
    let mut bytes = vec![0u8; tot.serialized_len()];
    tot.serialize_into(&mut bytes).unwrap();
    assert_floor!(TotSection<'_>, bytes, 14, 14);
}

#[test]
fn unt_floor_is_eighteen_bytes() {
    let unt = UntSection::new(
        UntActionType::Reserved,
        0,
        version(),
        0,
        0,
        UntDescriptorLoop::new(&[]),
        vec![],
    );
    let mut bytes = vec![0u8; unt.serialized_len()];
    unt.serialize_into(&mut bytes).unwrap();
    assert_floor!(UntSection<'_>, bytes, 18, 18);
}

#[test]
fn protection_message_floor_is_twelve_bytes() {
    let section =
        ProtectionMessageSection::new(0x0200, 0, true, 0, 0, ProtectionMessageBody::Raw(&[]));
    let mut bytes = vec![0u8; section.serialized_len()];
    section.serialize_into(&mut bytes).unwrap();
    assert_floor!(ProtectionMessageSection<'_>, bytes, 12, 12);
}
