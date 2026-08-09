//! The channel schedule: an ordered list of entries (programme / ad / slate).
//!
//! The wire/data format here is deliberately **ours to define** — issue
//! [#748](https://github.com/fishloa/rust-broadcast/issues/748)'s
//! design-decision comment: SCTE 224 (ESNI) covers policy/blackout
//! signalling, not channel assembly, and adopting it would be cargo-culting a
//! standard that does not answer this question. There is no applicable open
//! standard for "ordered list of sources with planned starts" to transcribe,
//! so [`Schedule`] is a plain in-memory model, not a wire type: it has no
//! `Parse`/`Serialize` pair, and it is deliberately minimal — enough to plan
//! [transitions](crate::transition) and the SCTE-35 cues they imply, nothing
//! about rights, geography, or blackout rules.

use crate::error::{Error, Result};
use alloc::string::String;
use alloc::vec::Vec;

/// What kind of content a [`ScheduleEntry`] carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum EntryKind {
    /// Regular programme content.
    Programme,
    /// An advertisement — a scheduled ad-break asset.
    Ad,
    /// Filler/slate content played when nothing else is scheduled (e.g. a
    /// hold slide, or gap-filler between programme and ad).
    Slate,
}

impl EntryKind {
    /// Stable label.
    pub fn name(&self) -> &'static str {
        match self {
            EntryKind::Programme => "programme",
            EntryKind::Ad => "ad",
            EntryKind::Slate => "slate",
        }
    }
}
broadcast_common::impl_spec_display!(EntryKind);

/// Caller-supplied opaque identity for a source's codec configuration (its
/// `avcC`/`hvcC`/`esds`, etc.), used only for equality comparison.
///
/// This crate never parses codec-configuration bitstreams — the "no
/// transcoding or conforming" non-goal in issue #748's design decision — so
/// it cannot compute this itself. The caller (whatever demuxed the source,
/// e.g. `transmux`'s init-segment parsing) computes a stable fingerprint of
/// its own config and passes it here; a change in fingerprint between
/// consecutive entries is exactly what
/// [`TransitionPlan::discontinuity`](crate::transition::TransitionPlan::discontinuity)
/// treats as a discontinuity to signal downstream (e.g. via
/// `broadcast_hls::mark_init_discontinuities`) — never something this crate
/// re-encodes or conforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CodecConfigId(pub u64);

/// One entry in the schedule: a source, its kind, and when it is planned to
/// start on the shared channel timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScheduleEntry {
    /// Caller-chosen identity for the source this entry plays (e.g. an asset
    /// id or URI) — opaque to this crate beyond equality/display.
    pub id: String,
    /// What kind of content this entry is.
    pub kind: EntryKind,
    /// The channel-timeline instant (caller's clock unit — SCTE-35 cues use
    /// 90 kHz ticks, so that's the natural choice) this entry is scheduled to
    /// start at.
    pub planned_start: u64,
    /// The PTS value the source's own first presented sample carries, in the
    /// same clock unit. Needed to compute the PTS-rebase offset at the
    /// transition into this entry — see [`crate::transition`].
    pub source_start_pts: u64,
    /// Codec-config identity, for discontinuity detection at the transition
    /// into this entry.
    pub codec_config: CodecConfigId,
}

/// An ordered schedule: entries strictly increasing by `planned_start`.
///
/// This crate does not sort entries for you — [`Schedule::push`] rejects an
/// out-of-order entry immediately, so a caller building a schedule sees
/// ordering mistakes at construction time rather than as a silently
/// misordered playout later.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Schedule {
    entries: Vec<ScheduleEntry>,
}

impl Schedule {
    /// An empty schedule.
    #[must_use]
    pub fn new() -> Self {
        Schedule {
            entries: Vec::new(),
        }
    }

    /// Append `entry`. Errors with [`Error::OutOfOrder`] if `entry` does not
    /// start strictly after the schedule's current last entry.
    pub fn push(&mut self, entry: ScheduleEntry) -> Result<()> {
        if let Some(last) = self.entries.last()
            && entry.planned_start <= last.planned_start
        {
            return Err(Error::OutOfOrder {
                prev: last.planned_start,
                next: entry.planned_start,
            });
        }
        self.entries.push(entry);
        Ok(())
    }

