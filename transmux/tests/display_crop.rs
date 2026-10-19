//! Crop / conformance-window tests (DESIGN.md Phase 5).

use transmux::ir::DisplayInfo;
use transmux::ir::{AspectRatioIdc, DisplayGeometry, PictureRect};

// --- local bit sink (mirror of sps.rs test helper) ---
struct BitSink {
    bytes: Vec<u8>,
    cur: u8,
    nbits: u32,
}
impl BitSink {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            cur: 0,
            nbits: 0,
        }
    }
    fn bit(&mut self, b: bool) {
        if b {
            self.cur |= 1 << (7 - self.nbits);
        }
        self.nbits += 1;
        if self.nbits == 8 {
            self.bytes.push(self.cur);
            self.cur = 0;
            self.nbits = 0;
        }
    }
    fn bits(&mut self, v: u64, n: u32) {
        for i in (0..n).rev() {
            self.bit((v >> i) & 1 == 1);
        }
    }
    fn ue(&mut self, v: u64) {
        let code = v + 1;
        let len = 64 - code.leading_zeros();
        for _ in 0..len - 1 {
            self.bit(false);
        }
        self.bits(code, len);
    }
    fn finish(mut self) -> Vec<u8> {
        if self.nbits > 0 {
            self.bytes.push(self.cur);
        }
        self.bytes
    }
}

fn avc_sps_crop(
    chroma_format_idc: u8,
    separate_colour_plane: bool,
    frame_mbs_only: bool,
    crop: (u64, u64, u64, u64), // left,right,top,bottom
) -> Vec<u8> {
    let mut nal = vec![0x67u8, 100, 0x00, 30]; // high profile
    let mut s = BitSink::new();
    s.ue(0); // seq_parameter_set_id
    // high profile branch
    s.ue(u64::from(chroma_format_idc));
    if chroma_format_idc == 3 {
        s.bit(separate_colour_plane);
    }
    s.ue(0); // bit_depth_luma_minus8
    s.ue(0); // bit_depth_chroma_minus8
    s.bit(false); // qpprime_y_zero_transform_bypass
    s.bit(false); // seq_scaling_matrix_present
    s.ue(0); // log2_max_frame_num_minus4
    s.ue(0); // pic_order_cnt_type
    s.ue(0); // log2_max_pic_order_cnt_lsb_minus4
    s.ue(1); // max_num_ref_frames
    s.bit(false); // gaps
    s.ue(44); // pic_width_in_mbs_minus1 -> 720
    // pic height: keep coded height 576 for both
    if frame_mbs_only {
        s.ue(35); // 36 map units *16 = 576
    } else {
        s.ue(17); // 18 map units *2*16 = 576
    }
    s.bit(frame_mbs_only); // frame_mbs_only_flag
    if !frame_mbs_only {
        s.bit(false); // mb_adaptive_frame_field
    }
    s.bit(true); // direct_8x8_inference
    s.bit(true); // frame_cropping_flag
    s.ue(crop.0);
    s.ue(crop.1);
    s.ue(crop.2);
    s.ue(crop.3);
    s.bit(false); // vui
    s.bit(true); // rbsp stop
    nal.extend(s.finish());
    nal
}

// Because HEVC default-display-window testing via hand-built SPS is complex,
// we instead test default_display_rect via DisplayGeometry directly in test E.

