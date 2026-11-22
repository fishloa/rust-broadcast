//! Hand-built PSI sections and TS packets: a deliberately independent oracle.
//!
//! Every byte is laid out here from the section syntax (ISO/IEC 13818-1
//! Table 2-30/2-33), including a bitwise CRC-32/MPEG-2, so a demux test that
//! feeds these never trusts the production serializer or CRC it is checking.

const TS_PACKET_SIZE: usize = 188;
const SYNC_BYTE: u8 = 0x47;
const PUSI: u8 = 0x40;
const PAYLOAD_ONLY: u8 = 0x10;
const PID_HI_MASK: u8 = 0x1F;
const STUFFING: u8 = 0xFF;
const PAYLOAD_START: usize = 5;
const SSI_RESERVED: u8 = 0xB0;
const LENGTH_HI_MASK: u8 = 0x0F;
const VERSION_RESERVED: u8 = 0xC1;
const VERSION_MASK: u8 = 0x1F;
const PID_RESERVED: u8 = 0xE0;
const PROGRAM_INFO_LEN_HI: u8 = 0xF0;
const LONG_FORM_EXTRA: usize = 5;
const CRC_LEN: usize = 4;
const CRC_POLY: u32 = 0x04C1_1DB7;
const CRC_INIT: u32 = 0xFFFF_FFFF;
const PAT_TABLE_ID: u8 = 0x00;
const PMT_TABLE_ID: u8 = 0x02;
const STREAM_TYPE_MPEG2_VIDEO: u8 = 0x02;
const PAT_PID: u16 = 0;

/// Bitwise CRC-32/MPEG-2 (poly `0x04C11DB7`, init all ones, no reflection, no xorout).
#[must_use]
pub fn crc32_mpeg2(bytes: &[u8]) -> u32 {
    let mut crc = CRC_INIT;
    for &b in bytes {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ CRC_POLY
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// A section header (`table_id`, syntax indicator, length) followed by
/// `body_after_length` verbatim: no extension header and no CRC.
///
/// # Panics
/// If the body does not fit a 12-bit `section_length`.
#[must_use]
pub fn short_section(table_id: u8, body_after_length: &[u8]) -> Vec<u8> {
    let section_length = u16::try_from(body_after_length.len())
        .ok()
        .filter(|l| *l <= 0x0FFF)
        .expect("section_length is 12 bits");
    let mut v = vec![
        table_id,
        SSI_RESERVED | (section_length >> 8) as u8 & LENGTH_HI_MASK,
        (section_length & 0xFF) as u8,
    ];
    v.extend_from_slice(body_after_length);
    v
}

/// A long-form section (current, `last_section_number == section_number`) with a correct CRC-32.
///
/// # Panics
/// If the payload does not fit a 12-bit `section_length`.
#[must_use]
pub fn long_section(
    table_id: u8,
    ext: u16,
    version: u8,
    section_number: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut after_length = Vec::with_capacity(LONG_FORM_EXTRA + payload.len() + CRC_LEN);
    after_length.extend_from_slice(&ext.to_be_bytes());
    after_length.push(VERSION_RESERVED & !(VERSION_MASK << 1) | ((version & VERSION_MASK) << 1));
    after_length.push(section_number);
    after_length.push(section_number);
    after_length.extend_from_slice(payload);
    let mut v = short_section(table_id, &[&after_length[..], &[0; CRC_LEN]].concat());
    let end = v.len() - CRC_LEN;
    let crc = crc32_mpeg2(&v[..end]);
    v[end..].copy_from_slice(&crc.to_be_bytes());
    v
}

/// A PAT section mapping `(program_number, program_map_PID)` pairs.
#[must_use]
pub fn pat_section(tsid: u16, version: u8, entries: &[(u16, u16)]) -> Vec<u8> {
    let mut body = Vec::new();
    for &(pn, pid) in entries {
        body.extend_from_slice(&pn.to_be_bytes());
        body.push(PID_RESERVED | (pid >> 8) as u8 & PID_HI_MASK);
        body.push((pid & 0xFF) as u8);
    }
    long_section(PAT_TABLE_ID, tsid, version, 0, &body)
}

/// A PMT section with no program descriptors and no elementary streams.
#[must_use]
pub fn pmt_section_empty(program_number: u16, version: u8, pcr_pid: u16) -> Vec<u8> {
    long_section(
        PMT_TABLE_ID,
        program_number,
        version,
        0,
        &pmt_header(pcr_pid),
    )
}

/// A PMT section with one MPEG-2 video stream on `pcr_pid + 1`.
#[must_use]
pub fn pmt_section(program_number: u16, version: u8, pcr_pid: u16) -> Vec<u8> {
    let es_pid = pcr_pid + 1;
    let mut body = pmt_header(pcr_pid).to_vec();
    body.extend_from_slice(&[
        STREAM_TYPE_MPEG2_VIDEO,
        PID_RESERVED | (es_pid >> 8) as u8 & PID_HI_MASK,
        (es_pid & 0xFF) as u8,
        PROGRAM_INFO_LEN_HI,
        0x00,
    ]);
    long_section(PMT_TABLE_ID, program_number, version, 0, &body)
}

fn pmt_header(pcr_pid: u16) -> [u8; 4] {
    [
        PID_RESERVED | (pcr_pid >> 8) as u8 & PID_HI_MASK,
        (pcr_pid & 0xFF) as u8,
        PROGRAM_INFO_LEN_HI,
        0x00,
    ]
}

/// One TS packet with PUSI set on `pid`, continuity counter 0, `pointer_field` 0, the
/// section, then `0xFF` stuffing.
///
/// # Panics
/// If the section does not fit the one packet.
#[must_use]
pub fn ts_packet(pid: u16, section: &[u8]) -> [u8; TS_PACKET_SIZE] {
    assert!(
        PAYLOAD_START + section.len() <= TS_PACKET_SIZE,
        "section too big"
    );
    let mut pkt = [STUFFING; TS_PACKET_SIZE];
    pkt[0] = SYNC_BYTE;
    pkt[1] = PUSI | (pid >> 8) as u8 & PID_HI_MASK;
    pkt[2] = (pid & 0xFF) as u8;
    pkt[3] = PAYLOAD_ONLY;
    pkt[4] = 0x00;
    pkt[PAYLOAD_START..PAYLOAD_START + section.len()].copy_from_slice(section);
    pkt
}

/// A valid PAT (transport stream 1, program 1 on PID `0x0020`) in one TS packet on PID 0.
#[must_use]
pub fn pat_packet() -> [u8; TS_PACKET_SIZE] {
    ts_packet(PAT_PID, &pat_section(1, 0, &[(1, 0x0020)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_the_catalogue_check_value() {
        assert_eq!(crc32_mpeg2(b"123456789"), 0x0376_E6E7);
    }

    #[test]
    fn pat_packet_is_the_documented_literal() {
        let mut want = vec![
            0x00, 0xB0, 0x0D, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xE0, 0x20,
        ];
        want.extend_from_slice(&crc32_mpeg2(&want).to_be_bytes());
        let pkt = pat_packet();
        assert_eq!(&pkt[..5], [0x47, 0x40, 0x00, 0x10, 0x00]);
        assert_eq!(&pkt[5..5 + want.len()], want.as_slice());
        assert!(pkt[5 + want.len()..].iter().all(|b| *b == 0xFF));
    }

    #[test]
    fn short_section_carries_the_body_after_a_three_byte_header() {
        assert_eq!(short_section(0x42, &[1, 2, 3]), [0x42, 0xB0, 0x03, 1, 2, 3]);
    }
}
