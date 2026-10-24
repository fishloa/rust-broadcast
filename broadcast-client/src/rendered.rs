//! The host's explicit rendered-track declaration, resolved against the announced track set.
//!
//! A declared id that is announced renders as declared. A declared id that was announced and then
//! vanished (a PMT drop and restore returns the stream under a new id) is replaced by the lowest
//! announced track of its kind that is not itself declared. A declared id that was never announced
//! is pending: it is ignored until a later announcement names it, and reported once.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use transmux::{TrackId, TrackSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Video,
    Audio,
}

fn kind_of(spec: &TrackSpec) -> Option<Kind> {
    if spec.config.is_video() {
        Some(Kind::Video)
    } else if spec.config.audio_sample_rate().is_some() {
        Some(Kind::Audio)
    } else {
        None
    }
}

#[derive(Debug, Default)]
pub(crate) struct Resolution {
    pub(crate) changed: bool,
    pub(crate) rendered: Option<Vec<TrackId>>,
    pub(crate) unknown: Vec<TrackId>,
}

#[derive(Default)]
pub(crate) struct Declaration {
    declared: Option<Vec<TrackId>>,
    announced: BTreeMap<TrackId, Option<Kind>>,
    vanished: BTreeMap<TrackId, Kind>,
    reported: BTreeSet<TrackId>,
    announced_once: bool,
    applied: Option<Vec<TrackId>>,
}

impl Declaration {
    pub(crate) fn declare(&mut self, tracks: &[TrackId]) -> Resolution {
        let mut seen = BTreeSet::new();
        self.declared = Some(tracks.iter().copied().filter(|t| seen.insert(*t)).collect());
        self.vanished.clear();
        self.reported.clear();
        self.resolve()
    }

    pub(crate) fn on_tracks(&mut self, specs: &[TrackSpec]) -> Resolution {
        let next: BTreeMap<TrackId, Option<Kind>> =
            specs.iter().map(|s| (s.track_id, kind_of(s))).collect();
        if let Some(declared) = &self.declared {
            for (id, kind) in &self.announced {
                if let (false, true, Some(kind)) =
                    (next.contains_key(id), declared.contains(id), kind)
                {
                    self.vanished.insert(*id, *kind);
                }
            }
        }
        self.vanished.retain(|id, _| !next.contains_key(id));
        self.announced = next;
        self.announced_once = true;
        self.resolve()
    }

    fn resolve(&mut self) -> Resolution {
        let Some(declared) = &self.declared else {
            return self.settle(None, Vec::new());
        };
        let mut picked: Vec<TrackId> = Vec::new();
        let mut unknown: Vec<TrackId> = Vec::new();
        for id in declared {
            if self.announced.contains_key(id) {
                picked.push(*id);
                continue;
            }
            let replacement = self.vanished.get(id).and_then(|kind| {
                self.announced
                    .iter()
                    .find(|(cand, k)| {
                        **k == Some(*kind) && !declared.contains(cand) && !picked.contains(cand)
                    })
                    .map(|(cand, _)| *cand)
            });
            match replacement {
                Some(cand) => picked.push(cand),
                None => unknown.push(*id),
            }
        }
        let pending_only = !declared.is_empty() && picked.is_empty();
        let unknown: Vec<TrackId> = if self.announced_once {
            unknown
                .into_iter()
                .filter(|id| self.reported.insert(*id))
                .collect()
        } else {
            Vec::new()
        };
        self.settle((!pending_only).then_some(picked), unknown)
    }

    fn settle(&mut self, rendered: Option<Vec<TrackId>>, unknown: Vec<TrackId>) -> Resolution {
        let changed = self.declared.is_some() || self.applied != rendered;
        self.applied.clone_from(&rendered);
        Resolution {
            changed,
            rendered,
            unknown,
        }
    }
}