#[test]
fn avc_crop_unit_table() {
    // (chroma, separate, frame_mbs_only) -> (CropUnitX, CropUnitY)
    let cases: &[(u8, bool, bool, (u64, u64))] = &[
        (1, false, true, (2, 2)),  // 4:2:0 progressive
        (1, false, false, (2, 4)), // 4:2:0 interlaced
        (2, false, true, (2, 1)),  // 4:2:2 progressive
        (2, false, false, (2, 2)), // 4:2:2 interlaced
        (3, false, true, (1, 1)),  // 4:4:4 progressive
        (3, false, false, (1, 2)), // 4:4:4 interlaced
        (0, false, true, (1, 1)),  // monochrome progressive (2 - 1)
        (0, false, false, (1, 2)), // monochrome interlaced
        (3, true, true, (1, 1)),   // separate plane -> ChromaArrayType 0
        (3, true, false, (1, 2)),
    ];
    for (chroma, sep, fmo, (cu_x, cu_y)) in cases {
        let sps = avc_sps_crop(*chroma, *sep, *fmo, (1, 2, 3, 4));
        let g = DisplayGeometry::from_avc_sps(&sps).expect("parse");
        let coded_w: u32 = 720;
        let coded_h: u32 = 576;
        assert_eq!(
            g.coded_width, coded_w,
            "chroma {chroma} sep {sep} fmo {fmo}"
        );
        assert_eq!(
            g.coded_height, coded_h,
            "chroma {chroma} sep {sep} fmo {fmo}"
        );
        let expected = PictureRect::new(
            *cu_x as u32,
            (cu_y * 3) as u32,
            coded_w - (cu_x * 3) as u32,
            coded_h - (cu_y * 7) as u32,
        );
        assert_eq!(
            g.output_rect, expected,
            "chroma {chroma} sep {sep} fmo {fmo}"
        );
    }
}

#[test]
fn avc_out_of_range_crop_is_full_picture() {
    // offsets that exceed the picture
    let sps = avc_sps_crop(1, false, true, (1000, 1000, 1000, 1000));
    let g = DisplayGeometry::from_avc_sps(&sps).expect("parse");
    assert_eq!(
        g.output_rect,
        PictureRect::new(0, 0, g.coded_width, g.coded_height)
    );
}

#[test]
fn changes_from_crop_and_default() {
    let g1 = DisplayGeometry::new(100, 100, None);
    // crop-only: change output_rect, coded size same
    let mut g2 = DisplayGeometry::new(100, 100, None);
    g2.output_rect = PictureRect::new(10, 10, 80, 80);
    let i1 = DisplayInfo::new(Some(g1.clone()));
    let i2 = DisplayInfo::new(Some(g2.clone()));
    let c = i2.changes_from(Some(&i1));
    assert!(c.output_rect);
    assert!(!c.aspect_ratio);
    assert!(!c.default_display_rect);
    assert!(!c.video_signal);
    assert!(!c.overscan);

    // SAR-only change does not raise output_rect
    let mut g3 = DisplayGeometry::new(100, 100, Some(AspectRatioIdc::Square));
    g3.output_rect = PictureRect::new(0, 0, 100, 100);
    let mut g4 = DisplayGeometry::new(100, 100, Some(AspectRatioIdc::Sar16x11));
    g4.output_rect = PictureRect::new(0, 0, 100, 100);
    let c2 = DisplayInfo::new(Some(g4)).changes_from(Some(&DisplayInfo::new(Some(g3))));
    assert!(!c2.output_rect);
    assert!(c2.aspect_ratio);

    // default-display-window-only change raises default_display_rect only
    let g5 = DisplayGeometry::new(100, 100, None);
    let mut g6 = DisplayGeometry::new(100, 100, None);
    g6.default_display_rect = Some(PictureRect::new(5, 5, 90, 90));
    let c3 = DisplayInfo::new(Some(g6)).changes_from(Some(&DisplayInfo::new(Some(g5))));
    assert!(c3.default_display_rect);
    assert!(!c3.output_rect);
    assert!(!c3.aspect_ratio);
}

// Real fixtures
fn read(fixture: &str) -> Option<Vec<u8>> {
    let path = test_fixtures::require(fixture, "display_crop")?;
    Some(std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())))
}

fn streaming_specs(data: &[u8]) -> Vec<transmux::ir::TrackSpec> {
    use transmux::ts_demux::{DemuxEvent, StreamingTsDemux};
    let mut demux = StreamingTsDemux::new();
    let mut specs = Vec::new();
    for chunk in data.chunks(188 * 512) {
        demux.feed(chunk);
        while let Some(ev) = demux.poll_event() {
            if let DemuxEvent::TrackAdded(s) | DemuxEvent::TrackUpdated(s) = ev
                && matches!(s.config, transmux::pipeline::CodecConfig::Avc { .. })
            {
                specs.push(s);
            }
        }
    }
    demux.finish();
    while let Some(ev) = demux.poll_event() {
        if let DemuxEvent::TrackAdded(s) | DemuxEvent::TrackUpdated(s) = ev
            && matches!(s.config, transmux::pipeline::CodecConfig::Avc { .. })
        {
            specs.push(s);
        }
    }
    specs
}

