use broadcast_common::crc32_mpeg2;

const PACKET: usize = 188;
const PUSI: u8 = 0x40;
const PID_MASK_HIGH: u8 = 0x1f;
const PAT_PID: u16 = 0;
const PAYLOAD_ONLY: u8 = 0x10;
const SECTION_HEADER: usize = 3;
const CRC_LEN: usize = 4;
const PAT_PROGRAM_OFFSET: usize = 8;
const PAT_ENTRY: usize = 4;
const PMT_FIXED: usize = 12;
const PMT_ES_FIXED: usize = 5;
const VERSION_BYTE: usize = 5;
const VERSION_BASE: u8 = 0xC1;
const VERSION_MASK: u8 = 0x1f;
const ES_PID_RESERVED: u8 = 0xE0;
const ES_INFO_RESERVED: u8 = 0xF0;
const FILL: u8 = 0xff;

#[derive(Debug, Clone)]
pub struct Es {
    pub stream_type: u8,
    pub pid: u16,
    pub descriptors: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Pmt {
    pub pid: u16,
    pub version: u8,
    pub es: Vec<Es>,
    head: Vec<u8>,
}

fn pid_of(p: &[u8]) -> u16 {
    (u16::from(p[1] & PID_MASK_HIGH) << 8) | u16::from(p[2])
}

fn section_of(p: &[u8]) -> &[u8] {
    &p[5 + usize::from(p[4])..]
}

fn section_len(s: &[u8]) -> usize {
    SECTION_HEADER + ((usize::from(s[1] & 0xf) << 8) | usize::from(s[2]))
}

pub fn find(bytes: &[u8]) -> Option<Pmt> {
    let pat = bytes
        .chunks_exact(PACKET)
        .find(|p| pid_of(p) == PAT_PID && p[1] & PUSI != 0)?;
    let s = section_of(pat);
    let end = section_len(s) - CRC_LEN;
    let mut i = PAT_PROGRAM_OFFSET;
    let mut pmt_pid = None;
    while i + PAT_ENTRY <= end {
        let program = (u16::from(s[i]) << 8) | u16::from(s[i + 1]);
        if program != 0 {
            pmt_pid = Some((u16::from(s[i + 2] & PID_MASK_HIGH) << 8) | u16::from(s[i + 3]));
            break;
        }
        i += PAT_ENTRY;
    }
    let pid = pmt_pid?;
    let packet = bytes
        .chunks_exact(PACKET)
        .find(|p| pid_of(p) == pid && p[1] & PUSI != 0)?;
    let s = section_of(packet);
    let len = section_len(s);
    if 5 + usize::from(packet[4]) + len > PACKET {
        return None;
    }
    let section = &s[..len];
    let info_len = (usize::from(section[10] & 0xf) << 8) | usize::from(section[11]);
    let head = section[..PMT_FIXED + info_len].to_vec();
    let mut es = Vec::new();
    let mut j = PMT_FIXED + info_len;
    while j + PMT_ES_FIXED <= len - CRC_LEN {
        let il = (usize::from(section[j + 3] & 0xf) << 8) | usize::from(section[j + 4]);
        es.push(Es {
            stream_type: section[j],
            pid: (u16::from(section[j + 1] & PID_MASK_HIGH) << 8) | u16::from(section[j + 2]),
            descriptors: section[j + PMT_ES_FIXED..j + PMT_ES_FIXED + il].to_vec(),
        });
        j += PMT_ES_FIXED + il;
    }
    Some(Pmt {
        pid,
        version: (section[VERSION_BYTE] >> 1) & VERSION_MASK,
        es,
        head,
    })
}

fn build(pmt: &Pmt, es: &[Es], version: u8, cc: u8) -> Vec<u8> {
    let mut sec = pmt.head.clone();
    sec[VERSION_BYTE] = VERSION_BASE | ((version & VERSION_MASK) << 1);
    for e in es {
        sec.push(e.stream_type);
        sec.push(ES_PID_RESERVED | (e.pid >> 8) as u8);
        sec.push(e.pid as u8);
        let il = e.descriptors.len();
        sec.push(ES_INFO_RESERVED | (il >> 8) as u8);
        sec.push(il as u8);
        sec.extend_from_slice(&e.descriptors);
    }
    let len = sec.len() - SECTION_HEADER + CRC_LEN;
    sec[1] = (sec[1] & 0xf0) | (len >> 8) as u8;
    sec[2] = len as u8;
    let crc = crc32_mpeg2::compute(&sec);
    sec.extend_from_slice(&crc.to_be_bytes());
    let mut p = vec![
        0x47,
        PUSI | (pmt.pid >> 8) as u8,
        pmt.pid as u8,
        PAYLOAD_ONLY | (cc & 0xf),
        0,
    ];
    p.extend_from_slice(&sec);
    p.resize(PACKET, FILL);
    p
}

pub fn rewrite(bytes: &[u8], pmt: &Pmt, plan: &dyn Fn(f64) -> Option<(Vec<Es>, u8)>) -> Vec<u8> {
    let n = bytes.len() / PACKET;
    let mut out = Vec::with_capacity(bytes.len());
    for (i, p) in bytes.chunks_exact(PACKET).enumerate() {
        if pid_of(p) == pmt.pid
            && p[1] & PUSI != 0
            && let Some((es, version)) = plan(i as f64 / n as f64)
        {
            out.extend(build(pmt, &es, version, p[3]));
            continue;
        }
        out.extend_from_slice(p);
    }
    out
}
