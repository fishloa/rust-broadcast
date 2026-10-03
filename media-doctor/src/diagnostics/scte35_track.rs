//! The one SCTE-35 `splice_insert` tracker shared by [`Scte35Check`] (batch
//! `check`) and `watch` (streaming `media-doctor watch`).
//!
//! Both reassembled `splice_info_section`s on a PMT-declared PID, parsed the
//! `splice_insert`, skipped cancels, and ran the same open/closed state
//! machine — two copies that drifted (the `check` copy was fixed for the PID
//! discovery of #1046 and for `auto_return` while `watch` kept its own; audit
//! #1141). [`SpliceTracker`] is the single implementation of that pipeline:
//! callers supply only what they do with each observed event.
//!
//! State machine (ANSI/SCTE 35 §9.8.2, §9.9.2.2): an "out"
//! (`out_of_network_indicator`) without `break_duration.auto_return` opens its
//! `splice_event_id`; a matching "in" closes it; an "out" carrying
//! `auto_return` closes itself at its own duration and so is never left open.
//! The tracker stores only the *currently open* ids, so it cannot grow over a
//! long run (audit MD-W3).
//!
//! [`Scte35Check`]: crate::Scte35Check

use alloc::collections::BTreeSet;

use broadcast_common::Parse;
use mpeg_ts::ts::SectionReassembler;

/// `table_id` of a SCTE-35 `splice_info_section` (ANSI/SCTE 35 §9.6.1).
const SCTE35_TABLE_ID: u8 = 0xFC;

/// What one non-cancelled `splice_insert` did to the open set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SpliceObservation {
    /// The event's `splice_event_id`.
    pub(crate) event_id: u32,
    /// An "out" arrived while the same event was already open with no
    /// intervening "in".
    pub(crate) duplicate_open: bool,
}

/// Section reassembly + open-event tracking for one SCTE-35 PID.
#[derive(Default)]
pub(crate) struct SpliceTracker {
    reassembler: SectionReassembler,
    open: BTreeSet<u32>,
}

impl SpliceTracker {
    /// Feed one TS packet payload of this PID; `on_event` is called once per
    /// complete, non-cancelled `splice_insert` section it finishes.
    pub(crate) fn feed(
        &mut self,
        payload: &[u8],
        pusi: bool,
        mut on_event: impl FnMut(SpliceObservation),
    ) {
        self.reassembler.feed(payload, pusi);
        while let Some(section) = self.reassembler.pop_section() {
            if section.first() != Some(&SCTE35_TABLE_ID) {
                continue;
            }
            let Ok(sis) = scte35_splice::SpliceInfoSection::parse(&section[..]) else {
                continue;
            };
            let Some(ref clear) = sis.clear else {
                continue;
            };
            let scte35_splice::commands::AnyCommand::SpliceInsert(si) = &clear.command else {
                continue;
            };
            // A cancel names an event and neither opens nor closes a splice.
            if si.splice_event_cancel_indicator {
                continue;
            }
            let auto_return = si
                .break_duration
                .is_some_and(|break_duration| break_duration.auto_return);
            let duplicate_open =
                self.observe(si.splice_event_id, si.out_of_network_indicator, auto_return);
            on_event(SpliceObservation {
                event_id: si.splice_event_id,
                duplicate_open,
            });
        }
    }

    /// Apply one "out"/"in" to the open set; `true` when it is a duplicate
    /// open "out".
    fn observe(&mut self, event_id: u32, out: bool, auto_return: bool) -> bool {
        if self.open.contains(&event_id) {
            if out {
                if auto_return {
                    // The duplicate closes at its own duration.
                    self.open.remove(&event_id);
                }
                true
            } else {
                // Matching "in".
                self.open.remove(&event_id);
                false
            }
        } else {
            if out && !auto_return {
                self.open.insert(event_id);
            }
            false
        }
    }

    /// Number of currently open (unmatched) events.
    pub(crate) fn open_count(&self) -> usize {
        self.open.len()
    }

    /// The currently open event ids, ascending.
    pub(crate) fn open_events(&self) -> impl Iterator<Item = u32> + '_ {
        self.open.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> SpliceTracker {
        SpliceTracker::default()
    }

    /// The state machine both `Scte35Check` and `watch` now share, pinned
    /// against the table each pre-consolidation copy implemented.
    #[test]
    fn out_then_in_is_balanced() {
        let mut t = tracker();
        assert!(!t.observe(1, true, false));
        assert_eq!(t.open_events().collect::<alloc::vec::Vec<_>>(), [1]);
        assert!(!t.observe(1, false, false));
        assert_eq!(t.open_count(), 0);
    }

    #[test]
    fn duplicate_out_is_flagged_and_stays_open() {
        let mut t = tracker();
        t.observe(7, true, false);
        assert!(t.observe(7, true, false));
        assert_eq!(t.open_count(), 1);
    }

    #[test]
    fn auto_return_out_is_never_left_open() {
        let mut t = tracker();
        assert!(!t.observe(2, true, true));
        assert_eq!(t.open_count(), 0);
        // ...and a duplicate auto-return out on an open event closes it.
        t.observe(3, true, false);
        assert!(t.observe(3, true, true));
        assert_eq!(t.open_count(), 0);
    }

    #[test]
    fn standalone_in_and_reopen_after_close() {
        let mut t = tracker();
        assert!(!t.observe(4, false, false));
        assert_eq!(t.open_count(), 0);
        t.observe(4, true, false);
        t.observe(4, false, false);
        assert!(!t.observe(4, true, false));
        assert_eq!(t.open_count(), 1);
    }
}
