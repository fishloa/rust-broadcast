//! SCTE-35 `splice_insert` open-event tracker shared by media-doctor and
//! compliance tooling.
//!
//! State machine (ANSI/SCTE 35 §9.8.2, §9.9.2.2): an "out"
//! (`out_of_network_indicator`) without `break_duration.auto_return` opens its
//! `splice_event_id`; a matching "in" closes it; an "out" carrying
//! `auto_return` closes itself at its own duration and so is never left open.
//! The tracker stores only the *currently open* ids, capped at
//! [`MAX_OPEN_EVENTS`], so it cannot grow over a long run even on an out-only
//! feed (audit MD-W3).
//!
//! A section byte-identical to the previous one on the PID (same CRC) is a
//! retransmission, not a second "out": it never raises a duplicate.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::commands::AnyCommand;
use crate::section::SpliceInfoSection;
use broadcast_common::Parse;
use mpeg_ts::ts::SectionReassembler;

/// `table_id` of a SCTE-35 `splice_info_section` (ANSI/SCTE 35 §9.6.1).
const SCTE35_TABLE_ID: u8 = 0xFC;

/// Upper bound on simultaneously tracked open events.
pub const MAX_OPEN_EVENTS: usize = 1024;

/// What one non-cancelled `splice_insert` did to the open set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpliceObservation {
    /// The event's `splice_event_id`.
    pub event_id: u32,
    /// An "out" arrived while the same event was already open with no
    /// intervening "in".
    pub duplicate_open: bool,
}

/// Section reassembly + open-event tracking for one SCTE-35 PID.
#[derive(Default)]
pub struct SpliceTracker {
    reassembler: SectionReassembler,
    open: BTreeSet<u32>,
    last_section: Vec<u8>,
}

impl SpliceTracker {
    /// Feed one TS packet payload of this PID; `on_event` is called once per
    /// complete, non-cancelled `splice_insert` section it finishes.
    pub fn feed(
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
            let Ok(sis) = SpliceInfoSection::parse(&section[..]) else {
                continue;
            };
            let Some(ref clear) = sis.clear else {
                continue;
            };
            let AnyCommand::SpliceInsert(si) = &clear.command else {
                continue;
            };
            if si.splice_event_cancel_indicator {
                continue;
            }
            let retransmission =
                !self.last_section.is_empty() && section[..] == self.last_section[..];
            self.last_section.clear();
            self.last_section.extend_from_slice(&section[..]);
            let auto_return = si
                .break_duration
                .is_some_and(|break_duration| break_duration.auto_return);
            let duplicate_open = !retransmission
                && self.observe(si.splice_event_id, si.out_of_network_indicator, auto_return);
            on_event(SpliceObservation {
                event_id: si.splice_event_id,
                duplicate_open,
            });
        }
    }

    /// Apply one "out"/"in" to the open set; `true` when it is a duplicate
    /// open "out".
    pub fn observe(&mut self, event_id: u32, out: bool, auto_return: bool) -> bool {
        if self.open.contains(&event_id) {
            if out {
                if auto_return {
                    self.open.remove(&event_id);
                }
                true
            } else {
                self.open.remove(&event_id);
                false
            }
        } else {
            if out && !auto_return {
                self.open.insert(event_id);
                if self.open.len() > MAX_OPEN_EVENTS {
                    self.open.pop_first();
                }
            }
            false
        }
    }

    /// Number of currently open (unmatched) events.
    pub fn open_count(&self) -> usize {
        self.open.len()
    }

    /// The currently open event ids, ascending.
    pub fn open_events(&self) -> impl Iterator<Item = u32> + '_ {
        self.open.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> SpliceTracker {
        SpliceTracker::default()
    }

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
        t.observe(3, true, false);
        assert!(t.observe(3, true, true));
        assert_eq!(t.open_count(), 0);
    }

    #[test]
    fn open_set_is_bounded_on_an_out_only_feed() {
        let mut t = tracker();
        let total = MAX_OPEN_EVENTS as u32 * 4;
        for id in 0..total {
            t.observe(id, true, false);
        }
        assert_eq!(t.open_count(), MAX_OPEN_EVENTS);
        assert_eq!(t.open_events().next(), Some(total - MAX_OPEN_EVENTS as u32));
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
