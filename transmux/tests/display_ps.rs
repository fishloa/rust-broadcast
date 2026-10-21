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

/// The SPS NAL payloads (nal_unit_type 7) found in one Annex B access unit,
/// scanned from the raw bytes (independent of the display tracker).
fn sps_nals(au: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= au.len() {
        if au[i] == 0 && au[i + 1] == 0 && au[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::new();
    for (k, &st) in starts.iter().enumerate() {
        let end = starts.get(k + 1).map_or(au.len(), |&n| n - 3);
        if st < end && au[st] & 0x1F == 7 {
            // Trailing zero bytes belong to the next start code.
            let mut e = end;
            while e > st && au[e - 1] == 0 {
                e -= 1;
            }
            out.push(&au[st..e]);
        }
    }
    out
}

#[test]
fn sarswitch_h264_ps_tags_the_switch_with_the_sps_change() {
    // ffprobe on the file: 78 frames SAR 64:45 then 80 frames SAR 16:11 (a real
    // in-band SPS change). PS demux emits one sample per coded picture (a
    // field pair is two samples), so sample counts differ from ffprobe's frame
    // counts; the switch position is instead derived from the raw SPS bytes.
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

    // Oracle: the first sample carrying an SPS that differs from the first SPS.
    let mut first_sps: Option<&[u8]> = None;
    let mut expected_switch = None;
    for (idx, s) in track.samples.iter().enumerate() {
        for sps in sps_nals(&s.data) {
            match first_sps {
                None => first_sps = Some(sps),
                Some(f) if f != sps && expected_switch.is_none() => expected_switch = Some(idx),
                _ => {}
            }
        }
    }
    let switch = expected_switch.expect("the fixture has an in-band SPS change");
    assert!(switch > 0 && switch < sars.len());
    assert!(
        sars[..switch].iter().all(|&r| r == (64, 45)),
        "before the SPS change"
    );
    assert!(
        sars[switch..].iter().all(|&r| r == (16, 11)),
        "from the SPS change on"
    );
}