fn batch_specs(data: &[u8]) -> Vec<transmux::ir::TrackSpec> {
    use transmux::ts_demux::TsDemux;
    let media = TsDemux::new().demux(data).expect("batch");
    media
        .tracks
        .iter()
        .filter(|t| matches!(t.config(), transmux::pipeline::CodecConfig::Avc { .. }))
        .map(|t| t.spec.clone())
        .collect()
}

fn check_real(fixture: &str, coded: (u32, u32), out: (u32, u32, u32, u32)) {
    let Some(data) = read(fixture) else {
        return;
    };
    for specs in [batch_specs(&data), streaming_specs(&data)] {
        assert!(!specs.is_empty(), "{fixture}: no AVC track");
        for spec in &specs {
            let g = spec.display.as_ref().expect("display");
            assert_eq!((g.coded_width, g.coded_height), coded, "{fixture}");
            assert_eq!(
                g.output_rect,
                PictureRect::new(out.0, out.1, out.2, out.3),
                "{fixture}"
            );
        }
    }
}

#[test]
fn real_fixtures_crop() {
    check_real(
        "ts/h264/high_1080_cropped.ts",
        (1920, 1088),
        (0, 0, 1920, 1080),
    );
    // transformers: coded 720x576, output 720x576-8=568? coded height 34 map units*16*2? Let's compute: pic_height_map=33+1=34, frame_height=2*34*16=1088? Wait ffmpeg said pic_height 34 map units, frame_mbs_only 0 -> frame_height=2*34*16=1088. Output = 1088 - 4*2? CropUnitY=2*2=4, bottom offset 2 -> 8 lines => 1080. But ffprobe says 720x576? Hmm maybe fixture is 720x576 coded? Let's not assert exact; instead assert output_rect bottom crop 8: height 1080.
    check_real(
        "ts/transformers-eia608-h264.ts",
        (1920, 1088),
        (0, 0, 1920, 1080),
    );
    check_real(
        "ts/melodie-anamorphic-sar-64-45-zenith-tv.ts",
        (720, 576),
        (0, 0, 720, 576),
    );
    check_real(
        "ts/krone-anamorphic-sar-16-11-zenith-tv.ts",
        (720, 576),
        (0, 0, 720, 576),
    );
}

fn hevc_sps_conf(chroma: u8, conf: (u64, u64, u64, u64)) -> Vec<u8> {
    let mut nal = vec![0x42u8, 0x01];
    let mut s = BitSink::new();
    s.bits(0, 4);
    s.bits(0, 3);
    s.bit(false);
    s.bits(0, 2);
    s.bit(false);
    s.bits(1, 5);
    s.bits(0, 32);
    s.bits(0xE0_00_00_00_00_00u64, 48);
    s.bits(93, 8);
    s.ue(0); // sps id
    s.ue(u64::from(chroma));
    if chroma == 3 {
        s.bit(false);
    }
    s.ue(32); // pic width
    s.ue(32); // pic height
    s.bit(true); // conf window flag
    s.ue(conf.0);
    s.ue(conf.1);
    s.ue(conf.2);
    s.ue(conf.3);
    s.ue(0);
    s.ue(0); // bit depths
    // minimal tail: rbsp stop (VUI absent) - conf offsets already read
    s.bit(true);
    nal.extend(s.finish());
    nal
}

