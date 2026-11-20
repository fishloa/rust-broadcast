//! Per-video-track live join state for the HLS source, the counterpart of the `LiveJoin` the TS-over-HTTP path drives from `buffer.rs`.

use alloc::collections::BTreeMap;

use transmux::{JoinDecision, Sample, TrackId, TrackSpec};

use crate::join::VideoJoin;

#[derive(Default)]
pub(crate) struct HlsJoins {
    joins: BTreeMap<TrackId, VideoJoin>,
}

impl HlsJoins {
    pub(crate) fn on_tracks(&mut self, specs: &[TrackSpec]) {
        self.joins
            .retain(|id, _| specs.iter().any(|s| s.track_id == *id));
        for spec in specs {
            if let Some(join) = VideoJoin::for_config(&spec.config) {
                self.joins.entry(spec.track_id).or_insert(join);
            }
        }
    }

    pub(crate) fn rejoin(&mut self) {
        for j in self.joins.values_mut() {
            j.rejoin();
        }
    }

    pub(crate) fn push(&mut self, track: TrackId, sample: &Sample) -> Option<JoinDecision> {
        self.joins.get_mut(&track).map(|j| j.push(sample))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transmux::{DemuxEvent, DropReason, StreamingTsDemux};

    const FIXTURE: &str = "ts/h264-aac.ts";

    struct Real {
        spec: TrackSpec,
        rap: Sample,
        non_rap: Sample,
    }

    fn real_video_track() -> Real {
        let bytes = test_fixtures::read(FIXTURE);
        let mut demux = StreamingTsDemux::new();
        demux.feed(&bytes).expect("valid timing");
        let mut spec: Option<TrackSpec> = None;
        let mut rap: Option<Sample> = None;
        let mut non_rap: Option<Sample> = None;
        while let Some(event) = demux.poll_event() {
            match event {
                DemuxEvent::TrackAdded(s)
                    if spec.is_none() && VideoJoin::for_config(&s.config).is_some() =>
                {
                    spec = Some(s);
                }
                DemuxEvent::Sample {
                    track_id, sample, ..
                } if spec.as_ref().is_some_and(|s| s.track_id == track_id) => {
                    if sample.flags.is_sync && rap.is_none() {
                        rap = Some(sample);
                    } else if !sample.flags.is_sync && non_rap.is_none() {
                        non_rap = Some(sample);
                    }
                }
                _ => {}
            }
        }
        Real {
            spec: spec.expect("a video track in the fixture"),
            rap: rap.expect("a random access point in the fixture"),
            non_rap: non_rap.expect("a non-random-access sample in the fixture"),
        }
    }

    #[test]
    fn a_track_still_announced_keeps_its_join_and_a_dropped_one_is_forgotten() {
        let real = real_video_track();
        let track = real.spec.track_id;
        let mut joins = HlsJoins::default();
        joins.on_tracks(core::slice::from_ref(&real.spec));
        assert_eq!(
            joins.push(track, &real.non_rap),
            Some(JoinDecision::Drop(DropReason::BeforeRandomAccessPoint))
        );
        assert!(matches!(
            joins.push(track, &real.rap),
            Some(JoinDecision::JoinPoint(_))
        ));
        assert_eq!(joins.push(track, &real.non_rap), Some(JoinDecision::Keep));

        joins.on_tracks(core::slice::from_ref(&real.spec));
        assert_eq!(
            joins.push(track, &real.non_rap),
            Some(JoinDecision::Keep),
            "a re-announced track keeps the join it had"
        );

        joins.on_tracks(&[]);
        assert_eq!(joins.push(track, &real.non_rap), None);
    }

    #[test]
    fn rejoin_makes_the_next_sample_wait_for_a_random_access_point_again() {
        let real = real_video_track();
        let track = real.spec.track_id;
        let mut joins = HlsJoins::default();
        joins.on_tracks(core::slice::from_ref(&real.spec));
        assert!(matches!(
            joins.push(track, &real.rap),
            Some(JoinDecision::JoinPoint(_))
        ));
        assert_eq!(joins.push(track, &real.non_rap), Some(JoinDecision::Keep));
        joins.rejoin();
        assert_eq!(
            joins.push(track, &real.non_rap),
            Some(JoinDecision::Drop(DropReason::BeforeRandomAccessPoint))
        );
    }
}
