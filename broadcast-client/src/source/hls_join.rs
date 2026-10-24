//! Per-video-track live join state for the HLS source, the counterpart of the `LiveJoin` the TS-over-HTTP path drives from `buffer.rs`.

use alloc::collections::BTreeMap;

use transmux::{JoinDecision, LiveJoin, Sample, TrackId, TrackSpec};

#[derive(Default)]
pub(crate) struct HlsJoins {
    joins: BTreeMap<TrackId, LiveJoin>,
}

impl HlsJoins {
    pub(crate) fn on_tracks(&mut self, specs: &[TrackSpec]) {
        self.joins
            .retain(|id, _| specs.iter().any(|s| s.track_id == *id));
        for spec in specs {
            if let Some(join) = LiveJoin::for_config(&spec.config) {
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
