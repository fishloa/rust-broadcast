//! SCTE-35 `splice_insert` cue sanity and open-event tracking shared by the
//! workspace's two live TS monitors (`media-doctor` and `compliance-probe`).
//!
//! Two engines live here, over one home:
//!
//! - [`SpliceTracker`] — section reassembly plus the open/closed break state
//!   machine, what `media-doctor check`/`watch` report as unbalanced or
//!   duplicate breaks.
//! - [`SpliceEvent`] and [`judge_splice_time`] — the per-cue cue-sanity
//!   verdict (what kind of cue arrived, and whether its target time is still
//!   ahead of a reference "now"), what `compliance-probe` reports through its
//!   own metric names.
//!
//! # Break state machine
//!
//! ANSI/SCTE 35 §9.8.2 Table 15 (`break_duration()`, transcribed at
//! `private/specs-verified/scte/scte35-splice/tables/scte_35/break-duration-982-table-15-pdf-pp-57-58.md`):
//! with `auto_return` set the splicing device returns to the network feed at
//! the end of `duration`, so an "out" (`out_of_network_indicator` set)
//! carrying `auto_return` closes itself and is never left open; with
//! `auto_return` clear the end of the break is a separate return cue. The
//! tracker stores only the *currently open* ids, capped at
//! [`MAX_OPEN_EVENTS`], so it cannot grow over a long run even on an out-only
//! feed.
//!
//! A section byte-identical to the previous one on the same PID (same CRC,
//! ISO/IEC 13818-1 §2.4.4.5 `CRC_32`) is a retransmission, not a second
//! "out": it is reported as one observation with no duplicate marker.
//!
//! # Cue timing
//!
//! ANSI/SCTE 35 §9.6.1 (Table 5,
//! `private/specs-verified/scte/scte35-splice/tables/scte_35/splice-info-section-96-table-5-pdf-pp-38-43.md`)
//! defines the effective splice time as `pts_time + pts_adjustment` with the
//! carry ignored, so every 33-bit operation here goes through
//! [`broadcast_common::clock33`].

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::commands::AnyCommand;
use crate::section::SpliceInfoSection;
use broadcast_common::Parse;
use mpeg_ts::ts::SectionReassembler;

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
            let Ok(sis) = SpliceInfoSection::parse(&section[..]) else {
                continue;
            };
            let Some(observation) = self.observe_section(&sis, &section[..]) else {
                continue;
            };
            on_event(observation);
        }
    }

    /// Track one already-reassembled `splice_info_section`. Returns `None`
    /// for a section this tracker does not judge: an encrypted one (no key
    /// here, §9.6.1 `encrypted_packet`), a command other than
    /// `splice_insert`, or a `splice_insert` cancelling a named event.
    ///
    /// `raw` is the section's own wire bytes, used only to recognise a
    /// retransmission of the previous section on this PID.
    pub fn observe_section(
        &mut self,
        section: &SpliceInfoSection<'_>,
        raw: &[u8],
    ) -> Option<SpliceObservation> {
        let clear = section.clear.as_ref()?;
        let AnyCommand::SpliceInsert(si) = &clear.command else {
            return None;
        };
        if si.splice_event_cancel_indicator {
            return None;
        }
        let retransmission = !self.last_section.is_empty() && raw == &self.last_section[..];
        self.last_section.clear();
        self.last_section.extend_from_slice(raw);
        let auto_return = si
            .break_duration
            .is_some_and(|break_duration| break_duration.auto_return);
        let duplicate_open = !retransmission
            && self.observe(si.splice_event_id, si.out_of_network_indicator, auto_return);
        Some(SpliceObservation {
            event_id: si.splice_event_id,
            duplicate_open,
        })
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

/// What one SCTE-35 `splice_info_section` is, as far as cue sanity goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpliceSanity {
    /// A well-formed `encrypted_packet` section: the command is not
    /// inspectable without the key, so there is nothing to judge.
    Encrypted,
    /// A `splice_insert` with `splice_event_cancel_indicator` set: it names
    /// an earlier event to cancel and carries no timing or mode fields.
    Cancelled,
    /// Parsed, but not a `splice_insert` (e.g. `splice_null`, `time_signal`,
    /// `splice_schedule`).
    NotSpliceInsert,
}

