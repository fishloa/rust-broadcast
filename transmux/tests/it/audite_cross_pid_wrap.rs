//! Bug audit E: audio and video PIDs of ONE program must share a 33-bit
//! unroll epoch. `WrapState` is per PID and seeds each from its own first raw
//! stamp, so audio that starts just before the 2^33 wrap and video that
//! starts just after it land ~26.5 hours apart in the IR.

#![cfg(feature = "std")]

use broadcast_common::Unpackage;
use transmux::TsDemux;
use transmux::pipeline::CodecConfig;

const TS_WRAP: i64 = 1 << 33;
/// 90 kHz / 30 fps.
const FRAME_DUR: i64 = 3000;

const TS_PACKET_SIZE: usize = 188;
const PAT_PID: u16 = 0x0000;
const PMT_PID: u16 = 0x1000;
#[allow(dead_code)]
const ES_PID: u16 = 0x0100;
const STREAM_TYPE_AVC: u8 = 0x1B;

/// A plausible Baseline SPS + PPS, so `TsDemux`'s single-shot config recovery
/// resolves an `avcC` for the synthesised elementary stream.
const SPS: [u8; 9] = [0x67, 0x42, 0xc0, 0x1e, 0xd9, 0x00, 0x80, 0x1e, 0x24];
const PPS: [u8; 4] = [0x68, 0xce, 0x3c, 0x80];

/// Encode a 33-bit PTS/DTS into its 5-byte PES field (§2.4.3.7): a 4-bit
/// `prefix`, then the value split 3 / 15 / 15 bits with marker bits set.
fn encode_pes_ts(value: i64, prefix: u8) -> [u8; 5] {
    broadcast_common::clock33::pack_marked(value as u64, prefix)
}

/// One Annex B access unit: parameter sets on the first AU (so config
/// resolves), then an IDR slice NAL tagged with `tag` so samples are
/// distinguishable.
fn annexb_au(with_param_sets: bool, tag: u8) -> Vec<u8> {
    let mut au = Vec::new();
    let mut push = |nal: &[u8]| {
        au.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        au.extend_from_slice(nal);
    };
    if with_param_sets {
        push(&SPS);
        push(&PPS);
    }
    push(&[0x65, 0x88, 0x84, tag]);
    au
}

/// Wrap an access unit in a PES packet carrying both PTS and DTS (§2.4.3.6).
fn pes_packet(au: &[u8], pts: i64, dts: i64) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x00, 0x00, 0x01, 0xE0]); // start code + video stream_id
    let header_data_len = 10u8; // PTS(5) + DTS(5)
    let payload_len = au.len() + 3 + header_data_len as usize;
    out.extend_from_slice(&(payload_len as u16).to_be_bytes());
    out.push(0x80); // '10' marker, no scrambling
    out.push(0xC0); // PTS_DTS_flags = '11'
    out.push(header_data_len);
    out.extend_from_slice(&encode_pes_ts(pts, 0b0011));
    out.extend_from_slice(&encode_pes_ts(dts, 0b0001));
    out.extend_from_slice(au);
    out
}

/// Packetise a payload into 188-byte TS packets on `pid`, PUSI on the first,
/// stuffing the tail of the final packet via an adaptation field.
fn packetise(pid: u16, payload: &[u8], cc: &mut u8, out: &mut Vec<u8>) {
    let mut first = true;
    let mut off = 0usize;
    while off < payload.len() {
        let remaining = payload.len() - off;
        let mut pkt = Vec::with_capacity(TS_PACKET_SIZE);
        pkt.push(0x47);
        let pusi = if first { 0x40 } else { 0x00 };
        pkt.push(pusi | ((pid >> 8) as u8 & 0x1F));
        pkt.push((pid & 0xFF) as u8);
        let body_cap = TS_PACKET_SIZE - 4;
        if remaining >= body_cap {
            pkt.push(0x10 | (*cc & 0x0F)); // payload only
            pkt.extend_from_slice(&payload[off..off + body_cap]);
            off += body_cap;
        } else {
            // Adaptation field carrying stuffing so the packet stays 188 bytes.
            pkt.push(0x30 | (*cc & 0x0F));
            let af_len = body_cap - 1 - remaining;
            pkt.push(af_len as u8);
            if af_len > 0 {
                pkt.push(0x00); // flags
                pkt.extend(core::iter::repeat_n(0xFF, af_len - 1));
            }
            pkt.extend_from_slice(&payload[off..]);
            off = payload.len();
        }
        assert_eq!(pkt.len(), TS_PACKET_SIZE, "TS packet must be 188 bytes");
        *cc = cc.wrapping_add(1);
        out.extend_from_slice(&pkt);
        first = false;
    }
}

