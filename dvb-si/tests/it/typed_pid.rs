//! Typed-PID boundary: every PID-carrying constructor takes a `Pid`, so a
//! value above 13 bits is unrepresentable (`Pid::try_new` rejects it before a
//! section/descriptor can be built), and the maximum PID `0x1FFF` serializes
//! to exactly the 13 wire bits with the reserved bits untouched.
use broadcast_common::{Parse, Serialize};
use dvb_si::descriptors::DescriptorLoop;
use dvb_si::descriptors::ca::CaDescriptor;
use dvb_si::descriptors::extension::Cp;
use dvb_si::tables::pat::{PatEntry, PatSection};
use dvb_si::tables::pmt::{PmtSection, PmtStream, StreamType};
use mpeg_ts::pid::Pid;

const OVER_13_BITS: [u16; 3] = [0x2000, 0x8000, 0xFFFF];

#[test]
fn every_over_range_value_is_rejected_at_the_boundary() {
    for v in OVER_13_BITS {
        let err = Pid::try_new(v).expect_err("over-13-bit PID must be rejected");
        assert_eq!(err.value, v);
    }
    assert_eq!(Pid::try_new(0x1FFF).unwrap().value(), 0x1FFF);
}

#[test]
fn max_pid_round_trips_through_every_typed_api() {
    let max = Pid::try_new(0x1FFF).unwrap();

    // PAT program_map_PID: reserved(3)=111 then 13 PID bits.
    let pat = PatSection::new(1, 0, true, 0, 0, vec![PatEntry::new(1, max)]);
    let mut buf = vec![0u8; pat.serialized_len()];
    pat.serialize_into(&mut buf).unwrap();
    assert_eq!(&buf[10..12], &[0xFF, 0xFF]);
    assert_eq!(PatSection::parse(&buf).unwrap().entries[0].pid, max);

    // PMT PCR_PID + elementary_PID.
    let pmt = PmtSection::new(
        1,
        0,
        true,
        0,
        0,
        max,
        DescriptorLoop::new(&[]),
        vec![PmtStream::new(
            StreamType::H264,
            Pid::try_new(0x0100).unwrap(),
            DescriptorLoop::new(&[]),
        )],
    );
    let mut buf = vec![0u8; pmt.serialized_len()];
    pmt.serialize_into(&mut buf).unwrap();
    assert_eq!(&buf[8..10], &[0xFF, 0xFF]);
    let back = PmtSection::parse(&buf).unwrap();
    assert_eq!(back.pcr_pid, max);
    assert_eq!(back.streams[0].elementary_pid.value(), 0x0100);

    // CA_descriptor CA_PID.
    let ca = CaDescriptor::new(0x0500, max, &[]);
    let mut buf = vec![0u8; ca.serialized_len()];
    ca.serialize_into(&mut buf).unwrap();
    assert_eq!(&buf[4..6], &[0xFF, 0xFF]);
    assert_eq!(CaDescriptor::parse(&buf).unwrap().ca_pid, max);

    // CP_descriptor CP_PID.
    let cp = Cp::new(0x1234, max, &[]);
    let mut buf = vec![0u8; cp.serialized_len()];
    cp.serialize_into(&mut buf).unwrap();
    assert_eq!(&buf[2..4], &[0xFF, 0xFF]);
    assert_eq!(Cp::parse(&buf).unwrap().cp_pid, max);
}

#[cfg(feature = "serde")]
#[test]
fn serde_output_keeps_pids_as_plain_numbers() {
    let pat = PatSection::new(
        1,
        0,
        true,
        0,
        0,
        vec![PatEntry::new(1, Pid::try_new(0x0100).unwrap())],
    );
    let v = serde_json::to_value(&pat).unwrap();
    assert_eq!(v["entries"][0]["pid"], serde_json::json!(256));
    let ca = CaDescriptor::new(0x0500, Pid::try_new(0x1FFF).unwrap(), &[]);
    let v = serde_json::to_value(&ca).unwrap();
    assert_eq!(v["ca_pid"], serde_json::json!(8191));
}