impl SpliceSanity {
    /// Stable label for this variant.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Encrypted => "encrypted",
            Self::Cancelled => "cancelled",
            Self::NotSpliceInsert => "not_splice_insert",
        }
    }
}

broadcast_common::impl_spec_display!(SpliceSanity);

/// One well-formed, non-cancelled `splice_insert` cue, decoded far enough to
/// judge where it lands on the 90 kHz clock.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SpliceEvent {
    /// `out_of_network_indicator`: `true` for a break start, `false` for the
    /// return to the network feed.
    pub out_of_network_indicator: bool,
    /// The cue's effective splice times (`pts_time + pts_adjustment`, carry
    /// ignored), one per program or per component; empty when the cue is
    /// immediate.
    pub times: Vec<u64>,
}

impl SpliceEvent {
    /// The `kind` label this cue carries, `break_start` or `break_end`.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        kind_label(self.out_of_network_indicator)
    }
}

/// `out_of_network_indicator` as a cue-kind label — the same tokens
/// `timed_metadata::EventKind::name()` uses, so a wire-path reader and a
/// Trunk-cursor reader report one vocabulary.
#[must_use]
pub fn kind_label(out_of_network_indicator: bool) -> &'static str {
    if out_of_network_indicator {
        "break_start"
    } else {
        "break_end"
    }
}

/// Decode one `splice_insert` command: its direction plus its effective
/// splice times. `None` when the command is not a `splice_insert` or cancels
/// a named event (neither opens, closes nor schedules a splice).
#[must_use]
pub fn splice_event<'a>(command: &'a AnyCommand<'a>, pts_adjustment: u64) -> Option<SpliceEvent> {
    let AnyCommand::SpliceInsert(si) = command else {
        return None;
    };
    if si.splice_event_cancel_indicator {
        return None;
    }
    let times = if si.splice_immediate_flag {
        Vec::new()
    } else {
        let program_time = si.splice_time.as_ref().and_then(|st| st.pts_time);
        let component_times = si
            .components
            .iter()
            .filter_map(|c| c.splice_time.as_ref().and_then(|st| st.pts_time));
        let raw: Vec<u64> = if si.program_splice_flag {
            program_time.into_iter().collect()
        } else {
            component_times.collect()
        };
        raw.into_iter()
            .map(|pts_time| crate::time::pts_add_wrapping(pts_time, pts_adjustment))
            .collect()
    };
    Some(SpliceEvent {
        out_of_network_indicator: si.out_of_network_indicator,
        times,
    })
}

