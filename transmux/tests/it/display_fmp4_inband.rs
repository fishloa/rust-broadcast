//! Per-AU display tagging for in-band-parameter-set fMP4 (avc3/hev1), Phase 2b.
use test_harness::collect::Collected as Media;
use transmux::CodecConfig;

fn inband_track(media: &Media) -> &test_harness::collect::CollectedTrack {
    media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("avc track")
}

fn sar(s: &transmux::Sample) -> (u16, u16) {
    let d = s.display.as_ref().expect("display");
    let g = d.geometry.as_ref().expect("geom");
    g.sample_aspect_ratio().expect("sar")
}

#[test]
fn melodie_avc3_all_samples_sar_64_45() {
    let Some(bytes) = test_fixtures::require_bytes("mp4/melodie-avc3.mp4", "display_fmp4_inband")
    else {
        return;
    };
    let media =
        test_harness::collect::media_from_mp4(&bytes, test_harness::collect::CollectMode::Seekable)
            .unwrap();
    let track = inband_track(&media);
    assert!(!track.samples.is_empty());
    for s in &track.samples {
        let d = s.display.as_ref().expect("display");
        let g = d.geometry.as_ref().expect("geom");
        assert_eq!(g.coded_width, 720);
        assert_eq!(g.coded_height, 576);
        let r = g.sample_aspect_ratio().expect("sar");
        assert_eq!(r, (64, 45));
    }
}

/// ISO/IEC 14496-12:2026 §12.1.4.1: the sample entry's `pasp` over-rides the
/// SAR its own parameter sets declare. `melodie-avc3.mp4`'s SPS declares
/// 64:45; its `pasp` is rewritten to 16:11, so every access unit reads 16:11.
#[test]
fn pasp_overrides_the_in_band_sps_sar() {
    let Some(mut bytes) =
        test_fixtures::require_bytes("mp4/melodie-avc3.mp4", "display_fmp4_inband")
    else {
        return;
    };
    let at = bytes
        .windows(4)
        .position(|w| w == b"pasp")
        .expect("pasp box");
    assert_eq!(&bytes[at + 4..at + 12], [0, 0, 0, 64, 0, 0, 0, 45]);
    bytes[at + 4..at + 12].copy_from_slice(&[0, 0, 0, 16, 0, 0, 0, 11]);
    let media =
        test_harness::collect::media_from_mp4(&bytes, test_harness::collect::CollectMode::Seekable)
            .unwrap();
    let track = inband_track(&media);
    assert!(!track.samples.is_empty());
    for s in &track.samples {
        assert_eq!(sar(s), (16, 11));
    }
}

#[test]
fn sarswitch_avc3_switch_at_sps_change() {
    let Some(bytes) = test_fixtures::require_bytes("mp4/sarswitch-avc3.mp4", "display_fmp4_inband")
    else {
        return;
    };
    let media =
        test_harness::collect::media_from_mp4(&bytes, test_harness::collect::CollectMode::Seekable)
            .unwrap();
    let track = inband_track(&media);

    // Independent oracle: scan each sample's length-prefixed NALs for an SPS
    // (type 7) whose bytes differ from the previous one.
    use broadcast_bitstream::annexb::iter_length_prefixed_nals;
    let mut prev_sps: Option<Vec<u8>> = None;
    let mut switch_idx: Option<usize> = None;
    for (i, s) in track.samples.iter().enumerate() {
        let mut sps_this: Option<Vec<u8>> = None;
        if let Ok(nals) = iter_length_prefixed_nals(&s.data) {
            for nal in nals {
                if !nal.is_empty() && (nal[0] & 0x1F) == 7 {
                    sps_this = Some(nal.to_vec());
                    break;
                }
            }
        }
        if let Some(sps) = sps_this {
            if let Some(prev) = &prev_sps
                && *prev != sps
            {
                switch_idx = Some(i);
                break;
            }
            prev_sps = Some(sps);
        }
    }
    let switch_idx = switch_idx.expect("switch");

    // Display SAR must change at exactly that index.
    let mut disp_switch: Option<usize> = None;
    for i in 1..track.samples.len() {
        if sar(&track.samples[i - 1]) != sar(&track.samples[i]) {
            disp_switch = Some(i);
            break;
        }
    }
    assert_eq!(disp_switch, Some(switch_idx));

    // Counts (swap-sensitive).
    let mut c16 = 0usize;
    let mut c64 = 0usize;
    for s in &track.samples {
        let (w, h) = sar(s);
        if w == 16 && h == 11 {
            c16 += 1;
        }
        if w == 64 && h == 45 {
            c64 += 1;
        }
    }
    assert_eq!(c16, 82);
    assert_eq!(c64, 156);

    // Arc sharing: consecutive equal-state samples share one allocation.
    use std::sync::Arc;
    for i in 0..track.samples.len() - 1 {
        let a = track.samples[i].display.as_ref().unwrap();
        let b = track.samples[i + 1].display.as_ref().unwrap();
        if a.geometry == b.geometry && a.active_format == b.active_format {
            assert!(Arc::ptr_eq(a, b), "samples {i} and {} not shared", i + 1);
        }
    }
}

