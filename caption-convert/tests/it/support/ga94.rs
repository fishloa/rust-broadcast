//! Extract ATSC A/53 `GA94` `cc_data()` payloads from the H.264 video of a real
//! MPEG-2 TS, using the workspace crates (`mpeg-ts` for TS framing, `mpeg-pes`
//! for PES reassembly and PTS). The committed reference lists in
//! `fixtures/captions/**/*.cc_data.txt` were produced by an independent Python
//! extractor (`fixtures/captions/gen_cc_data_reference.py`); the tests demand the
//! two agree byte for byte.
//!
//! Spec: ITU-T H.264 §7.3.2.3 (SEI message framing), §7.4.1.2.3 (NAL unit
//! type 6), emulation prevention §7.4.1; ATSC A/53 Part 4 §6.2.3 / ANSI/SCTE
//! 128-1 (`itu_t_t35` country 0xB5, provider 0x0031, `GA94`,
//! `user_data_type_code` 0x03).
#![allow(dead_code)]

use mpeg_pes::{PesAssembler, PesPacket};
use mpeg_ts::TsPacket;

const TS_PACKET_SIZE: usize = 188;
const SEI_NAL_TYPE: u8 = 6;
const NAL_TYPE_MASK: u8 = 0x1F;
const SEI_PAYLOAD_T35: u32 = 4;
const T35_COUNTRY_US: u8 = 0xB5;
const T35_PROVIDER_ATSC: [u8; 2] = [0x00, 0x31];
const GA94: &[u8; 4] = b"GA94";
const USER_DATA_TYPE_CC: u8 = 0x03;
const RBSP_TRAILING_BITS: u8 = 0x80;
const EMULATION_PREVENTION: u8 = 0x03;
const VIDEO_STREAM_ID_MIN: u8 = 0xE0;
const VIDEO_STREAM_ID_MAX: u8 = 0xEF;

/// `(pts_90k, cc_data() bytes)` per `GA94` SEI, in PES order.
pub fn ga94_from_ts(ts: &[u8]) -> Vec<(u64, Vec<u8>)> {
    let video_pid = first_video_pid(ts).expect("a video PES stream in the capture");
    let mut asm = PesAssembler::new();
    let mut out = Vec::new();
    for chunk in ts.chunks_exact(TS_PACKET_SIZE) {
        let pkt = TsPacket::parse(chunk).expect("TS packet");
        if pkt.header.pid.value() != video_pid {
            continue;
        }
        let Some(payload) = pkt.payload else { continue };
        if let Some(done) = asm.feed(pkt.header.pusi, payload) {
            collect(&done, &mut out);
        }
    }
    if let Some(done) = asm.flush() {
        collect(&done, &mut out);
    }
    out
}

fn first_video_pid(ts: &[u8]) -> Option<u16> {
    for chunk in ts.chunks_exact(TS_PACKET_SIZE) {
        let pkt = TsPacket::parse(chunk).ok()?;
        let payload = pkt.payload?;
        if pkt.header.pusi
            && payload.len() > 3
            && payload[..3] == [0, 0, 1]
            && (VIDEO_STREAM_ID_MIN..=VIDEO_STREAM_ID_MAX).contains(&payload[3])
        {
            return Some(pkt.header.pid.value());
        }
    }
    None
}

fn collect(pes_bytes: &[u8], out: &mut Vec<(u64, Vec<u8>)>) {
    let pes = PesPacket::parse(pes_bytes).expect("PES packet");
    let pts = pes.header.as_ref().and_then(|h| h.pts).map_or(0, |p| p.0);
    for nal in annex_b_nals(pes.payload) {
        if nal.first().map(|b| b & NAL_TYPE_MASK) == Some(SEI_NAL_TYPE) {
            for cc in ga94_in_sei(&nal[1..]) {
                out.push((pts, cc));
            }
        }
    }
}

/// Split an Annex B byte stream on `00 00 01`, dropping trailing zero bytes.
pub fn annex_b_nals(es: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= es.len() {
        if es[i..i + 3] == [0, 0, 1] {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut nals = Vec::new();
    for (n, &s) in starts.iter().enumerate() {
        let e = starts.get(n + 1).map_or(es.len(), |next| next - 3);
        let mut nal = &es[s..e];
        while nal.last() == Some(&0) {
            nal = &nal[..nal.len() - 1];
        }
        nals.push(nal);
    }
    nals
}

fn unescape(rbsp_escaped: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rbsp_escaped.len());
    let mut zeros = 0;
    for &c in rbsp_escaped {
        if zeros >= 2 && c == EMULATION_PREVENTION {
            zeros = 0;
            continue;
        }
        out.push(c);
        zeros = if c == 0 { zeros + 1 } else { 0 };
    }
    out
}

fn read_var(b: &[u8], pos: &mut usize) -> u32 {
    let mut v = 0u32;
    while b[*pos] == 0xFF {
        v += 255;
        *pos += 1;
    }
    v += u32::from(b[*pos]);
    *pos += 1;
    v
}

/// The `cc_data()` bytes of every `GA94` `user_data_registered_itu_t_t35` SEI
/// message in one SEI NAL's escaped payload (NAL header byte already removed).
pub fn ga94_in_sei(escaped: &[u8]) -> Vec<Vec<u8>> {
    let rbsp = unescape(escaped);
    let mut pos = 0;
    let mut found = Vec::new();
    while pos < rbsp.len() && rbsp[pos] != RBSP_TRAILING_BITS {
        let payload_type = read_var(&rbsp, &mut pos);
        let size = read_var(&rbsp, &mut pos) as usize;
        let body = &rbsp[pos..pos + size];
        pos += size;
        if payload_type == SEI_PAYLOAD_T35
            && body.len() > 8
            && body[0] == T35_COUNTRY_US
            && body[1..3] == T35_PROVIDER_ATSC
            && &body[3..7] == GA94
            && body[7] == USER_DATA_TYPE_CC
        {
            found.push(body[8..].to_vec());
        }
    }
    found
}

/// Parse a `<pts_90k> <hex>` reference file (`#` comments allowed).
pub fn parse_reference(text: &str) -> Vec<(u64, Vec<u8>)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let (pts, hex) = l.split_once(' ').expect("`<pts> <hex>` line");
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
                .collect();
            (pts.parse().expect("pts"), bytes)
        })
        .collect()
}
