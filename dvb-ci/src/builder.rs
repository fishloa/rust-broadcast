//! `CA_PMT` builder — project a `dvb-si` PMT into the `ca_pmt` object handed to
//! a CICAM — ETSI EN 50221 §8.4.3.4 (Table 25), per `docs/en_50221/ca-pmt.md`.
//!
//! The host extracts the PMT, strips every descriptor that is not a
//! `CA_descriptor()` (ISO/IEC 13818-1 §2.6.16, tag `0x09`), and keeps the
//! surviving CA descriptors at programme and elementary-stream level (per
//! `ca-pmt.md` field notes: "Only CA_descriptors are present; all other
//! descriptors are removed from the PMT by the host"). Each surviving descriptor
//! loop is prefixed with a `ca_pmt_cmd_id` byte.
//!
//! The filtered descriptor bytes do not exist as a contiguous slice in the
//! source PMT, so [`build_ca_pmt`] returns an owned [`CaPmtBuilt`] that holds the
//! filtered loops; borrow a [`CaPmt`] view from it with
//! [`CaPmtBuilt::as_ca_pmt`], or take the finished wire bytes with
//! [`CaPmtBuilt::to_bytes`].
//!
//! [`CaPmt`]: crate::objects::ca_pmt::CaPmt

use crate::objects::ca_pmt::{
    CaPmt, CaPmtCmdId, CaPmtListManagement, CaPmtStream, CA_DESCRIPTOR_TAG,
};
use alloc::vec::Vec;
use dvb_common::Serialize;
use dvb_si::descriptors::DescriptorLoop;
use dvb_si::tables::pmt::PmtSection;

/// An owned, CA-only projection of a PMT. Holds the filtered `CA_descriptor`
/// loops (programme + per-ES) so a borrowed [`CaPmt`] can be reconstructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaPmtBuilt {
    list_management: CaPmtListManagement,
    program_number: u16,
    version_number: u8,
    current_next_indicator: bool,
    cmd_id: CaPmtCmdId,
    program_ca_descriptors: Vec<u8>,
    streams: Vec<BuiltStream>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BuiltStream {
    stream_type: u8,
    elementary_pid: u16,
    ca_descriptors: Vec<u8>,
}

/// Filter a descriptor loop to only its `CA_descriptor()` entries (tag `0x09`),
/// concatenating the surviving entries' verbatim TLV wire bytes.
fn ca_descriptors_only(loop_: &DescriptorLoop<'_>) -> Vec<u8> {
    let mut out = Vec::new();
    for (tag, body) in loop_.raw_tags() {
        if tag == CA_DESCRIPTOR_TAG {
            // Re-emit the full TLV: tag, length, body. raw_tags has already
            // validated `body.len()` fits in the declared length byte.
            out.push(tag);
            out.push(body.len() as u8);
            out.extend_from_slice(body);
        }
    }
    out
}

/// Build the `ca_pmt` projection of `pmt` for the given list-management and
/// command-id. Strips all non-CA descriptors; keeps `CA_descriptor`s at
/// programme and ES level.
///
/// Every elementary stream of the PMT is carried; a stream with no surviving CA
/// descriptor has no `ca_pmt_cmd_id` (its `ES_info_length` is 0 per Table 25),
/// so the CAM sees the full component list while only CA-bearing streams carry
/// CA info.
#[must_use]
pub fn build_ca_pmt(
    pmt: &PmtSection<'_>,
    list_management: CaPmtListManagement,
    cmd_id: CaPmtCmdId,
) -> CaPmtBuilt {
    let program_ca_descriptors = ca_descriptors_only(&pmt.program_info);
    let streams = pmt
        .streams
        .iter()
        .map(|s| BuiltStream {
            stream_type: s.stream_type.to_u8(),
            elementary_pid: s.elementary_pid,
            ca_descriptors: ca_descriptors_only(&s.es_info),
        })
        .collect();
    CaPmtBuilt {
        list_management,
        program_number: pmt.program_number,
        version_number: pmt.version_number,
        current_next_indicator: pmt.current_next_indicator,
        cmd_id,
        program_ca_descriptors,
        streams,
    }
}

impl CaPmtBuilt {
    /// Borrow a [`CaPmt`] view over the owned filtered descriptor loops. The
    /// `ca_pmt_cmd_id` is attached to a loop only when that loop has surviving
    /// CA descriptors (matching Table 25's `..._info_length != 0` guard).
    #[must_use]
    pub fn as_ca_pmt(&self) -> CaPmt<'_> {
        CaPmt {
            list_management: self.list_management,
            program_number: self.program_number,
            version_number: self.version_number,
            current_next_indicator: self.current_next_indicator,
            cmd_id: cmd_for(self.cmd_id, &self.program_ca_descriptors),
            program_ca_descriptors: &self.program_ca_descriptors,
            streams: self
                .streams
                .iter()
                .map(|s| CaPmtStream {
                    stream_type: s.stream_type,
                    elementary_pid: s.elementary_pid,
                    cmd_id: cmd_for(self.cmd_id, &s.ca_descriptors),
                    ca_descriptors: &s.ca_descriptors,
                })
                .collect(),
        }
    }

    /// Serialize the finished `ca_pmt` APDU (tag `9F 80 32` + length + body).
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        self.as_ca_pmt().to_bytes()
    }
}

