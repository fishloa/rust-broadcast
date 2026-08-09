//! Transition planning: given a [`Schedule`] and
//! the current channel-timeline position, what transition happens next, at
//! what instant, and what timeline-continuity actions it implies.
//!
//! Issue [#748](https://github.com/fishloa/rust-broadcast/issues/748)'s
//! framing: **the hard part is the transition, not the schedule.** Joining
//! two sources means timeline continuity across the join — a PTS-rebase
//! offset so the incoming source's own clock lands continuously on the
//! shared channel timeline (the parameter `transmux`'s IR transform would
//! consume to actually rewrite sample timestamps — this crate computes the
//! number, it does not touch sample bytes), plus a discontinuity flag when
//! the codec configuration changes across the join (the signal
//! `broadcast_hls::mark_init_discontinuities` would act on downstream). "A
//! schedule that plays the right thing at the wrong timestamp is worse than
//! no scheduler" — so [`TransitionPlan`] is the primary thing this crate's
//! tests hold to timeline-correctness, not the schedule bookkeeping.

use crate::schedule::{Schedule, ScheduleEntry};

/// The computed plan for joining `from` into `to` at [`Self::at_pts`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionPlan {
    /// The channel-timeline instant the join happens at — `to`'s
    /// `planned_start`.
    pub at_pts: u64,
    /// Add this to one of `to`'s own PTS values to place it on the shared
    /// channel timeline (see [`Self::rebase`]). `i128` so it can hold the
    /// full possible difference of two `u64` instants without clamping —
    /// this crate never silently narrows a rebase computation. May be
    /// negative when `to`'s own clock numbering starts after the channel
    /// instant it is joining at (an unusual but not-impossible schedule).
    pub pts_rebase_offset: i128,
    /// Whether `from` and `to` carry different
    /// [`CodecConfigId`](crate::schedule::CodecConfigId)s — a discontinuity
    /// that must be signalled downstream (init-segment change / new codec
    /// config), never something this crate transcodes or conforms away.
    pub discontinuity: bool,
}

impl TransitionPlan {
    /// Plan the transition from `from` into `to`. `to.planned_start` is the
    /// channel-timeline instant the join happens at; the rebase offset is
    /// derived so that `to.source_start_pts`, once rebased, lands exactly on
    /// that instant.
    #[must_use]
    pub fn plan(from: &ScheduleEntry, to: &ScheduleEntry) -> Self {
        let at_pts = to.planned_start;
        let pts_rebase_offset = i128::from(at_pts) - i128::from(to.source_start_pts);
        TransitionPlan {
            at_pts,
            pts_rebase_offset,
            discontinuity: from.codec_config != to.codec_config,
        }
    }

    /// Map one of `to`'s own PTS values onto the shared channel timeline
    /// (`source_pts + pts_rebase_offset`).
    ///
    /// Returns `None` rather than wrapping if the rebased instant would fall
    /// outside `u64`'s range (e.g. a source PTS before the source's own
    /// declared start, rebasing to before the join instant) — this crate
    /// never silently produces a timeline instant that doesn't correspond to
    /// the input.
    #[must_use]
    pub fn rebase(&self, source_pts: u64) -> Option<u64> {
        let rebased = i128::from(source_pts) + self.pts_rebase_offset;
        u64::try_from(rebased).ok()
    }
}

/// A planned transition together with the two entries either side of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannedTransition<'a> {
    /// The entry currently playing.
    pub from: &'a ScheduleEntry,
    /// The entry the channel joins into.
    pub to: &'a ScheduleEntry,
    /// The computed plan for that join.
    pub plan: TransitionPlan,
}