    /// The entries, in schedule order.
    #[must_use]
    pub fn entries(&self) -> &[ScheduleEntry] {
        &self.entries
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the schedule has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entry active at `pts` — the last entry whose `planned_start <=
    /// pts` — or `None` if `pts` precedes every entry (or the schedule is
    /// empty).
    #[must_use]
    pub fn active_at(&self, pts: u64) -> Option<&ScheduleEntry> {
        self.active_index_at(pts).map(|i| &self.entries[i])
    }

    /// Index form of [`Self::active_at`], for [`crate::transition`] to also
    /// reach the *next* entry without a second lookup.
    pub(crate) fn active_index_at(&self, pts: u64) -> Option<usize> {
        if self.entries.is_empty() {
            return None;
        }
        let idx = self.entries.partition_point(|e| e.planned_start <= pts);
        if idx == 0 { None } else { Some(idx - 1) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, kind: EntryKind, planned_start: u64) -> ScheduleEntry {
        ScheduleEntry {
            id: id.into(),
            kind,
            planned_start,
            source_start_pts: 0,
            codec_config: CodecConfigId(1),
        }
    }

    #[test]
    fn push_accepts_strictly_increasing_starts() {
        let mut sched = Schedule::new();
        sched
            .push(entry("prog-1", EntryKind::Programme, 0))
            .unwrap();
        sched.push(entry("ad-1", EntryKind::Ad, 1_000)).unwrap();
        assert_eq!(sched.len(), 2);
    }

    #[test]
    fn push_rejects_non_increasing_start() {
        let mut sched = Schedule::new();
        sched
            .push(entry("prog-1", EntryKind::Programme, 1_000))
            .unwrap();
        let err = sched.push(entry("ad-1", EntryKind::Ad, 1_000)).unwrap_err();
        match err {
            Error::OutOfOrder { prev, next } => {
                assert_eq!(prev, 1_000);
                assert_eq!(next, 1_000);
            }
            other => panic!("expected OutOfOrder, got {other:?}"),
        }

        let err = sched.push(entry("ad-2", EntryKind::Ad, 500)).unwrap_err();
        assert!(matches!(
            err,
            Error::OutOfOrder {
                prev: 1_000,
                next: 500
            }
        ));
    }

    #[test]
    fn active_at_finds_the_last_entry_starting_at_or_before_pts() {
        let mut sched = Schedule::new();
        sched
            .push(entry("prog-1", EntryKind::Programme, 0))
            .unwrap();
        sched.push(entry("ad-1", EntryKind::Ad, 1_000)).unwrap();
        sched
            .push(entry("prog-2", EntryKind::Programme, 2_000))
            .unwrap();

        assert_eq!(sched.active_at(0).unwrap().id, "prog-1");
        assert_eq!(sched.active_at(999).unwrap().id, "prog-1");
        assert_eq!(sched.active_at(1_000).unwrap().id, "ad-1");
        assert_eq!(sched.active_at(1_999).unwrap().id, "ad-1");
        assert_eq!(sched.active_at(2_000).unwrap().id, "prog-2");
        assert_eq!(sched.active_at(1_000_000).unwrap().id, "prog-2");
    }

    #[test]
    fn active_at_before_the_first_entry_is_none() {
        let mut sched = Schedule::new();
        sched
            .push(entry("prog-1", EntryKind::Programme, 1_000))
            .unwrap();
        assert!(sched.active_at(999).is_none());
    }

    #[test]
    fn empty_schedule_has_no_active_entry() {
        let sched = Schedule::new();
        assert!(sched.is_empty());
        assert!(sched.active_at(0).is_none());
    }

    #[test]
    fn label_convention() {
        assert_eq!(EntryKind::Programme.name(), "programme");
        assert_eq!(alloc::format!("{}", EntryKind::Ad), "ad");
        assert_eq!(EntryKind::Slate.name(), "slate");
    }
}