/// A `ca_pmt_cmd_id` accompanies a descriptor loop only when that loop is
/// non-empty (otherwise `..._info_length` is 0 and no cmd_id byte is present).
fn cmd_for(cmd_id: CaPmtCmdId, descriptors: &[u8]) -> Option<CaPmtCmdId> {
    if descriptors.is_empty() {
        None
    } else {
        Some(cmd_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::ca_pmt::CaPmt;
    use dvb_common::Parse;

    #[test]
    fn builds_from_real_pmt_fixture() {
        // The m6-single.ts fixture in dvb-si carries a real broadcast PMT with
        // CA descriptors. Build a PMT section from a hand-rolled wire buffer that
        // mirrors a real CA-protected service: program CA_descriptor + two ES,
        // one scrambled (with ES CA_descriptor) and one clear.
        let pmt_bytes = build_test_pmt();
        let pmt = PmtSection::parse(&pmt_bytes).expect("valid PMT");

        let built = build_ca_pmt(&pmt, CaPmtListManagement::Only, CaPmtCmdId::OkDescrambling);
        let bytes = built.to_bytes();

        // Round-trips through the ca_pmt parser.
        let parsed = CaPmt::parse(&bytes).unwrap();
        let view = built.as_ca_pmt();
        assert_eq!(parsed, view);

        // Programme-level CA descriptor survived; non-CA descriptors stripped.
        assert!(!parsed.program_ca_descriptors.is_empty());
        assert_eq!(parsed.program_ca_descriptors[0], CA_DESCRIPTOR_TAG);
        assert_eq!(parsed.cmd_id, Some(CaPmtCmdId::OkDescrambling));

        // Both ES carried; only the scrambled one has CA info + cmd_id.
        assert_eq!(parsed.streams.len(), 2);
        assert!(!parsed.streams[0].ca_descriptors.is_empty());
        assert_eq!(parsed.streams[0].cmd_id, Some(CaPmtCmdId::OkDescrambling));
        assert!(parsed.streams[1].ca_descriptors.is_empty());
        assert_eq!(parsed.streams[1].cmd_id, None);
    }

    #[test]
    fn strips_non_ca_descriptors() {
        let pmt_bytes = build_test_pmt();
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();
        let built = build_ca_pmt(&pmt, CaPmtListManagement::Add, CaPmtCmdId::Query);
        let view = built.as_ca_pmt();
        // The source program_info had a non-CA descriptor too; only 0x09 remains.
        let mut pos = 0;
        let d = view.program_ca_descriptors;
        while pos < d.len() {
            assert_eq!(d[pos], CA_DESCRIPTOR_TAG);
            pos += 2 + d[pos + 1] as usize;
        }
    }

    // --- helper: assemble a small but realistic PMT with CA descriptors ---

    fn ca_descriptor(ca_system_id: u16, pid: u16) -> [u8; 6] {
        [
            0x09,
            0x04,
            (ca_system_id >> 8) as u8,
            ca_system_id as u8,
            0xE0 | ((pid >> 8) as u8 & 0x1F),
            pid as u8,
        ]
    }

    fn build_test_pmt() -> Vec<u8> {
        // program_info: a CA_descriptor + a (non-CA) registration descriptor(0x05).
        let prog_ca = ca_descriptor(0x0500, 0x0100);
        let reg = [0x05u8, 0x04, b'H', b'D', b'M', b'V'];
        let mut program_info = Vec::new();
        program_info.extend_from_slice(&prog_ca);
        program_info.extend_from_slice(&reg);

        // ES0: scrambled video, stream_type 0x02, pid 0x0200, with ES CA_descriptor.
        let es0_ca = ca_descriptor(0x0500, 0x0101);
        // ES1: clear audio, stream_type 0x03, pid 0x0201, only a language descriptor.
        let lang = [0x0Au8, 0x04, b'e', b'n', b'g', 0x00];

        let mut body = Vec::new();
        // table_id 0x02
        body.push(0x02);
        // section_length placeholder (filled later): 2 bytes
        body.push(0);
        body.push(0);
        // program_number 0x0001
        body.extend_from_slice(&[0x00, 0x01]);
        // reserved(2)|version(5)|cni(1): version 1, cni 1 -> 0b110000_11 = 0xC3
        body.push(0xC3);
        // section_number, last_section_number
        body.push(0x00);
        body.push(0x00);
        // reserved(3)|PCR_PID(13): pid 0x0200
        body.push(0xE0 | 0x02);
        body.push(0x00);
        // reserved(4)|program_info_length(12)
        let pil = program_info.len();
        body.push(0xF0 | ((pil >> 8) as u8 & 0x0F));
        body.push(pil as u8);
        body.extend_from_slice(&program_info);

        // ES0
        body.push(0x02); // stream_type
        body.push(0xE0 | 0x02); // pid 0x0200
        body.push(0x00);
        body.push(0xF0 | ((es0_ca.len() >> 8) as u8 & 0x0F));
        body.push(es0_ca.len() as u8);
        body.extend_from_slice(&es0_ca);

        // ES1
        body.push(0x03);
        body.push(0xE0 | 0x02); // pid 0x0201
        body.push(0x01);
        body.push(0xF0 | ((lang.len() >> 8) as u8 & 0x0F));
        body.push(lang.len() as u8);
        body.extend_from_slice(&lang);

        // Now fix section_length = (bytes after the length field) + CRC(4).
        let section_length = body.len() - 3 + 4;
        body[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        body[2] = section_length as u8;

        // Append a CRC (the parser validates length, not CRC for construction;
        // compute the real MPEG-2 CRC so the section is well-formed).
        let crc = dvb_common::crc32_mpeg2::compute(&body);
        body.extend_from_slice(&crc.to_be_bytes());
        body
    }
}