fn hevc_sps_ddw(chroma: u8, conf: (u64, u64, u64, u64), def: (u64, u64, u64, u64)) -> Vec<u8> {
    let mut nal = vec![0x42u8, 0x01];
    let mut s = BitSink::new();
    s.bits(0, 4);
    s.bits(0, 3);
    s.bit(false);
    s.bits(0, 2);
    s.bit(false);
    s.bits(1, 5);
    s.bits(0, 32);
    s.bits(0xE0_00_00_00_00_00u64, 48);
    s.bits(93, 8);
    s.ue(0);
    s.ue(u64::from(chroma));
    if chroma == 3 {
        s.bit(false);
    }
    s.ue(64);
    s.ue(64);
    s.bit(true);
    s.ue(conf.0);
    s.ue(conf.1);
    s.ue(conf.2);
    s.ue(conf.3);
    s.ue(0);
    s.ue(0);
    // tail to VUI
    s.ue(3); // log2_max_pic_order_cnt_lsb_minus4
    s.bit(false); // sub_layer_ordering present
    s.ue(0);
    s.ue(0);
    s.ue(0); // buffering
    s.ue(0);
    s.ue(3);
    s.ue(0);
    s.ue(3);
    s.ue(1);
    s.ue(1); // transform depths
    s.bit(false); // scaling_list
    s.bit(false); // amp
    s.bit(false); // sao
    s.bit(false); // pcm
    s.ue(0); // num_short_term
    s.bit(false); // long_term
    s.bit(false); // temporal mvp
    s.bit(false); // strong intra
    s.bit(true); // vui present
    s.bit(false); // aspect ratio
    s.bit(false); // overscan
    s.bit(false); // video signal
    s.bit(false); // chroma loc
    s.bit(false); // neutral chroma
    s.bit(false); // field_seq
    s.bit(false); // frame_field_info
    s.bit(true); // default_display_window_flag
    s.ue(def.0);
    s.ue(def.1);
    s.ue(def.2);
    s.ue(def.3);
    s.bit(false); // vui timing
    s.bit(true); // rbsp stop
    nal.extend(s.finish());
    nal
}

#[test]
fn hevc_conformance_window_table() {
    // chroma -> (SubWidthC, SubHeightC)
    let subs: &[(u8, (u64, u64))] = &[(0, (1, 1)), (1, (2, 2)), (2, (2, 1)), (3, (1, 1))];
    for (chroma, (sw, sh)) in subs {
        let conf = (1u64, 2, 3, 4);
        let sps = hevc_sps_conf(*chroma, conf);
        let g = DisplayGeometry::from_hevc_sps(&sps).expect("parse");
        let coded = (32u32, 32u32);
        assert_eq!((g.coded_width, g.coded_height), coded);
        let expected = PictureRect::new(
            (sw * conf.0) as u32,
            (sh * conf.2) as u32,
            (coded.0 as u64 - sw * (conf.0 + conf.1)) as u32,
            (coded.1 as u64 - sh * (conf.2 + conf.3)) as u32,
        );
        assert_eq!(g.output_rect, expected, "chroma {chroma}");
        assert!(g.default_display_rect.is_none());
    }
}

#[test]
fn hevc_default_display_window_adds() {
    let conf = (1u64, 2, 3, 4);
    let def = (5u64, 6, 7, 8);
    let sps = hevc_sps_ddw(1, conf, def);
    let g = DisplayGeometry::from_hevc_sps(&sps).expect("parse");
    // output_rect uses conf only
    let sw = 2u64;
    let sh = 2u64;
    let out = PictureRect::new(
        (sw * conf.0) as u32,
        (sh * conf.2) as u32,
        (64 - sw * (conf.0 + conf.1)) as u32,
        (64 - sh * (conf.2 + conf.3)) as u32,
    );
    assert_eq!(g.output_rect, out);
    // default = conf + def
    let left_off = conf.0 + def.0;
    let right_off = conf.1 + def.1;
    let top_off = conf.2 + def.2;
    let bot_off = conf.3 + def.3;
    let dd = g.default_display_rect.expect("ddw");
    assert_eq!(
        dd,
        PictureRect::new(
            (sw * left_off) as u32,
            (sh * top_off) as u32,
            (64 - sw * (left_off + right_off)) as u32,
            (64 - sh * (top_off + bot_off)) as u32
        )
    );
}

#[test]
fn hevc_default_display_window_flag_zero_is_none() {
    // no vui -> None
    let sps = hevc_sps_conf(1, (1, 2, 3, 4));
    let g = DisplayGeometry::from_hevc_sps(&sps).expect("parse");
    assert!(g.default_display_rect.is_none());
}
