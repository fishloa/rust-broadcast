//! Integration test: build a `ca_pmt` from a real broadcast PMT section.
//!
//! `real-pmt.bin` is a genuine TSDuck-captured PMT section (table_id 0x02) with
//! several MPEG descriptors but no CA_descriptor. The builder must parse it
//! through `dvb-si`, strip every (non-CA) descriptor, and emit a well-formed,
//! round-trippable `ca_pmt` carrying the full component list with empty CA loops.

use broadcast_common::{Parse, Serialize};
use dvb_ci::builder::build_ca_pmt;
use dvb_ci::objects::ca_pmt::{CaPmt, CaPmtCmdId, CaPmtListManagement};
use dvb_si::tables::pmt::PmtSection;

const REAL_PMT: &[u8] = include_bytes!("fixtures/real-pmt.bin");

#[test]
fn builds_ca_pmt_from_real_broadcast_pmt() {
    let pmt = PmtSection::parse(REAL_PMT).expect("real PMT parses");
    let original_streams = pmt.streams.len();
    assert!(original_streams > 0, "fixture should carry ES");

    let built = build_ca_pmt(&pmt, CaPmtListManagement::Only, CaPmtCmdId::OkDescrambling);
    let wire = built.to_bytes();

    // Re-parse the produced ca_pmt and confirm it round-trips byte-for-byte.
    let parsed = CaPmt::parse(&wire).expect("produced ca_pmt parses");
    assert_eq!(parsed, built.as_ca_pmt());
    assert_eq!(parsed.to_bytes(), wire);

    // Programme number/version carried verbatim from the PMT.
    assert_eq!(parsed.program_number, pmt.program_number);
    assert_eq!(parsed.version_number, pmt.version_number);

    // Every component is carried, and since the fixture has no CA_descriptor all
    // CA loops are empty and no ca_pmt_cmd_id bytes are present.
    assert_eq!(parsed.streams.len(), original_streams);
    assert!(parsed.program_ca_descriptors.is_empty());
    assert!(parsed.cmd_id.is_none());
    for s in &parsed.streams {
        assert!(s.ca_descriptors.is_empty());
        assert!(s.cmd_id.is_none());
    }
}
