//! The host's explicit rendered-track declaration, resolved against the announced track set.
//!
//! A declared id that is announced renders as declared. A declared id that was announced and then
//! vanished (a PMT drop and restore returns the stream under a new id) is replaced by the lowest
//! announced track of its kind that is not itself declared. A declared id that was never announced
//! is pending: it is ignored until a later announcement names it, and reported once. A declared id
//! that vanished and has no replacement yet is waiting, not unknown, and is not reported.

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
                None if self.vanished.contains_key(id) => {}
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

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Unpackage;

    fn specs() -> (TrackSpec, TrackSpec) {
        let data = test_fixtures::read("ts/h264_aac.ts");
        let media = transmux::TsDemux::new().unpackage(&data[..]).unwrap();
        let mut video = None;
        let mut audio = None;
        for t in media.tracks {
            if t.spec.config.is_video() {
                video = Some(t.spec);
            } else if t.spec.config.audio_sample_rate().is_some() {
                audio = Some(t.spec);
            }
        }
        (video.unwrap(), audio.unwrap())
    }

    fn with_id(spec: &TrackSpec, id: u32) -> TrackSpec {
        let mut s = spec.clone();
        s.track_id = TrackId::new(id);
        s
    }

    fn ids(v: &[u32]) -> Vec<TrackId> {
        v.iter().map(|i| TrackId::new(*i)).collect()
    }

    #[test]
    fn no_declaration_leaves_the_default_policy() {
        let (v, a) = specs();
        let mut d = Declaration::default();
        let r = d.on_tracks(&[with_id(&v, 1), with_id(&a, 2)]);
        assert_eq!(r.rendered, None);
        assert!(!r.changed);
    }

    #[test]
    fn an_empty_declaration_renders_nothing_but_a_wholly_pending_one_falls_back() {
        let (v, a) = specs();
        let mut d = Declaration::default();
        d.on_tracks(&[with_id(&v, 1), with_id(&a, 2)]);
        assert_eq!(d.declare(&[]).rendered, Some(Vec::new()));
        let r = d.declare(&ids(&[999]));
        assert_eq!(r.rendered, None);
        assert_eq!(r.unknown, ids(&[999]));
    }

    #[test]
    fn unknown_ids_are_reported_once_per_declaration_and_only_after_an_announcement() {
        let (v, a) = specs();
        let mut d = Declaration::default();
        assert!(d.declare(&ids(&[1, 999])).unknown.is_empty());
        let r = d.on_tracks(&[with_id(&v, 1), with_id(&a, 2)]);
        assert_eq!(r.rendered, Some(ids(&[1])));
        assert_eq!(r.unknown, ids(&[999]));
        let again = d.on_tracks(&[with_id(&v, 1), with_id(&a, 2)]);
        assert!(again.unknown.is_empty());
        assert_eq!(d.declare(&ids(&[1, 999])).unknown, ids(&[999]));
    }

    #[test]
    fn a_pending_id_applies_when_it_is_announced() {
        let (v, a) = specs();
        let mut d = Declaration::default();
        d.on_tracks(&[with_id(&v, 1)]);
        assert_eq!(d.declare(&ids(&[1, 7])).rendered, Some(ids(&[1])));
        let r = d.on_tracks(&[with_id(&v, 1), with_id(&a, 7)]);
        assert_eq!(r.rendered, Some(ids(&[1, 7])));
    }

    #[test]
    fn a_vanished_declared_track_is_replaced_by_the_same_kind_and_waits_silently_meanwhile() {
        let (v, a) = specs();
        let mut d = Declaration::default();
        d.on_tracks(&[with_id(&v, 1), with_id(&a, 2)]);
        d.declare(&ids(&[1, 2]));
        let gone = d.on_tracks(&[with_id(&v, 1)]);
        assert_eq!(gone.rendered, Some(ids(&[1])));
        assert!(gone.unknown.is_empty());
        let back = d.on_tracks(&[with_id(&v, 1), with_id(&a, 3)]);
        assert_eq!(back.rendered, Some(ids(&[1, 3])));
    }

    #[test]
    fn a_replacement_is_never_a_track_the_host_declared_itself() {
        let (v, a) = specs();
        let mut d = Declaration::default();
        d.on_tracks(&[with_id(&v, 1), with_id(&a, 2), with_id(&a, 3)]);
        d.declare(&ids(&[1, 2, 3]));
        d.on_tracks(&[with_id(&v, 1), with_id(&a, 3)]);
        let r = d.on_tracks(&[with_id(&v, 1), with_id(&a, 3), with_id(&a, 4)]);
        assert_eq!(r.rendered, Some(ids(&[1, 4, 3])));
    }

    #[test]
    fn the_original_id_wins_when_it_returns() {
        let (v, a) = specs();
        let mut d = Declaration::default();
        d.on_tracks(&[with_id(&v, 1), with_id(&a, 2)]);
        d.declare(&ids(&[1, 2]));
        d.on_tracks(&[with_id(&v, 1), with_id(&a, 3)]);
        let r = d.on_tracks(&[with_id(&v, 1), with_id(&a, 2), with_id(&a, 3)]);
        assert_eq!(r.rendered, Some(ids(&[1, 2])));
    }

    #[test]
    fn a_video_id_is_never_replaced_by_an_audio_track() {
        let (v, a) = specs();
        let mut d = Declaration::default();
        d.on_tracks(&[with_id(&v, 1), with_id(&a, 2)]);
        d.declare(&ids(&[1]));
        let r = d.on_tracks(&[with_id(&a, 2)]);
        assert_eq!(r.rendered, None);
    }
}
