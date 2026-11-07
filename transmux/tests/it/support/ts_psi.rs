//! Byte-level TS packet and PSI walking for the transmux tests, deliberately
//! independent of the crate's own demuxer so it can act as an oracle.

pub const TS: usize = 188;

/// The 13-bit PID of a 188-byte packet.
pub fn pid_of(pkt: &[u8]) -> u16 {
    (((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16
}

/// The payload_unit_start_indicator of a packet.
pub fn pusi_of(pkt: &[u8]) -> bool {
    pkt[1] & 0x40 != 0
}

/// Offset of the payload in a packet (skips the 4-byte header and any
/// adaptation field); `TS` when the packet has no payload.
pub fn payload_offset(pkt: &[u8]) -> usize {
    let afc = (pkt[3] >> 4) & 0x3;
    let has_af = afc & 0b10 != 0;
    let has_payload = afc & 0b01 != 0;
    if !has_payload {
        return TS;
    }
    if has_af { 4 + 1 + pkt[4] as usize } else { 4 }
}

/// The first complete single-packet PSI section carried on `pid`, without the
/// pointer_field, trimmed to `section_length`.
pub fn first_section(ts: &[u8], pid: u16) -> Option<Vec<u8>> {
    for pkt in ts.chunks_exact(TS) {
        if pid_of(pkt) != pid || !pusi_of(pkt) {
            continue;
        }
        let off = payload_offset(pkt);
        if off >= TS {
            continue;
        }
        let payload = &pkt[off..];
        let ptr = payload[0] as usize;
        let sec_start = 1 + ptr;
        if sec_start + 3 > payload.len() {
            continue;
        }
        let sec = &payload[sec_start..];
        let section_length = (((sec[1] & 0x0F) as usize) << 8) | sec[2] as usize;
        let total = 3 + section_length;
        if total > sec.len() {
            continue;
        }
        return Some(sec[..total].to_vec());
    }
    None
}

/// `(program_number, program_map_PID)` pairs of a PAT section.
pub fn parse_pat(sec: &[u8]) -> Vec<(u16, u16)> {
    let body = &sec[8..sec.len() - 4];
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= body.len() {
        let prog = u16::from_be_bytes([body[i], body[i + 1]]);
        let pmt_pid = (((body[i + 2] & 0x1F) as u16) << 8) | body[i + 3] as u16;
        out.push((prog, pmt_pid));
        i += 4;
    }
    out
}

/// `(stream_type, elementary_PID)` pairs of a PMT section.
pub fn parse_pmt(sec: &[u8]) -> Vec<(u8, u16)> {
    let body = &sec[8..sec.len() - 4];
    let program_info_length = (((body[2] & 0x0F) as usize) << 8) | body[3] as usize;
    let mut i = 4 + program_info_length;
    let mut out = Vec::new();
    while i + 5 <= body.len() {
        let stream_type = body[i];
        let es_pid = (((body[i + 1] & 0x1F) as u16) << 8) | body[i + 2] as u16;
        let es_info_len = (((body[i + 3] & 0x0F) as usize) << 8) | body[i + 4] as usize;
        out.push((stream_type, es_pid));
        i += 5 + es_info_len;
    }
    out
}

/// The coded NAL payloads of 4-byte length-prefixed NAL data.
pub fn split_lp(lp: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut off = 0;
    while off + 4 <= lp.len() {
        let n = u32::from_be_bytes([lp[off], lp[off + 1], lp[off + 2], lp[off + 3]]) as usize;
        off += 4;
        if off + n > lp.len() {
            break;
        }
        out.push(lp[off..off + n].to_vec());
        off += n;
    }
    out
}