/// Is the cue's target at or ahead of `now`, on the 90 kHz wrap modulo
/// `2^33` (ANSI/SCTE 35 §9.2, §9.6.1)? A target more than half the modulus
/// behind `now` is the wrap reading of a genuinely elapsed cue, so it is in
/// the past; anything else is in the future.
#[must_use]
pub fn judge_splice_time(target_pts: u64, now_pts: u64) -> bool {
    broadcast_common::clock33::wrapping_forward_distance(now_pts, target_pts)
        <= broadcast_common::clock33::WRAP_33BIT_HALF
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{SpliceInsert, SpliceInsertComponent};
    use crate::time::{PTS_MODULUS, SpliceTime};
    use broadcast_common::Serialize;

    fn tracker() -> SpliceTracker {
        SpliceTracker::default()
    }

    fn splice_insert(
        pts_time: Option<u64>,
        out_of_network: bool,
    ) -> alloc::vec::Vec<u8> {
        let si = SpliceInsert {
            splice_event_id: 7,
            out_of_network_indicator: out_of_network,
            program_splice_flag: true,
            splice_immediate_flag: pts_time.is_none(),
            splice_time: pts_time.map(SpliceTime::with_pts),
            ..SpliceInsert::default()
        };
        let section = SpliceInfoSection::new_clear(AnyCommand::SpliceInsert(si), &[]);
        let mut buf = alloc::vec![0u8; section.serialized_len()];
        section.serialize_into(&mut buf).expect("serialize");
        buf
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

    #[test]
    fn section_observation_carries_the_event_id() {
        let mut t = tracker();
        let raw = splice_insert(Some(90_000), true);
        let sis = SpliceInfoSection::parse(&raw).expect("parse");
        let observation = t.observe_section(&sis, &raw).expect("a splice_insert");
        assert_eq!(observation.event_id, 7);
        assert!(!observation.duplicate_open);
        assert_eq!(t.open_count(), 1);
    }

    #[test]
    fn identical_sections_are_one_observation_not_a_duplicate() {
        let mut t = tracker();
        let raw = splice_insert(Some(90_000), true);
        let sis = SpliceInfoSection::parse(&raw).expect("parse");
        assert!(!t.observe_section(&sis, &raw).expect("first").duplicate_open);
        assert!(!t.observe_section(&sis, &raw).expect("retransmission").duplicate_open);
    }

    #[test]
    fn cancel_and_encrypted_sections_are_not_observed() {
        let mut t = tracker();
        let si = SpliceInsert {
            splice_event_id: 9,
            splice_event_cancel_indicator: true,
            ..SpliceInsert::default()
        };
        let cancel = SpliceInfoSection::new_clear(AnyCommand::SpliceInsert(si), &[]);
        let mut cancel_buf = alloc::vec![0u8; cancel.serialized_len()];
        cancel.serialize_into(&mut cancel_buf).expect("serialize");
        let cancel_parsed = SpliceInfoSection::parse(&cancel_buf).expect("parse");
        assert!(t.observe_section(&cancel_parsed, &cancel_buf).is_none());

        let mut encrypted =
            SpliceInfoSection::new_clear(AnyCommand::SpliceInsert(SpliceInsert::default()), &[]);
        encrypted.encrypted_packet = true;
        encrypted.clear = None;
        let payload = [0x5Au8; 24];
        encrypted.encrypted_payload = Some(&payload);
        encrypted.encrypted_splice_command_length = Some(20);
        let mut enc_buf = alloc::vec![0u8; encrypted.serialized_len()];
        encrypted.serialize_into(&mut enc_buf).expect("serialize");
        let enc_parsed = SpliceInfoSection::parse(&enc_buf).expect("parse");
        assert!(t.observe_section(&enc_parsed, &enc_buf).is_none());
    }

    #[test]
    fn program_splice_time_is_adjusted_and_labelled() {
        let si = SpliceInsert {
            splice_event_id: 7,
            out_of_network_indicator: true,
            program_splice_flag: true,
            splice_time: Some(SpliceTime::with_pts(90_000 * 3)),
            ..SpliceInsert::default()
        };
        let mut section = SpliceInfoSection::new_clear(AnyCommand::SpliceInsert(si), &[]);
        section.pts_adjustment = 90_000 * 8;
        let event = splice_event(&section.clear.as_ref().expect("clear").command, section.pts_adjustment)
            .expect("a splice_insert");
        assert_eq!(event.times, [90_000 * 11]);
        assert_eq!(event.kind(), "break_start");
    }

    #[test]
    fn immediate_cue_has_no_times() {
        let si = SpliceInsert {
            splice_event_id: 7,
            splice_immediate_flag: true,
            ..SpliceInsert::default()
        };
        let section = SpliceInfoSection::new_clear(AnyCommand::SpliceInsert(si), &[]);
        let event = splice_event(&section.clear.as_ref().expect("clear").command, 0)
            .expect("a splice_insert");
        assert!(event.times.is_empty());
    }

    #[test]
    fn component_mode_cue_uses_component_times() {
        let si = SpliceInsert {
            splice_event_id: 8,
            out_of_network_indicator: true,
            program_splice_flag: false,
            components: [1u8, 2]
                .into_iter()
                .map(|tag| SpliceInsertComponent {
                    component_tag: tag,
                    splice_time: Some(SpliceTime::with_pts(u64::from(tag) * 90_000)),
                })
                .collect(),
            ..SpliceInsert::default()
        };
        let section = SpliceInfoSection::new_clear(AnyCommand::SpliceInsert(si), &[]);
        let event = splice_event(&section.clear.as_ref().expect("clear").command, 0)
            .expect("a splice_insert");
        assert_eq!(event.times, [90_000, 180_000]);
    }

    #[test]
    fn judgement_is_correct_across_the_33_bit_wrap() {
        let now = PTS_MODULUS - 1_000;
        assert!(judge_splice_time(2_000, now));
        assert!(judge_splice_time(now, now));
        assert!(!judge_splice_time(now - 1, now));
    }
}
