//! Per-AU display tagging for Program Stream (PS) video samples.

use broadcast_common::Unpackage;
use std::sync::Arc;
use transmux::PsDemux;
use transmux::media::Media;
use transmux::pipeline::CodecConfig;

fn video_track(media: &Media, is_mpeg2: bool) -> &transmux::Track {
    media
        .tracks
        .iter()
        .find(|t| {
            matches!(
                (&t.spec.config, is_mpeg2),
                (CodecConfig::Avc { .. }, false) | (CodecConfig::Mpeg2Video { .. }, true)
            )
        })
        .expect("video track")
}

#[test]
fn melodie_h264_ps_all_samples_720x576_sar_64_45() {
    let Some(bytes) = test_fixtures::require_bytes("ps/melodie-h264-sar-64-45.ps", "display_ps")
    else {
        return;
    };
    let media: Media = PsDemux::new().unpackage(&bytes).expect("demux");
    let track = video_track(&media, false);
    assert!(!track.samples.is_empty());
    for s in &track.samples {
        let d = s.display.as_ref().expect("display");
        let g = d.geometry.as_ref().expect("geom");
        assert_eq!(g.coded_width, 720);
        assert_eq!(g.coded_height, 576);
        let r = g.sample_aspect_ratio().expect("sar");
        // Distinct from 16:11 and square.
        assert_eq!(r, (64, 45), "sar must be 64:45, not 16:11 or square");
    }
}

#[test]
fn test108_mpeg2_ps_sar_and_afd() {
    // ffprobe command:
    //   ffprobe -v error -select_streams v:0 -show_frames \
    //     -show_entries frame=pkt_dts_time,side_data_list \
    //     -of csv private/fixtures/ps/test108-mpeg2-afd.ps
    // First frame carrying Active format description side data:
    //   pkt_dts_time=0.620000, active_format=8 (SameAsCodedFrame).
    // In decode order this corresponds to sample index 11.
    let Some(bytes) = test_fixtures::require_bytes("ps/test108-mpeg2-afd.ps", "display_ps") else {
        return;
    };
    let media: Media = PsDemux::new().unpackage(&bytes).expect("demux");
    let track = video_track(&media, true);
    assert!(!track.samples.is_empty());
    for s in &track.samples {
        let d = s.display.as_ref().expect("display");
        let g = d.geometry.as_ref().expect("geom");
        assert_eq!(g.coded_width, 720);
        assert_eq!(g.coded_height, 576);
        assert_eq!(g.sample_aspect_ratio().expect("sar"), (64, 45));
    }
    // At least one sample carries an AFD.
    let afd_samples: Vec<usize> = track
        .samples
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.display
                .as_ref()
                .and_then(|d| d.active_format.as_ref())
                .and_then(|a| a.afd)
                .is_some()
        })
        .map(|(i, _)| i)
        .collect();
    assert!(!afd_samples.is_empty());
    // First AFD-bearing sample index and value match ffprobe.
    let first = afd_samples[0];
    assert_eq!(first, 11);
    let af = track.samples[first]
        .display
        .as_ref()
        .unwrap()
        .active_format
        .as_ref()
        .unwrap()
        .afd
        .unwrap();
    assert_eq!(af, transmux::ActiveFormatDescription::SameAsCodedFrame);
}

