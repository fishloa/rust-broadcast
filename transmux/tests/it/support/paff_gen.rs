// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Alex Fishlock
//
//! paff-gen — spec-built H.264 PAFF test-stream generator.
//!
//! Writes a tiny, losslessly decodable H.264 stream in which every picture
//! is a FIELD (`field_pic_flag = 1`, PAFF): SPS with `frame_mbs_only_flag = 0`,
//! `mb_adaptive_frame_field_flag = 0`; every macroblock is `I_PCM`
//! (raw samples, ITU-T H.264 §7.3.5 / §7.4.5), deblocking disabled, so the
//! decoded output equals the input pattern bit for bit. Each field carries a
//! different moving pattern, so a decoder or demuxer that pairs fields wrongly
//! produces visibly (and measurably) wrong frames.
//!
//! Outputs (written next to `<out>`):
//!   <out>.h264  Annex B elementary stream (AUD + [SPS + PPS] + one slice per field)
//!   <out>.ts    MPEG-2 TS, one PES packet per FIELD (as broadcasters send PAFF),
//!               PTS per field (frame period 3600, field offset 1800 at 90 kHz)
//!   <out>.yuv   expected decoder output: woven frames, yuv420p, decode order
//!
//! Written from the specifications only (ITU-T H.264 (08/2021) §7.3, §7.4,
//! §8.3.5, Annex B; ISO/IEC 13818-1 §2.4.3, §2.4.4); no code from any other
//! encoder or muxer.
//!
//! Vendored into transmux tests (issue #1161) from the zenith-tv `paff-gen` tool
//! (same author, same licence); `main()` replaced by [`generate`].

#![allow(dead_code, clippy::all)]

// ---------------------------------------------------------------- bit writer

struct Bits {
    out: Vec<u8>,
    cur: u8,
    n: u8,
}

impl Bits {
    fn new() -> Self {
        Bits {
            out: Vec::new(),
            cur: 0,
            n: 0,
        }
    }
    fn u(&mut self, bits: u32, v: u32) {
        for i in (0..bits).rev() {
            self.cur = (self.cur << 1) | ((v >> i) & 1) as u8;
            self.n += 1;
            if self.n == 8 {
                self.out.push(self.cur);
                self.cur = 0;
                self.n = 0;
            }
        }
    }
    /// ue(v), §9.1
    fn ue(&mut self, v: u32) {
        let x = v + 1;
        let len = 32 - x.leading_zeros();
        self.u(len - 1, 0);
        self.u(len, x);
    }
    /// se(v), §9.1.1
    fn se(&mut self, v: i32) {
        self.ue(if v > 0 {
            (2 * v - 1) as u32
        } else {
            (-2 * v) as u32
        });
    }
    fn aligned(&self) -> bool {
        self.n == 0
    }
    fn align_zero(&mut self) {
        while self.n != 0 {
            self.u(1, 0);
        }
    }
    /// rbsp_trailing_bits(), §7.3.2.11
    fn trailing(mut self) -> Vec<u8> {
        self.u(1, 1);
        self.align_zero();
        self.out
    }
}

/// NAL unit with emulation prevention (§7.3.1, §7.4.1) and a 4-byte start code (Annex B).
fn nal(ref_idc: u8, nal_type: u8, rbsp: &[u8]) -> Vec<u8> {
    let mut v = vec![0, 0, 0, 1, (ref_idc << 5) | nal_type];
    let mut zeros = 0;
    for &b in rbsp {
        if zeros >= 2 && b <= 3 {
            v.push(3);
            zeros = 0;
        }
        v.push(b);
        zeros = if b == 0 { zeros + 1 } else { 0 };
    }
    v
}

// ---------------------------------------------------------------- H.264 syntax

const LOG2_MAX_FRAME_NUM: u32 = 4;

/// seq_parameter_set_rbsp(), §7.3.2.1.1. Main profile (allows interlace), level 3.0.
fn sps(w: usize, h: usize) -> Vec<u8> {
    let mut b = Bits::new();
    b.u(8, 77); // profile_idc = Main
    b.u(8, 0); // constraint_set flags + reserved_zero_2bits
    b.u(8, 30); // level_idc
    b.ue(0); // seq_parameter_set_id
    b.ue(LOG2_MAX_FRAME_NUM - 4); // log2_max_frame_num_minus4
    b.ue(2); // pic_order_cnt_type = 2 (output order = decode order)
    b.ue(1); // max_num_ref_frames
    b.u(1, 0); // gaps_in_frame_num_value_allowed_flag
    b.ue((w / 16 - 1) as u32); // pic_width_in_mbs_minus1
    b.ue((h / 32 - 1) as u32); // pic_height_in_map_units_minus1 (field MB rows)
    b.u(1, 0); // frame_mbs_only_flag = 0 -> field coding allowed
    b.u(1, 0); // mb_adaptive_frame_field_flag = 0 -> PAFF only, no MBAFF
    b.u(1, 1); // direct_8x8_inference_flag (shall be 1 when frame_mbs_only_flag = 0)
    b.u(1, 0); // frame_cropping_flag
    b.u(1, 0); // vui_parameters_present_flag
    b.trailing()
}