/// Find and plan the next transition due at or after `now_pts`.
///
/// Returns `None` if `now_pts` precedes every entry, or if the entry active
/// at `now_pts` is the schedule's last entry (nothing scheduled to follow it
/// yet).
#[must_use]
pub fn next_transition(schedule: &Schedule, now_pts: u64) -> Option<PlannedTransition<'_>> {
    let idx = schedule.active_index_at(now_pts)?;
    let entries = schedule.entries();
    let to = entries.get(idx + 1)?;
    let from = &entries[idx];
    Some(PlannedTransition {
        from,
        to,
        plan: TransitionPlan::plan(from, to),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::{CodecConfigId, EntryKind, ScheduleEntry};

    fn entry(
        id: &str,
        kind: EntryKind,
        planned_start: u64,
        source_start_pts: u64,
        codec_config: u64,
    ) -> ScheduleEntry {
        ScheduleEntry {
            id: id.into(),
            kind,
            planned_start,
            source_start_pts,
            codec_config: CodecConfigId(codec_config),
        }
    }

    /// The primary property under test: continuity across a join. Once
    /// rebased, the incoming source's own PTS sequence must land exactly on
    /// the channel timeline with no gap and no overlap — `rebase(start) ==
    /// at_pts`, and every later source PTS advances the channel timeline by
    /// exactly the same amount it advanced on the source's own clock.
    #[test]
    fn rebase_lands_the_incoming_source_continuously_on_the_channel_timeline() {
        let programme = entry("prog-1", EntryKind::Programme, 0, 0, 1);
        // The ad's own asset timeline starts at PTS 5_000 (e.g. it is itself
        // a clip cut from a longer source), and is scheduled to join the
        // channel at channel-timeline instant 1_000_000.
        let ad = entry("ad-1", EntryKind::Ad, 1_000_000, 5_000, 1);

        let plan = TransitionPlan::plan(&programme, &ad);
        assert_eq!(plan.at_pts, 1_000_000);

        // The ad's very first sample lands exactly at the join instant.
        assert_eq!(plan.rebase(5_000), Some(1_000_000));
        // One second later (90 kHz) on the ad's own clock is one second
        // later on the channel timeline too — no drift introduced by the
        // rebase itself.
        assert_eq!(plan.rebase(5_000 + 90_000), Some(1_090_000));
        // Ten seconds later.
        assert_eq!(plan.rebase(5_000 + 900_000), Some(1_900_000));
    }

    #[test]
    fn rebase_offset_can_be_negative_when_source_starts_after_the_join_instant() {
        // A degenerate but legal schedule: the source's own clock numbering
        // starts past the channel instant it joins at.
        let from = entry("prog-1", EntryKind::Programme, 0, 0, 1);
        let to = entry("ad-1", EntryKind::Ad, 100, 500, 1);
        let plan = TransitionPlan::plan(&from, &to);
        assert_eq!(plan.pts_rebase_offset, -400);
        assert_eq!(plan.rebase(500), Some(100));
        // A source PTS below its own declared start rebases to a negative
        // channel instant — reported as `None`, never wrapped.
        assert_eq!(plan.rebase(300), None);
    }

    #[test]
    fn rebase_returns_none_rather_than_wrapping_out_of_range() {
        let from = entry("prog-1", EntryKind::Programme, 0, 0, 1);
        let to = entry("ad-1", EntryKind::Ad, 0, u64::MAX, 1);
        let plan = TransitionPlan::plan(&from, &to);

        // The source's own declared start rebases exactly onto the join
        // instant, even though the raw offset is a huge negative i128.
        assert_eq!(plan.rebase(u64::MAX), Some(0));
        // Anything before the source's own start would rebase to a negative
        // channel instant — reported honestly as `None`, not wrapped into a
        // huge bogus `u64`.
        assert_eq!(plan.rebase(0), None);
    }

    #[test]
    fn discontinuity_flags_a_codec_config_change_across_the_join() {
        let from = entry("prog-1", EntryKind::Programme, 0, 0, 1);
        let same = entry("prog-2", EntryKind::Programme, 1_000, 0, 1);
        let different = entry("ad-1", EntryKind::Ad, 1_000, 0, 2);

        assert!(!TransitionPlan::plan(&from, &same).discontinuity);
        assert!(TransitionPlan::plan(&from, &different).discontinuity);
    }

    #[test]
    fn next_transition_walks_the_schedule() {
        let mut sched = Schedule::default();
        sched
            .push(entry("prog-1", EntryKind::Programme, 0, 0, 1))
            .unwrap();
        sched
            .push(entry("ad-1", EntryKind::Ad, 1_000, 0, 2))
            .unwrap();
        sched
            .push(entry("prog-2", EntryKind::Programme, 2_000, 0, 1))
            .unwrap();

        let t = next_transition(&sched, 500).unwrap();
        assert_eq!(t.from.id, "prog-1");
        assert_eq!(t.to.id, "ad-1");
        assert!(t.plan.discontinuity);

        let t = next_transition(&sched, 1_500).unwrap();
        assert_eq!(t.from.id, "ad-1");
        assert_eq!(t.to.id, "prog-2");

        // No entry follows the last one yet.
        assert!(next_transition(&sched, 2_500).is_none());
        // pts 0 IS prog-1's start, so the next transition is still found.
        assert!(next_transition(&sched, 0).is_some());
    }
}
