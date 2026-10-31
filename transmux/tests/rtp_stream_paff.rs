//! Field-coded (PAFF) H.264 over RTP: each field is its own access unit with
//! its own RTP timestamp, while `max_num_reorder_frames` counts frames, field
//! pairs and non-paired fields (ITU-T H.264 E.2.1), so the derived decode
//! timestamps must use a depth widened for field pictures
//! (`private/specs-verified/mpeg/transmux/codec/h264-slice-header.md`).
#![cfg(feature = "std")]

use transmux::TrackId;
use transmux::pipeline::CodecConfig;
use transmux::rtp::RtpMediaKind;
use transmux::{
    AVCConfigurationBox, AVCDecoderConfigurationRecord, AvcPps, AvcSps, RtpLossEvent,
    RtpStreamDepacketiser, RtpStreamTrack,
};

const FRAME_PERIOD: u32 = 3600;
const FIELD_OFFSET: u32 = FRAME_PERIOD / 2;
const MAX_NUM_REORDER_FRAMES: u32 = 1;
const NAL_SLICE: u8 = 1;
const NAL_IDR: u8 = 5;
const NAL_SPS: u8 = 7;
const NAL_PPS: u8 = 8;
const REF_IDC: u8 = 3;
const FRAME_ORDER: [u32; 13] = [0, 2, 1, 4, 3, 6, 5, 8, 7, 10, 9, 12, 11];

#[derive(Default)]
struct W {
    out: Vec<u8>,
    cur: u8,
    n: u8,
}

impl W {
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
    fn ue(&mut self, v: u32) {
        let x = v + 1;
        let len = 32 - x.leading_zeros();
        self.u(len - 1, 0);
        self.u(len, x);
    }
    fn finish(mut self) -> Vec<u8> {
        self.u(1, 1);
        while self.n != 0 {
            self.u(1, 0);
        }
        self.out
    }
}

fn nal(t: u8, rbsp: Vec<u8>) -> Vec<u8> {
    let mut v = vec![(REF_IDC << 5) | t];
    v.extend(rbsp);
    v
}

fn sps() -> Vec<u8> {
    let mut w = W::default();
    w.u(8, 77);
    w.u(8, 0);
    w.u(8, 30);
    w.ue(0);
    w.ue(0);
    w.ue(0);
    w.ue(2);
    w.ue(1);
    w.u(1, 0);
    w.ue(3);
    w.ue(1);
    w.u(1, 0);
    w.u(1, 0);
    w.u(1, 1);
    w.u(1, 0);
    w.u(1, 1);
    for _ in 0..8 {
        w.u(1, 0);
    }
    w.u(1, 1);
    w.u(1, 1);
    w.ue(0);
    w.ue(0);
    w.ue(0);
    w.ue(0);
    w.ue(MAX_NUM_REORDER_FRAMES);
    w.ue(MAX_NUM_REORDER_FRAMES + 1);
    nal(NAL_SPS, w.finish())
}

fn pps() -> Vec<u8> {
    let mut w = W::default();
    w.ue(0);
    w.ue(0);
    w.u(1, 0);
    w.u(1, 0);
    w.ue(0);
    w.ue(0);
    w.ue(0);
    w.u(1, 0);
    w.u(2, 0);
    for _ in 0..3 {
        w.ue(0);
    }
    w.u(1, 1);
    w.u(1, 0);
    w.u(1, 0);
    nal(NAL_PPS, w.finish())
}

fn field_slice(idr: bool, frame_num: u32, bottom: bool, poc_lsb: u32) -> Vec<u8> {
    let mut w = W::default();
    w.ue(0);
    w.ue(7);
    w.ue(0);
    w.u(4, frame_num);
    w.u(1, 1);
    w.u(1, u32::from(bottom));
    if idr {
        w.ue(0);
    }
    w.u(6, poc_lsb);
    w.ue(1);
    nal(if idr { NAL_IDR } else { NAL_SLICE }, w.finish())
}

fn rtp(seq: u16, ts: u32, nal: &[u8]) -> Vec<u8> {
    let mut p = vec![0x80u8, 0x80 | 96];
    p.extend_from_slice(&seq.to_be_bytes());
    p.extend_from_slice(&ts.to_be_bytes());
    p.extend_from_slice(&[0, 0, 0, 0]);
    p.extend_from_slice(nal);
    p
}

fn config() -> CodecConfig {
    CodecConfig::Avc {
        config: AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
            configuration_version: 1,
            profile_indication: 77,
            profile_compatibility: 0,
            level_indication: 30,
            length_size_minus_one: 3,
            sps: vec![AvcSps(sps())],
            pps: vec![AvcPps(pps())],
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext: vec![],
        }),
        width: 64,
        height: 64,
    }
}

#[test]
fn field_pictures_use_a_depth_widened_for_fields() {
    let id = TrackId::new(1);
    let mut d = RtpStreamDepacketiser::new(vec![RtpStreamTrack::new(
        id,
        RtpMediaKind::H264,
        config(),
        90_000,
    )]);
    let mut samples = Vec::new();
    let mut seq = 0u16;
    for (k, &frame) in FRAME_ORDER.iter().enumerate() {
        for (parity, bottom) in [false, true].into_iter().enumerate() {
            let ts = frame * FRAME_PERIOD + parity as u32 * FIELD_OFFSET;
            let n = field_slice(
                k == 0,
                (k % 16) as u32,
                bottom,
                (frame * 2 + parity as u32) % 64,
            );
            samples.extend(d.push(id, &rtp(seq, ts, &n)).unwrap());
            seq += 1;
        }
    }
    samples.extend(d.flush(id).unwrap());
    let mut events = Vec::new();
    while let Some(e) = d.poll_loss_event() {
        events.push(e);
    }
    assert_eq!(samples.len(), FRAME_ORDER.len() * 2);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, RtpLossEvent::ReorderDepthExceeded { .. })),
        "{events:?}"
    );
    for w in samples.windows(2) {
        assert!(w[0].dts < w[1].dts, "dts must increase");
    }
    for s in &samples {
        assert!(s.dts <= s.pts, "dts {:?} > pts {:?}", s.dts, s.pts);
    }
}