/// pic_parameter_set_rbsp(), §7.3.2.2. CAVLC, deblocking control present.
fn pps() -> Vec<u8> {
    let mut b = Bits::new();
    b.ue(0); // pic_parameter_set_id
    b.ue(0); // seq_parameter_set_id
    b.u(1, 0); // entropy_coding_mode_flag = CAVLC
    b.u(1, 0); // bottom_field_pic_order_in_frame_present_flag
    b.ue(0); // num_slice_groups_minus1
    b.ue(0); // num_ref_idx_l0_default_active_minus1
    b.ue(0); // num_ref_idx_l1_default_active_minus1
    b.u(1, 0); // weighted_pred_flag
    b.u(2, 0); // weighted_bipred_idc
    b.se(0); // pic_init_qp_minus26
    b.se(0); // pic_init_qs_minus26
    b.se(0); // chroma_qp_index_offset
    b.u(1, 1); // deblocking_filter_control_present_flag
    b.u(1, 0); // constrained_intra_pred_flag
    b.u(1, 0); // redundant_pic_cnt_present_flag
    b.trailing()
}

/// access_unit_delimiter_rbsp(), §7.3.2.4: primary_pic_type 0 = I slices only.
fn aud() -> Vec<u8> {
    let mut b = Bits::new();
    b.u(3, 0);
    b.trailing()
}

/// One field picture = one I slice of I_PCM macroblocks (§7.3.3, §7.3.4, §7.3.5).
/// `field` holds the field's samples: luma w x h/2, then Cb, Cr (w/2 x h/4 each).
fn slice(idr: bool, frame_num: u32, bottom: bool, w: usize, fh: usize, field: &Field) -> Vec<u8> {
    let mut b = Bits::new();
    b.ue(0); // first_mb_in_slice
    b.ue(7); // slice_type = I (all slices of the picture are I)
    b.ue(0); // pic_parameter_set_id
    b.u(LOG2_MAX_FRAME_NUM, frame_num); // frame_num
    b.u(1, 1); // field_pic_flag
    b.u(1, bottom as u32); // bottom_field_flag
    if idr {
        b.ue(0); // idr_pic_id
    }
    // pic_order_cnt_type 2: no POC syntax. nal_ref_idc != 0 -> dec_ref_pic_marking():
    if idr {
        b.u(1, 0); // no_output_of_prior_pics_flag
        b.u(1, 0); // long_term_reference_flag
    } else {
        b.u(1, 0); // adaptive_ref_pic_marking_mode_flag (sliding window)
    }
    b.se(0); // slice_qp_delta
    b.ue(1); // disable_deblocking_filter_idc = 1 (off): output == I_PCM input
    // slice_data(): CAVLC I slice, no skip runs.
    let (mbw, mbh) = (w / 16, fh / 16);
    for my in 0..mbh {
        for mx in 0..mbw {
            b.ue(25); // mb_type = I_PCM (Table 7-11)
            b.align_zero(); // pcm_alignment_zero_bit
            assert!(b.aligned());
            for y in 0..16 {
                for x in 0..16 {
                    b.u(8, field.y[(my * 16 + y) * w + mx * 16 + x] as u32);
                }
            }
            for plane in [&field.cb, &field.cr] {
                for y in 0..8 {
                    for x in 0..8 {
                        b.u(8, plane[(my * 8 + y) * (w / 2) + mx * 8 + x] as u32);
                    }
                }
            }
        }
    }
    b.trailing()
}

// ---------------------------------------------------------------- test pattern

struct Field {
    y: Vec<u8>,
    cb: Vec<u8>,
    cr: Vec<u8>,
}

/// Field number `t` (0, 1, 2, … in output order) of a w x (h/2) field. A
/// diagonal ramp moving 4 px per field plus a per-field chroma tint, so each
/// field differs from every other. Samples stay in 16..=235 (never 0).
fn make_field(t: usize, w: usize, fh: usize) -> Field {
    let mut y = vec![0u8; w * fh];
    for r in 0..fh {
        for c in 0..w {
            y[r * w + c] = (16 + ((c + 2 * r + 4 * t) % 64) * 3) as u8;
        }
    }
    let n = (w / 2) * (fh / 2);
    Field {
        y,
        cb: vec![(64 + 8 * (t % 16)) as u8; n],
        cr: vec![(192 - 8 * (t % 16)) as u8; n],
    }
}

