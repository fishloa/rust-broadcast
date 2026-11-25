//! A forced fMP4 track change rolls to a second period; each period holds one
//! `EXT-X-MAP` and plays on its own (8.3).

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use broadcast_common::mux::Unpackage;
use broadcast_hls::MediaPlaylist;
use bytes::Bytes;
use dvr_runtime::fs::{FsArchive, FsRecorder};
use dvr_runtime::read::parse_period_playlist;
use dvr_runtime::writer::InitGeneration;
use dvr_runtime::{ArchiveConfig, ArchiveContainer, ArchiveName, PeriodNum, RetentionLimits};
use media_plane::ids::SegmentSeq;
use media_plane::segmenter::program::{ProgramSegmenter, SegmenterConfig};
use media_plane::trunk::{RetentionClass, Trunk, TrunkConfig};
use transmux::Container;
use transmux::pipeline::{Sample, TrackSpec};

use crate::support::{cleanup, temp_dir};

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("nonzero")
}

fn trunk_config() -> TrunkConfig {
    TrunkConfig::default()
        .with_timed_capacity(nz(65_536))
        .with_sparse_capacity(nz(8))
        .with_segment_capacity(nz(64))
        .with_event_capacity(nz(8))
        .with_part_capacity(nz(8))
}

struct Source {
    spec: TrackSpec,
    samples: Vec<Sample>,
}

fn load() -> Vec<Source> {
    let bytes = test_fixtures::read("ts/h264-aac.ts");
    let media = transmux::ts_demux::TsDemux::new()
        .unpackage(&bytes)
        .expect("demux the fixture");
    media
        .tracks
        .into_iter()
        .map(|track| Source {
            spec: track.spec,
            samples: track.samples,
        })
        .collect()
}

pub struct Fmp4Recording {
    pub root: std::path::PathBuf,
    pub archive: FsArchive,
    pub periods: Vec<PeriodNum>,
}

fn poll(
    recorder: &mut FsRecorder,
    segmenter: &mut ProgramSegmenter,
    trunk: &Arc<Trunk>,
    now: Timestamp,
    last_init: &mut Option<Bytes>,
) {
    let mut events = Vec::new();
    segmenter.pump(media_plane::ingress::ProgramId(1), trunk, &mut events, &());
    if let Some(init) = segmenter.init_segment() {
        let init = Bytes::from(init);
        if last_init.as_deref() != Some(init.as_ref()) {
            let next = trunk
                .last_closed_segment()
                .map_or(SegmentSeq::new(0), |seq| seq.saturating_next());
            recorder
                .poll(now, |_| None, &[InitGeneration::new(next, init.clone())])
                .expect("poll with the new init");
            *last_init = Some(init);
            return;
        }
    }
    recorder.poll(now, |_| None, &[]).expect("poll");
}