#[test]
fn public_ps_fixtures_tagged() {
    // ffmpeg-h264-noaud.ps: h264 320x240 SAR 1:1
    let bytes = test_fixtures::read("ps/ffmpeg-h264-noaud.ps");
    let media: Media = PsDemux::new().unpackage(&bytes).expect("demux");
    let track = video_track(&media, false);
    assert!(!track.samples.is_empty());
    for s in &track.samples {
        assert!(s.display.is_some());
        let g = s.display.as_ref().unwrap().geometry.as_ref().unwrap();
        assert_eq!(g.coded_width, 320);
        assert_eq!(g.coded_height, 240);
        assert_eq!(g.sample_aspect_ratio().unwrap(), (1, 1));
    }

    // ffmpeg-mpeg2video-mp2.ps: mpeg2 352x288 SAR 1:1
    let bytes = test_fixtures::read("ps/ffmpeg-mpeg2video-mp2.ps");
    let media: Media = PsDemux::new().unpackage(&bytes).expect("demux");
    let track = video_track(&media, true);
    assert!(!track.samples.is_empty());
    for s in &track.samples {
        assert!(s.display.is_some());
        let g = s.display.as_ref().unwrap().geometry.as_ref().unwrap();
        assert_eq!(g.coded_width, 352);
        assert_eq!(g.coded_height, 288);
        assert_eq!(g.sample_aspect_ratio().unwrap(), (1, 1));
    }

    // fixtures/mpeg-ps/ffmpeg-mpeg2-ps.mpg: mpeg2 64x48 DAR 4:3 (SAR 1:1)
    let bytes = test_fixtures::read("mpeg-ps/ffmpeg-mpeg2-ps.mpg");
    let media: Media = PsDemux::new().unpackage(&bytes).expect("demux");
    let track = video_track(&media, true);
    assert!(!track.samples.is_empty());
    for s in &track.samples {
        assert!(s.display.is_some());
        let g = s.display.as_ref().unwrap().geometry.as_ref().unwrap();
        assert_eq!(g.coded_width, 64);
        assert_eq!(g.coded_height, 48);
        assert_eq!(g.sample_aspect_ratio().unwrap(), (1, 1));
    }
}

#[test]
fn ps_audio_track_display_none() {
    // `ffmpeg-mpeg2video-mp2.ps` carries MPEG-1 Layer II audio, which this
    // demuxer intentionally does not surface (see ps_demux module docs); use
    // a fixture that does expose an audio track instead.
    let bytes = test_fixtures::read("ps/ffmpeg-mpeg2video-2xac3.ps");
    let media: Media = PsDemux::new().unpackage(&bytes).expect("demux");
    let audio = media
        .tracks
        .iter()
        .find(|t| {
            !matches!(t.spec.config, CodecConfig::Mpeg2Video { .. })
                && !matches!(t.spec.config, CodecConfig::Avc { .. })
        })
        .expect("audio track");
    assert!(!audio.samples.is_empty());
    for s in &audio.samples {
        assert!(s.display.is_none());
    }
}

#[test]
fn ps_arc_sharing() {
    let Some(bytes) = test_fixtures::require_bytes("ps/melodie-h264-sar-64-45.ps", "display_ps")
    else {
        return;
    };
    let media: Media = PsDemux::new().unpackage(&bytes).expect("demux");
    let track = video_track(&media, false);
    for i in 0..track.samples.len() - 1 {
        let a = track.samples[i].display.as_ref().unwrap();
        let b = track.samples[i + 1].display.as_ref().unwrap();
        if a.geometry == b.geometry && a.active_format == b.active_format {
            assert!(Arc::ptr_eq(a, b), "samples {i} and {} not shared", i + 1);
        }
    }
}

#[test]
fn sarswitch_h264_ps_tags_the_switch_with_the_sps_change() {
    // ffprobe -show_entries frame=sample_aspect_ratio on the file: 78 frames
    // SAR 64:45 followed by 80 frames SAR 16:11 (a real in-band SPS change).
    let Some(bytes) = test_fixtures::require_bytes("ps/sarswitch-h264.ps", "display_ps") else {
        return;
    };
    let media: Media = PsDemux::new().unpackage(&bytes).expect("demux");
    let track = video_track(&media, false);
    let sars: Vec<(u16, u16)> = track
        .samples
        .iter()
        .map(|s| {
            s.display
                .as_ref()
                .expect("display")
                .geometry
                .as_ref()
                .expect("geom")
                .sample_aspect_ratio()
                .expect("sar")
        })
        .collect();
    let first = sars.iter().filter(|&&r| r == (64, 45)).count();
    let second = sars.iter().filter(|&&r| r == (16, 11)).count();
    assert_eq!((first, second), (78, 80));
    assert_eq!(first + second, sars.len());
    // One switch, in order: every 64:45 sample precedes every 16:11 sample.
    assert!(sars[..first].iter().all(|&r| r == (64, 45)));
    assert!(sars[first..].iter().all(|&r| r == (16, 11)));
}