#[test]
fn avc1_out_of_band_keeps_display_none() {
    let Some(bytes) =
        test_fixtures::require_bytes("mp4/frag/h264-high.frag.mp4", "display_fmp4_inband")
    else {
        return;
    };
    let media =
        test_harness::collect::media_from_mp4(&bytes, test_harness::collect::CollectMode::Seekable)
            .unwrap();
    // only check if an avc track exists
    if let Some(track) = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
    {
        // confirm out-of-band entry (avc1) by checking display per sample is None
        // We can't easily know fourcc; just assert all None (in-band would be Some).
        for s in &track.samples {
            assert!(s.display.is_none(), "avc1 track should not be tagged");
        }
    }
}

#[test]
fn hostile_overrun_length_prefix_does_not_panic() {
    let Some(bytes) = test_fixtures::require_bytes("mp4/melodie-avc3.mp4", "display_fmp4_inband")
    else {
        return;
    };
    // Find the first sample's NAL bytes in the file so we can corrupt its
    // (2-byte) length prefix in place, keeping the avc3 sample entry.
    let media =
        test_harness::collect::media_from_mp4(&bytes, test_harness::collect::CollectMode::Seekable)
            .unwrap();
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("avc track");
    let sample = &track.samples[0];
    let needle = &sample.data[4..8];
    let pos = bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("nal found");
    assert!(pos >= 2);
    let mut bytes = bytes.to_vec();
    // Overwrite the 2-byte length prefix preceding the NAL.
    bytes[pos - 2] = 0xFF;
    bytes[pos - 1] = 0xFF;

    let media2 =
        test_harness::collect::media_from_mp4(&bytes, test_harness::collect::CollectMode::Seekable)
            .unwrap();
    let track2 = media2
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("avc track");
    // No panic; first sample keeps a tag (seeded state).
    assert!(track2.samples[0].display.is_some());
}

// ── HEVC in-band entries: `hev1` and Dolby Vision `dvhe` ───────────────────
//
// ISO/IEC 14496-15:2017 §8.4.1.1: an `hev1` entry carries its parameter sets
// in-band, exactly as `avc3` does for AVC, and a Dolby Vision `dvhe` entry is
// an HEVC entry with `dvcC`/`dvvC` boxes. `hvc1` is the out-of-band one.

const HEVC_FRAGMENTED: &str = "mp4/frag/hevc-main.frag.mp4";

fn hevc_track(media: &Media) -> &test_harness::collect::CollectedTrack {
    media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }))
        .expect("hevc track")
}

/// The fixture with its sample-entry four-CC rewritten, so the same bytes are
/// read as an in-band (`hev1`/`dvhe`) or out-of-band (`hvc1`) entry.
fn hevc_with_entry(fourcc: &[u8; 4]) -> Vec<u8> {
    let mut bytes = test_fixtures::read(HEVC_FRAGMENTED);
    let at = bytes
        .windows(4)
        .position(|w| w == b"hev1")
        .expect("hev1 sample entry");
    bytes[at..at + 4].copy_from_slice(fourcc);
    bytes
}

#[test]
fn hev1_in_band_hevc_is_tagged_per_access_unit() {
    let media = test_harness::collect::media_from_mp4(
        hevc_with_entry(b"hev1").as_slice(),
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("demux");
    let track = hevc_track(&media);
    assert!(!track.samples.is_empty());
    for (i, s) in track.samples.iter().enumerate() {
        let d = s
            .display
            .as_ref()
            .unwrap_or_else(|| panic!("hev1 sample {i} must be tagged in-band"));
        let g = d.geometry.as_ref().expect("hev1 geometry");
        assert_eq!(
            (g.coded_width, g.coded_height),
            (320, 240),
            "hev1 sample {i} coded size from the in-band SPS"
        );
        assert_eq!(
            g.sample_aspect_ratio(),
            Some((1u16, 1u16)),
            "hev1 sample {i} SAR"
        );
    }
}

#[test]
fn dvhe_dolby_vision_hevc_is_tagged_per_access_unit() {
    let media = test_harness::collect::media_from_mp4(
        hevc_with_entry(b"dvhe").as_slice(),
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("demux");
    let track = hevc_track(&media);
    assert!(!track.samples.is_empty());
    for (i, s) in track.samples.iter().enumerate() {
        assert!(
            s.display.is_some(),
            "dvhe sample {i} must be tagged in-band, like hev1"
        );
    }
}

#[test]
fn hvc1_out_of_band_hevc_is_not_tagged() {
    let media = test_harness::collect::media_from_mp4(
        hevc_with_entry(b"hvc1").as_slice(),
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("demux");
    let track = hevc_track(&media);
    assert!(!track.samples.is_empty());
    for (i, s) in track.samples.iter().enumerate() {
        assert!(
            s.display.is_none(),
            "an hvc1 entry's parameter sets are out of band; sample {i} must not be tagged"
        );
    }
}
