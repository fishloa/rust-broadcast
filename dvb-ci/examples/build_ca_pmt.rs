//! Build a `ca_pmt` object from a `dvb-si` PMT and print the wire bytes.
//!
//! Run with: `cargo run -p dvb-ci --example build_ca_pmt`
//!
//! Shows the headline workflow: a host takes a parsed MPEG-2 PMT, strips every
//! non-CA descriptor, and hands the CA-only `ca_pmt` object to a CICAM.

use dvb_ci::builder::build_ca_pmt;
use dvb_ci::objects::ca_pmt::{CaPmt, CaPmtCmdId, CaPmtListManagement};
use dvb_common::Parse;
use dvb_si::tables::pmt::PmtSection;

fn main() {
    // A small PMT (program 1): a programme-level CA_descriptor (tag 0x09) plus a
    // non-CA registration descriptor, one scrambled video ES with an ES-level
    // CA_descriptor, and one clear audio ES with only a language descriptor.
    let pmt_bytes = sample_pmt();
    let pmt = PmtSection::parse(&pmt_bytes).expect("valid PMT");

    let built = build_ca_pmt(&pmt, CaPmtListManagement::Only, CaPmtCmdId::OkDescrambling);
    let wire = built.to_bytes();

    println!("ca_pmt APDU ({} bytes): {:02X?}", wire.len(), wire);

    // Re-parse to confirm the projection round-trips.
    let parsed = CaPmt::parse(&wire).expect("valid ca_pmt");
    println!("list_management : {}", parsed.list_management);
    println!("program_number : {}", parsed.program_number);
    println!(
        "program CA desc : {} byte(s) (non-CA descriptors stripped)",
        parsed.program_ca_descriptors.len()
    );
    for (i, s) in parsed.streams.iter().enumerate() {
        println!(
            "  ES[{i}] type=0x{:02X} pid=0x{:04X} ca_descriptors={} cmd_id={:?}",
            s.stream_type,
            s.elementary_pid,
            s.ca_descriptors.len(),
            s.cmd_id.map(|c| c.name())
        );
    }
}

fn sample_pmt() -> Vec<u8> {
    let prog_ca = [0x09, 0x04, 0x05, 0x00, 0xE1, 0x00]; // CA_system_id 0x0500, PID 0x0100
    let reg = [0x05, 0x04, b'H', b'D', b'M', b'V']; // non-CA, will be stripped
    let mut program_info = Vec::new();
    program_info.extend_from_slice(&prog_ca);
    program_info.extend_from_slice(&reg);

    let es0_ca = [0x09, 0x04, 0x05, 0x00, 0xE1, 0x01];
    let lang = [0x0A, 0x04, b'e', b'n', b'g', 0x00];

    let mut body = vec![0x02, 0x00, 0x00, 0x00, 0x01, 0xC3, 0x00, 0x00, 0xE2, 0x00];
    let pil = program_info.len();
    body.push(0xF0 | ((pil >> 8) as u8 & 0x0F));
    body.push(pil as u8);
    body.extend_from_slice(&program_info);

    body.extend_from_slice(&[
        0x02,
        0xE2,
        0x00,
        0xF0 | ((es0_ca.len() >> 8) as u8),
        es0_ca.len() as u8,
    ]);
    body.extend_from_slice(&es0_ca);
    body.extend_from_slice(&[
        0x03,
        0xE2,
        0x01,
        0xF0 | ((lang.len() >> 8) as u8),
        lang.len() as u8,
    ]);
    body.extend_from_slice(&lang);

    let section_length = body.len() - 3 + 4;
    body[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
    body[2] = section_length as u8;
    let crc = dvb_common::crc32_mpeg2::compute(&body);
    body.extend_from_slice(&crc.to_be_bytes());
    body
}