/// Weave a top and bottom field into a yuv420p frame (lines alternate T,B;
/// chroma lines alternate the same way, §6.2).
fn weave(top: &Field, bot: &Field, w: usize, h: usize, out: &mut Vec<u8>) {
    for r in 0..h {
        let f = if r % 2 == 0 { top } else { bot };
        out.extend_from_slice(&f.y[(r / 2) * w..(r / 2) * w + w]);
    }
    for plane in 0..2 {
        for r in 0..h / 2 {
            let f = if r % 2 == 0 { top } else { bot };
            let p = if plane == 0 { &f.cb } else { &f.cr };
            out.extend_from_slice(&p[(r / 2) * (w / 2)..(r / 2) * (w / 2) + w / 2]);
        }
    }
}

// ---------------------------------------------------------------- MPEG-2 TS

pub const PID_PMT: u16 = 0x1000;
const PID_VIDEO: u16 = 0x0100;

/// CRC-32/MPEG-2 (ISO/IEC 13818-1 Annex A) - the workspace's one implementation.
fn crc32_mpeg2(d: &[u8]) -> u32 {
    broadcast_common::crc32_mpeg2::compute(d)
}

/// Wrap a complete PSI section (with CRC) in one TS packet (pointer_field 0).
pub fn psi_packet(pid: u16, cc: &mut u8, section: &[u8]) -> [u8; 188] {
    let mut p = [0xFFu8; 188];
    p[0] = 0x47;
    p[1] = 0x40 | (pid >> 8) as u8; // payload_unit_start_indicator
    p[2] = pid as u8;
    p[3] = 0x10 | (*cc & 0xF); // payload only
    *cc = cc.wrapping_add(1);
    p[4] = 0; // pointer_field
    p[5..5 + section.len()].copy_from_slice(section);
    p
}

fn section(table_id: u8, ext_id: u16, body: &[u8]) -> Vec<u8> {
    let len = 5 + body.len() + 4; // ext fields + body + CRC
    let mut s = vec![
        table_id,
        0xB0 | ((len >> 8) as u8 & 0x0F), // section_syntax_indicator=1, '0', reserved '11'
        len as u8,
        (ext_id >> 8) as u8,
        ext_id as u8,
        0xC1, // reserved '11', version 0, current_next_indicator 1
        0,    // section_number
        0,    // last_section_number
    ];
    s.extend_from_slice(body);
    let crc = crc32_mpeg2(&s);
    s.extend_from_slice(&crc.to_be_bytes());
    s
}

pub fn pat() -> Vec<u8> {
    // program_number 1 -> PMT PID
    section(
        0x00,
        1,
        &[0x00, 0x01, 0xE0 | (PID_PMT >> 8) as u8, PID_PMT as u8],
    )
}

pub fn pmt() -> Vec<u8> {
    section(
        0x02,
        1,
        &[
            0xE0 | (PID_VIDEO >> 8) as u8,
            PID_VIDEO as u8, // PCR_PID
            0xF0,
            0x00, // program_info_length 0
            0x1B, // stream_type H.264
            0xE0 | (PID_VIDEO >> 8) as u8,
            PID_VIDEO as u8,
            0xF0,
            0x00, // ES_info_length 0
        ],
    )
}

fn pts_bytes(marker: u8, ts: u64) -> [u8; 5] {
    broadcast_common::clock33::pack_marked(ts, marker)
}

