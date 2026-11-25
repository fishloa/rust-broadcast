//! The writer against `origin/main`'s `MkvMuxStage` output, byte for byte
//! (streaming-first Phase 5, slice 3).

use std::time::Duration;

use broadcast_matroska::writer::{
    BlockFlags, InfoSpec, Layout, TrackSpec, Write, Writer, info_element, tracks_element,
};

const FIXTURES: &[&str] = &[
    "mkv/h264-aac.mkv",
    "mkv/hevc-aac.mkv",
    "mkv/vorbis.webm",
    "mkv/vp8-opus.webm",
    "mkv/vp9-opus.mkv",
    "mkv/vp9-opus.webm",
];

#[test]
fn writer_reproduces_the_recorded_mkv_mux_output() {
    for fixture in FIXTURES {
        let want = super::support::golden(fixture);
        let collected = super::support::collect(fixture, super::support::Driver::Seekable);
        let laid = super::support::lay_out(&collected);
        let specs: Vec<TrackSpec<'_>> = collected
            .tracks
            .iter()
            .enumerate()
            .map(|(index, track)| TrackSpec {
                track_number: track.number,
                track_uid: index as u64 + 1,
                track_type: track.track_type,
                codec_id: &track.codec_id,
                codec_private: (!track.codec_private.is_empty())
                    .then_some(track.codec_private.as_slice()),
                default_duration_ns: None,
                video: track.pixel_width.zip(track.pixel_height),
                audio: track.channels.zip(track.sampling_frequency),
            })
            .collect();
        let tracks = tracks_element(&specs).unwrap();
        let info = info_element(&InfoSpec {
            timestamp_scale_ns: collected.scale,
            duration_ticks: 0.0,
            muxing_app: "transmux",
            writing_app: "transmux",
        })
        .unwrap();
        let mut writer =
            Writer::new(Layout::new(Duration::from_millis(5_000)), &info, &tracks).unwrap();
        let mut out = Vec::new();
        let mut counters = vec![0usize; collected.tracks.len()];
        for (pts, frame) in laid.merged() {
            let index = collected
                .tracks
                .iter()
                .position(|track| track.number == frame.track)
                .unwrap();
            let position = counters[index];
            counters[index] += 1;
            let track_pts: Vec<i64> = laid.per_track[index].iter().map(|(pts, _)| *pts).collect();
            let delta = if position + 1 < track_pts.len() {
                track_pts[position + 1] - track_pts[position]
            } else if track_pts.len() >= 2 {
                track_pts[position] - track_pts[position - 1]
            } else {
                0
            };
            let duration = (delta > 0).then_some(delta as u64);
            let writes = writer
                .push_block(
                    frame.track,
                    pts,
                    duration,
                    BlockFlags::keyframe(frame.keyframe),
                    &frame.data,
                )
                .unwrap();
            apply(writes, &mut out);
        }
        apply(writer.finish().unwrap(), &mut out);
        assert_eq!(
            format!("len={} fnv={:016x}", out.len(), super::support::fnv1a(&out)),
            want.mkv,
            "{fixture}: writer output"
        );
    }
}

#[test]
fn a_writer_output_reads_back_through_the_reader() {
    let bytes = {
        let info = info_element(&InfoSpec::new(1_000_000, 0.0)).unwrap();
        let specs = [TrackSpec {
            track_number: 1,
            track_uid: 1,
            track_type: 1,
            codec_id: "V_VP9",
            codec_private: None,
            default_duration_ns: None,
            video: Some((32, 16)),
            audio: None,
        }];
        let tracks = tracks_element(&specs).unwrap();
        let mut writer =
            Writer::new(Layout::new(Duration::from_millis(500)), &info, &tracks).unwrap();
        let mut out = Vec::new();
        for (timestamp, keyframe) in [(0i64, true), (40, false), (80, false), (600, true)] {
            let writes = writer
                .push_block(
                    1,
                    timestamp,
                    Some(40),
                    BlockFlags::keyframe(keyframe),
                    &[7u8; 4],
                )
                .unwrap();
            apply(writes, &mut out);
        }
        apply(writer.finish().unwrap(), &mut out);
        out
    };
    let collected = super::support::collect_bytes_seekable(&bytes, usize::MAX);
    assert_eq!(collected.tracks.len(), 1);
    assert_eq!(collected.tracks[0].codec_id(), "V_VP9");
    assert_eq!(collected.frames.len(), 4);
    assert_eq!(collected.frames[0].block_pts, 0);
    assert_eq!(collected.frames[3].block_pts, 600);
    assert!(collected.frames[0].keyframe);
    assert!(collected.frames[3].keyframe);
    assert_eq!(collected.frames[0].data, [7u8; 4]);
}

fn apply(writes: Vec<Write>, out: &mut Vec<u8>) {
    for write in writes {
        match write {
            Write::Append(bytes) => out.extend_from_slice(&bytes),
            Write::At { offset, bytes } => {
                let start = usize::try_from(offset).unwrap();
                out[start..start + bytes.len()].copy_from_slice(&bytes);
            }
            _ => {}
        }
    }
}
