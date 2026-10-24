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