/// One PES packet (PTS only, PES_packet_length 0 = unbounded video) split into
/// TS packets; the first carries a PCR. Short tails are padded with
/// adaptation-field stuffing (§2.4.3.5).
pub fn pes_packets(pts: u64, es: &[u8], cc: &mut u8, out: &mut Vec<u8>) {
    let mut pes = vec![0, 0, 1, 0xE0, 0, 0, 0x80, 0x80, 5];
    pes.extend_from_slice(&pts_bytes(0b0010, pts));
    pes.extend_from_slice(es);
    let mut off = 0;
    let mut first = true;
    while off < pes.len() {
        let mut af: Vec<u8> = Vec::new();
        if first {
            let pcr_base = pts.saturating_sub(9000); // 100 ms ahead of the first PTS
            af = vec![
                0x10, // PCR_flag
                (pcr_base >> 25) as u8,
                (pcr_base >> 17) as u8,
                (pcr_base >> 9) as u8,
                (pcr_base >> 1) as u8,
                (((pcr_base & 1) as u8) << 7) | 0x7E, // reserved '111111', ext high bit 0
                0x00,
            ];
        }
        let room = 184 - if af.is_empty() { 0 } else { 1 + af.len() };
        let take = room.min(pes.len() - off);
        if take < room {
            // pad: grow (or create) the adaptation field so payload fills the packet
            let pad = room - take;
            if af.is_empty() {
                af = if pad == 1 { vec![] } else { vec![0x00] };
                af.extend(std::iter::repeat(0xFF).take(pad.saturating_sub(2)));
                if pad == 1 {
                    // a 1-byte adaptation field: just adaptation_field_length = 0
                }
            } else {
                af.extend(std::iter::repeat(0xFF).take(pad));
            }
        }
        let mut p = Vec::with_capacity(188);
        p.push(0x47);
        p.push(if first { 0x40 } else { 0 } | (PID_VIDEO >> 8) as u8);
        p.push(PID_VIDEO as u8);
        let has_af = !af.is_empty() || (take < room && room - take == 1);
        p.push(if has_af { 0x30 } else { 0x10 } | (*cc & 0xF));
        *cc = cc.wrapping_add(1);
        if has_af {
            p.push(af.len() as u8);
            p.extend_from_slice(&af);
        }
        p.extend_from_slice(&pes[off..off + take]);
        assert_eq!(p.len(), 188, "TS packet size");
        out.extend_from_slice(&p);
        off += take;
        first = false;
    }
}

// ---------------------------------------------------------------- entry

/// A generated PAFF stream: Annex B ES, TS (one PES per field), expected woven yuv420p.
pub struct Generated {
    pub es: Vec<u8>,
    pub ts: Vec<u8>,
    pub yuv: Vec<u8>,
    /// Per-field access units (PTS, Annex B bytes) in decode order.
    pub aus: Vec<(u64, Vec<u8>)>,
}

/// `frames` frames of `w` x `h` (W % 16 == 0, H % 32 == 0), `bff` = bottom field first.
pub fn generate(frames: usize, w: usize, h: usize, bff: bool) -> Generated {
    assert!(
        w % 16 == 0 && h % 32 == 0 && frames >= 1,
        "W%16, H%32, frames>=1"
    );
    let fh = h / 2;
    let (sps, pps) = (sps(w, h), pps());
    let mut es = Vec::new();
    let mut ts = Vec::new();
    let mut yuv = Vec::new();
    let mut aus = Vec::new();
    let (mut cc_pat, mut cc_pmt, mut cc_v) = (0u8, 0u8, 0u8);
    let base_pts = 90_000u64; // 1 s

    for k in 0..frames {
        if k % 10 == 0 {
            ts.extend_from_slice(&psi_packet(0, &mut cc_pat, &pat()));
            ts.extend_from_slice(&psi_packet(PID_PMT, &mut cc_pmt, &pmt()));
        }
        // field numbers in output order: first field 2k, second 2k+1
        let first = make_field(2 * k, w, fh);
        let second = make_field(2 * k + 1, w, fh);
        let (top, bot) = if bff {
            (&second, &first)
        } else {
            (&first, &second)
        };
        weave(top, bot, w, h, &mut yuv);
        let frame_num = (k as u32) % (1 << LOG2_MAX_FRAME_NUM);
        for (i, f) in [&first, &second].into_iter().enumerate() {
            let bottom = (i == 0) == bff; // first field is bottom iff BFF
            let idr = k == 0 && i == 0;
            let mut au = nal(0, 9, &aud());
            if k == 0 && i == 0 {
                au.extend(nal(3, 7, &sps));
                au.extend(nal(3, 8, &pps));
            }
            au.extend(nal(
                1,
                if idr { 5 } else { 1 },
                &slice(idr, frame_num, bottom, w, fh, f),
            ));
            es.extend_from_slice(&au);
            aus.push((base_pts + 3600 * k as u64 + 1800 * i as u64, au.clone()));
            pes_packets(
                base_pts + 3600 * k as u64 + 1800 * i as u64,
                &au,
                &mut cc_v,
                &mut ts,
            );
        }
    }
    Generated { es, ts, yuv, aus }
}

/// Re-wrap (PTS, AU) pairs as a TS stream, one PES per AU (PAT/PMT up front) —
/// for tests that drop or retime fields.
pub fn ts_from_aus(aus: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let (mut cc_pat, mut cc_pmt, mut cc_v) = (0u8, 0u8, 0u8);
    let mut ts = Vec::new();
    ts.extend_from_slice(&psi_packet(0, &mut cc_pat, &pat()));
    ts.extend_from_slice(&psi_packet(PID_PMT, &mut cc_pmt, &pmt()));
    for (pts, au) in aus {
        pes_packets(*pts, au, &mut cc_v, &mut ts);
    }
    ts
}
