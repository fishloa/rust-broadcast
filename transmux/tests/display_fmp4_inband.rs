//! Per-AU display tagging for in-band-parameter-set fMP4 (avc3/hev1), Phase 2b.

use broadcast_common::Unpackage;
use transmux::media::Fmp4Demux;
use transmux::{CodecConfig, Media};

fn inband_track(media: &Media) -> &transmux::Track {
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
    let Some(bytes) = test_fixtures::require_bytes("fmp4/melodie-avc3.mp4", "display_fmp4_inband")
    else {
        return;
    };
    let media = Fmp4Demux::new().unpackage(&bytes).unwrap();
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

#[test]
fn sarswitch_avc3_switch_at_sps_change() {
    let Some(bytes) =
        test_fixtures::require_bytes("fmp4/sarswitch-avc3.mp4", "display_fmp4_inband")
    else {
        return;
    };
    let media = Fmp4Demux::new().unpackage(&bytes).unwrap();
    let track = inband_track(&media);

    // Independent oracle: scan each sample's length-prefixed NALs for an SPS
    // (type 7) whose bytes differ from the previous one.
    use transmux::annexb::iter_length_prefixed_nals;
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
        test_fixtures::require_bytes("mp4/frag/h264_high.frag.mp4", "display_fmp4_inband")
    else {
        return;
    };
    let media = Fmp4Demux::new().unpackage(&bytes).unwrap();
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
    let Some(bytes) = test_fixtures::require_bytes("fmp4/melodie-avc3.mp4", "display_fmp4_inband")
    else {
        return;
    };
    // Find the first sample's NAL bytes in the file so we can corrupt its
    // (2-byte) length prefix in place, keeping the avc3 sample entry.
    let media = Fmp4Demux::new().unpackage(&bytes).unwrap();
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

    let media2 = Fmp4Demux::new().unpackage(&bytes).unwrap();
    let track2 = media2
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("avc track");
    // No panic; first sample keeps a tag (seeded state).
    assert!(track2.samples[0].display.is_some());
}
