pub const PACKET: usize = 188;
pub const SYNC_BYTE: u8 = 0x47;
pub const PTS_WRAP: i64 = 1 << 33;
pub const PCR_HZ: u64 = 27_000_000;
pub const VIDEO_TIMESCALE: u32 = 90_000;
const VIDEO_STREAM_IDS: std::ops::RangeInclusive<u8> = 0xE0..=0xEF;
const AUDIO_STREAM_IDS: std::ops::RangeInclusive<u8> = 0xC0..=0xDF;
const PRIVATE_STREAM_1: u8 = 0xBD;
const AF_PCR_FLAG: u8 = 0x10;
const AF_PRESENT: u8 = 0x20;
pub const AF_DISCONTINUITY_FLAG: u8 = 0x80;

pub fn pts27(pts: i64, timescale: u32) -> u64 {
    u64::try_from(i128::from(pts) * i128::from(PCR_HZ) / i128::from(timescale)).unwrap()
}

pub fn packet_pid(p: &[u8]) -> u16 {
    (u16::from(p[1] & 0x1f) << 8) | u16::from(p[2])
}

pub fn packet_has_pcr(p: &[u8]) -> bool {
    p[3] & AF_PRESENT != 0 && p[4] > 0 && p[5] & AF_PCR_FLAG != 0
}

fn pes_header_offset(p: &[u8]) -> Option<usize> {
    if p[0] != SYNC_BYTE || p[1] & 0x40 == 0 {
        return None;
    }
    let afc = (p[3] >> 4) & 3;
    if afc & 1 == 0 {
        return None;
    }
    let off = 4 + if afc & 2 != 0 {
        1 + usize::from(p[4])
    } else {
        0
    };
    (off + 14 <= PACKET && p[off..off + 3] == [0, 0, 1]).then_some(off)
}

fn read_ts(b: &[u8]) -> i64 {
    (i64::from(b[0] >> 1) & 7) << 30
        | i64::from(b[1]) << 22
        | (i64::from(b[2]) >> 1) << 15
        | i64::from(b[3]) << 7
        | i64::from(b[4]) >> 1
}

fn write_ts(b: &mut [u8], v: i64) {
    let prefix = b[0] & 0xF0;
    b[0] = prefix | ((((v >> 30) & 7) as u8) << 1) | 1;
    b[1] = (v >> 22) as u8;
    b[2] = ((((v >> 15) & 0x7f) as u8) << 1) | 1;
    b[3] = (v >> 7) as u8;
    b[4] = (((v & 0x7f) as u8) << 1) | 1;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Shift {
    Video,
    Audio,
    Both,
}

fn stream_kind(p: &[u8], off: usize) -> Option<Shift> {
    let id = p[off + 3];
    if VIDEO_STREAM_IDS.contains(&id) {
        Some(Shift::Video)
    } else if AUDIO_STREAM_IDS.contains(&id) || id == PRIVATE_STREAM_1 {
        Some(Shift::Audio)
    } else {
        None
    }
}

pub fn pes_starts(bytes: &[u8], kind: Shift) -> Vec<usize> {
    bytes
        .chunks(PACKET)
        .enumerate()
        .filter(|(_, p)| pes_header_offset(p).is_some_and(|o| stream_kind(p, o) == Some(kind)))
        .map(|(i, _)| i)
        .collect()
}

pub fn shift_pes(bytes: &[u8], from_packet: usize, delta_ticks: i64, which: Shift) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for (i, p) in out.chunks_mut(PACKET).enumerate() {
        if i < from_packet {
            continue;
        }
        let Some(off) = pes_header_offset(p) else {
            continue;
        };
        let Some(kind) = stream_kind(p, off) else {
            continue;
        };
        if which != Shift::Both && which != kind {
            continue;
        }
        let flags = p[off + 7];
        let hdr = off + 9;
        if flags & 0x80 != 0 {
            let v = (read_ts(&p[hdr..hdr + 5]) + delta_ticks).rem_euclid(PTS_WRAP);
            write_ts(&mut p[hdr..hdr + 5], v);
        }
        if flags & 0x40 != 0 {
            let v = (read_ts(&p[hdr + 5..hdr + 10]) + delta_ticks).rem_euclid(PTS_WRAP);
            write_ts(&mut p[hdr + 5..hdr + 10], v);
        }
    }
    out
}