pub fn record_into(root: std::path::PathBuf) -> Fmp4Recording {
    let mut sources = load();
    assert!(sources.len() >= 2, "the fixture carries video and audio");
    let video = sources.remove(0);
    let audio = sources.remove(0);
    let mut extra = audio.spec.clone();
    extra.track_id = transmux::TrackId::new(99);

    let trunk: Arc<Trunk> = Trunk::new(trunk_config());
    let writer = trunk.writer().expect("writer");
    writer.set_tracks(vec![video.spec.clone(), audio.spec.clone()]);
    let config = SegmenterConfig::default()
        .with_container(Container::Fmp4)
        .with_target_duration(Duration::from_secs(2))
        .with_part_target(Duration::from_millis(500))
        .with_route_name("cam")
        .with_trunk_config(trunk_config());
    let (segmenter, _) = ProgramSegmenter::try_new(&trunk, config);
    let mut segmenter = segmenter.expect("a segmenter");
    let archive_config = ArchiveConfig::new(
        ArchiveContainer::Fmp4,
        Duration::from_secs(2),
        RetentionLimits::new().with_periods(nz(10)),
    )
    .with_period_duration(Some(Duration::from_secs(3600)));
    let mut recorder = FsRecorder::new(
        &root,
        ArchiveName::new("cam").expect("name"),
        archive_config,
        &trunk,
    )
    .expect("recorder");

    let mut last_init: Option<Bytes> = None;
    let mut now = Timestamp::ZERO;
    poll(&mut recorder, &mut segmenter, &trunk, now, &mut last_init);

    let half = video.samples.len() / 2;
    for sample in &video.samples[..half] {
        writer.publish(video.spec.track_id, RetentionClass::Timed, sample.clone());
    }
    let audio_half = audio.samples.len() / 2;
    for sample in &audio.samples[..audio_half] {
        writer.publish(audio.spec.track_id, RetentionClass::Timed, sample.clone());
    }
    now = now.saturating_add(Duration::from_secs(4));
    poll(&mut recorder, &mut segmenter, &trunk, now, &mut last_init);

    writer.set_tracks(vec![video.spec.clone(), audio.spec.clone(), extra.clone()]);
    for sample in &video.samples[half..] {
        writer.publish(video.spec.track_id, RetentionClass::Timed, sample.clone());
    }
    for sample in &audio.samples[audio_half..] {
        writer.publish(audio.spec.track_id, RetentionClass::Timed, sample.clone());
    }
    for sample in &audio.samples[audio_half..] {
        writer.publish(extra.track_id, RetentionClass::Timed, sample.clone());
    }
    now = now.saturating_add(Duration::from_secs(4));
    poll(&mut recorder, &mut segmenter, &trunk, now, &mut last_init);
    segmenter.flush();
    now = now.saturating_add(Duration::from_secs(4));
    poll(&mut recorder, &mut segmenter, &trunk, now, &mut last_init);

    let mut archive = FsArchive::open(
        &root,
        &ArchiveName::new("cam").expect("name"),
        ArchiveContainer::Fmp4,
    );
    let scan = archive.scan().expect("scan");
    assert!(scan.unreadable.is_empty(), "{:?}", scan.unreadable);
    let mut periods = Vec::new();
    for segment in &scan.segments {
        if periods.last() != Some(&segment.period) {
            periods.push(segment.period);
        }
    }
    Fmp4Recording {
        root,
        archive,
        periods,
    }
}

#[test]
fn a_forced_track_change_rolls_to_a_second_period() {
    let mut recording = record_into(temp_dir("fmp4"));
    assert!(
        recording.periods.len() >= 2,
        "the init change must roll to another period, got {:?}",
        recording.periods
    );
    let scan = recording.archive.scan().expect("scan");
    for period in &recording.periods {
        let path = recording.root.join("cam").join(format!("p{period}.m3u8"));
        let text = std::fs::read_to_string(&path).expect("playlist");
        MediaPlaylist::parse(&text).expect("a standard HLS playlist parses");
        assert_eq!(
            text.matches("#EXT-X-MAP:").count(),
            1,
            "period {period} has one init"
        );
        let parsed = parse_period_playlist(*period, ArchiveContainer::Fmp4, text.as_bytes())
            .expect("the archive playlist parses");
        assert!(!parsed.segments.is_empty());

        let first = parsed.segments[0];
        let init_span = first.init.expect("an fMP4 init");
        let mut bytes = recording
            .archive
            .read_init(*period, init_span)
            .expect("read the init")
            .to_vec();
        for segment in &parsed.segments {
            bytes.extend_from_slice(&recording.archive.read(segment).expect("read a segment"));
        }
        let media = test_harness::collect::media_from_mp4(
            &bytes,
            test_harness::collect::CollectMode::Seekable,
        )
        .unwrap_or_else(|e| panic!("period {period} must play on its own: {e}"));
        assert!(!media.tracks.is_empty(), "period {period} recovers a track");
        assert!(
            media.tracks.iter().all(|track| !track.samples.is_empty()),
            "period {period} recovers samples"
        );
        let total = scan.segments.iter().filter(|s| s.period == *period).count();
        assert_eq!(parsed.segments.len(), total);
    }
    cleanup(&recording.root);
}