/// A long-form PSI section (§2.4.4.1): header + body + trailing CRC-32.
fn psi_section(table_id: u8, id_extension: u16, body: &[u8]) -> Vec<u8> {
    let mut s = Vec::new();
    s.push(table_id);
    let section_length = body.len() + 5 + 4; // the 5 bytes below + CRC
    s.push(0xB0 | ((section_length >> 8) as u8 & 0x0F));
    s.push((section_length & 0xFF) as u8);
    s.extend_from_slice(&id_extension.to_be_bytes());
    s.push(0xC1); // version 0, current_next = 1
    s.push(0x00); // section_number
    s.push(0x00); // last_section_number
    s.extend_from_slice(body);
    let crc = broadcast_common::crc32_mpeg2::compute(&s);
    s.extend_from_slice(&crc.to_be_bytes());
    // PSI packets carry a pointer_field before the section.
    let mut with_pointer = alloc_vec_with_pointer();
    with_pointer.extend_from_slice(&s);
    with_pointer
}

fn alloc_vec_with_pointer() -> Vec<u8> {
    vec![0x00]
}


const VID_PID: u16 = 0x0100;
const AUD_PID: u16 = 0x0101;
const STREAM_TYPE_AAC_ADTS: u8 = 0x0F;
const AAC_FRAME_TICKS: i64 = 1920;
const AAC_RATE: i64 = 48_000;

fn adts_frame() -> Vec<u8> {
    let len = 27usize;
    let mut f = vec![
        0xFF,
        0xF1,
        0x4C,
        0x80 | ((len >> 11) as u8 & 0x03),
        ((len >> 3) & 0xFF) as u8,
        (((len & 7) << 5) as u8) | 0x1F,
        0xFC,
    ];
    f.extend(core::iter::repeat_n(0x21, len - 7));
    f
}

fn audio_pes(frame: &[u8], pts: i64) -> Vec<u8> {
    let mut out = vec![0x00, 0x00, 0x01, 0xC0];
    let payload_len = frame.len() + 3 + 5;
    out.extend_from_slice(&(payload_len as u16).to_be_bytes());
    out.push(0x80);
    out.push(0x80);
    out.push(5);
    out.extend_from_slice(&encode_pes_ts(pts, 0b0010));
    out.extend_from_slice(frame);
    out
}

fn synth_two_pid_ts(video_first: i64, audio_first: i64) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pat_body = Vec::new();
    pat_body.extend_from_slice(&1u16.to_be_bytes());
    pat_body.extend_from_slice(&(0xE000 | PMT_PID).to_be_bytes());
    let mut cc = 0u8;
    packetise(PAT_PID, &psi_section(0x00, 1, &pat_body), &mut cc, &mut out);

    let mut pmt_body = Vec::new();
    pmt_body.extend_from_slice(&(0xE000 | VID_PID).to_be_bytes());
    pmt_body.extend_from_slice(&0xF000u16.to_be_bytes());
    for (st, pid) in [(STREAM_TYPE_AVC, VID_PID), (STREAM_TYPE_AAC_ADTS, AUD_PID)] {
        pmt_body.push(st);
        pmt_body.extend_from_slice(&(0xE000 | pid).to_be_bytes());
        pmt_body.extend_from_slice(&0xF000u16.to_be_bytes());
    }
    let mut cc = 0u8;
    packetise(PMT_PID, &psi_section(0x02, 1, &pmt_body), &mut cc, &mut out);

    let (mut cc_v, mut cc_a) = (0u8, 0u8);
    let wrap = TS_WRAP;
    for i in 0..6i64 {
        let dts = (video_first + i * FRAME_DUR).rem_euclid(wrap);
        let au = annexb_au(i == 0, i as u8);
        packetise(VID_PID, &pes_packet(&au, dts, dts), &mut cc_v, &mut out);
        for j in 0..2i64 {
            let n = i * 2 + j;
            let pts = (audio_first + n * AAC_FRAME_TICKS).rem_euclid(wrap);
            packetise(AUD_PID, &audio_pes(&adts_frame(), pts), &mut cc_a, &mut out);
        }
    }
    out
}

#[test]
fn audio_and_video_of_one_program_share_one_unroll_epoch_across_2_33() {
    let ts = synth_two_pid_ts(3000, TS_WRAP - 1500);
    let media = TsDemux::new().unpackage(&ts).expect("demux");
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("avc track");
    let audio = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("aac track");
    let v0 = video.samples[0].dts.expect("video dts");
    let a0 = audio.samples[0].dts.expect("audio dts");
    let a0_90k = a0 * 90_000 / AAC_RATE;
    let skew = (a0_90k - v0).abs();
    assert!(
        skew < 90_000,
        "audio starts 4500 ticks before video on the wire (across the 2^33 wrap) but the IR puts them {skew} ticks apart (audio dts {a0_90k}, video dts {v0})"
    );
}
