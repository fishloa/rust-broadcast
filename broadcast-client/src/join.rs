//! Live join for every video codec: the NAL-aware [`LiveJoin`] for AVC, HEVC and VVC, and a
//! sync-sample join for the codecs whose random-access points the container already flags
//! (MPEG-2 video, AV1, VP9, VP8).

use transmux::{CodecConfig, DropReason, JoinDecision, LiveJoin, RapKind, Sample};

#[derive(Debug, Clone)]
pub(crate) enum VideoJoin {
    Nal(LiveJoin),
    Sync(SyncJoin),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SyncJoin {
    joined: bool,
    join_pts: Option<i64>,
    leading_run: bool,
}

impl SyncJoin {
    fn push(&mut self, sample: &Sample) -> JoinDecision {
        if !self.joined {
            if !sample.flags.is_sync {
                return JoinDecision::Drop(DropReason::BeforeRandomAccessPoint);
            }
            self.joined = true;
            self.join_pts = sample.pts;
            self.leading_run = true;
            return JoinDecision::JoinPoint(RapKind::Idr);
        }
        if self.leading_run {
            let leading = matches!((sample.pts, self.join_pts), (Some(p), Some(j)) if p < j);
            if leading {
                return JoinDecision::Drop(DropReason::LeadingPicture);
            }
            self.leading_run = false;
        }
        JoinDecision::Keep
    }

    fn rejoin(&mut self) {
        *self = Self::default();
    }
}

impl VideoJoin {
    pub(crate) fn for_config(config: &CodecConfig) -> Option<Self> {
        if let Some(join) = LiveJoin::for_config(config) {
            return Some(Self::Nal(join));
        }
        config.is_video().then(|| Self::Sync(SyncJoin::default()))
    }

    pub(crate) fn push(&mut self, sample: &Sample) -> JoinDecision {
        match self {
            Self::Nal(j) => j.push(sample),
            Self::Sync(j) => j.push(sample),
        }
    }

    pub(crate) fn rejoin(&mut self) {
        match self {
            Self::Nal(j) => j.rejoin(),
            Self::Sync(j) => j.rejoin(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Unpackage;
    use transmux::{Fmp4Demux, Track};

    fn video_track(fixture: &str) -> Track {
        let data = test_fixtures::read(fixture);
        let media = Fmp4Demux::new().unpackage(&data[..]).unwrap();
        media
            .tracks
            .into_iter()
            .find(|t| t.spec.config.is_video())
            .unwrap()
    }

    fn assert_syncs_join(fixture: &str) {
        let track = video_track(fixture);
        let mut join = VideoJoin::for_config(&track.spec.config).expect("a video join");
        assert!(matches!(join, VideoJoin::Sync(_)));
        assert!(
            track.samples.iter().filter(|s| s.flags.is_sync).count() >= 1,
            "fixture has no sync sample"
        );
        let decisions: Vec<JoinDecision> = track.samples.iter().map(|s| join.push(s)).collect();
        let first_kept = decisions.iter().position(JoinDecision::is_keep).unwrap();
        assert!(track.samples[first_kept].flags.is_sync);
        assert!(matches!(decisions[first_kept], JoinDecision::JoinPoint(_)));
        assert!(
            decisions[..first_kept]
                .iter()
                .all(|d| matches!(d, JoinDecision::Drop(DropReason::BeforeRandomAccessPoint)))
        );
        let joins = decisions
            .iter()
            .filter(|d| matches!(d, JoinDecision::JoinPoint(_)))
            .count();
        assert_eq!(joins, 1);
        assert!(decisions[first_kept + 1..].iter().all(|d| matches!(
            d,
            JoinDecision::Keep | JoinDecision::Drop(DropReason::LeadingPicture)
        )));
    }

    #[test]
    fn av1_joins_at_its_first_sync_sample() {
        assert_syncs_join("mp4/frag/av1.frag.mp4");
    }

    #[test]
    fn vp9_joins_at_its_first_sync_sample() {
        assert_syncs_join("mp4/frag/vp9.frag.mp4");
    }

    #[test]
    fn a_sync_join_waits_for_the_next_sync_sample_after_rejoin() {
        let track = video_track("mp4/frag/vp9.frag.mp4");
        let mut join = VideoJoin::for_config(&track.spec.config).unwrap();
        let sync = track.samples.iter().find(|s| s.flags.is_sync).unwrap();
        let plain = track.samples.iter().find(|s| !s.flags.is_sync).unwrap();
        assert!(matches!(join.push(sync), JoinDecision::JoinPoint(_)));
        join.rejoin();
        assert_eq!(
            join.push(plain),
            JoinDecision::Drop(DropReason::BeforeRandomAccessPoint)
        );
        assert!(matches!(join.push(sync), JoinDecision::JoinPoint(_)));
    }

    #[test]
    fn avc_keeps_the_nal_aware_join() {
        let data = test_fixtures::read("ts/h264_aac.ts");
        let media = transmux::TsDemux::new().unpackage(&data[..]).unwrap();
        let video = media
            .tracks
            .iter()
            .find(|t| t.spec.config.is_video())
            .unwrap();
        assert!(matches!(
            VideoJoin::for_config(&video.spec.config),
            Some(VideoJoin::Nal(_))
        ));
    }
}
