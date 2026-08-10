//! SCTE-35 cue sanity: well-formedness, arrival, and future-vs-past `pts_time`
//! judgement (ANSI/SCTE 35:2023 §9.7.3.1 `splice_insert`).
//!
//! Two independent entry points, because "well-formed" is only observable at
//! one of them:
//!
//! - [`check_section`] — the **wire** path: raw `splice_info_section` bytes,
//!   as read straight off a [`media_plane::ByteTap`]-recovered TS packet's
//!   payload (after section reassembly, which this module does not itself
//!   do). Can genuinely detect malformed bytes, because nothing has parsed
//!   them yet.
//! - `check_event` (in [`crate::trunk_bridge`], `std` only) — the
//!   **Trunk-cursor** path: a [`timed_metadata::TimedEvent`] already
//!   published into a `media_plane::Trunk`'s event log. By construction,
//!   whoever called `TrunkWriter::publish_event` already had a successfully
//!   parsed section — this path structurally cannot see "malformed"; see
//!   [`crate::metric_names`]'s module docs for why that is stated rather than left
//!   to be discovered from an always-zero metric.

use scte35_splice::SpliceInfoSection;
use scte35_splice::commands::AnyCommand;

use broadcast_common::Parse;

use crate::record::record_counter;

/// Half of the 33-bit `pts_time` range (ANSI/SCTE 35 §9.2, `PTS_MODULUS` =
/// `1 << 33`) — the same wrap-vs-genuine-past threshold
/// `scte35_splice::time::pts_add_wrapping` and `media_doctor::pts_check`'s
/// decode-timestamp tracker both apply: a forward distance greater than half
/// the modulus is a wrap artefact of "actually behind", not "far ahead".
/// Alias for [`broadcast_common::clock33::WRAP_33BIT_HALF`] — the shared
/// owner of that threshold and of the [`judge`] distance calculation below.
const PTS_HALF: u64 = broadcast_common::clock33::WRAP_33BIT_HALF;

/// The result of checking one SCTE-35 `splice_insert` cue's target time
/// against a reference "now".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Scte35Sanity {
    /// The `splice_info_section` bytes did not parse, or were
    /// `encrypted_packet` with no accessible clear view. Wire path only.
    Malformed,
    /// Parsed, but not a `splice_insert` (e.g. `splice_null`, `time_signal`,
    /// `splice_schedule`) — this module only judges `splice_insert`'s
    /// `pts_time`, so there is nothing further to check.
    NotSpliceInsert,
    /// A `splice_insert` with `time_specified_flag == 0` (ANSI/SCTE 35
    /// §9.7.3.1) — an immediate splice has no `pts_time` to judge
    /// future-vs-past against.
    Immediate,
    /// The cue's target `pts_time` is at or ahead of the reference "now" —
    /// the ordinary, healthy case.
    InFuture,
    /// The cue's target `pts_time` had already elapsed relative to the
    /// reference "now" at the moment it was checked — a cue arriving too
    /// late to act on.
    InPast,
}

impl Scte35Sanity {
    /// Stable label for this variant.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Malformed => "malformed",
            Self::NotSpliceInsert => "not_splice_insert",
            Self::Immediate => "immediate",
            Self::InFuture => "in_future",
            Self::InPast => "in_past",
        }
    }
}
broadcast_common::impl_spec_display!(Scte35Sanity);

/// `out_of_network_indicator` as this crate's cue-kind label — mirrors
/// `timed_metadata::EventKind::name()`'s `break_start`/`break_end` tokens so
/// the wire path and the Trunk-cursor path ([`crate::trunk_bridge`]) report
/// the same label vocabulary for `compliance_probe_scte35_cues_total`.
fn kind_label(out_of_network_indicator: bool) -> &'static str {
    if out_of_network_indicator {
        "break_start"
    } else {
        "break_end"
    }
}

/// Check one raw `splice_info_section` (as reassembled from a PMT-declared
/// SCTE-35 PID, ANSI/SCTE 35 §9.6.1, `table_id = 0xFC`) against a reference
/// "now" on the **same 33-bit 90 kHz `pts_time` clock**
/// (ANSI/SCTE 35 §9.2) — typically the most recently observed PES PTS/DTS on
/// this program's reference video/audio PID.
///
/// Records the same `compliance_probe_scte35_*` counters
/// [`crate::metric_names`] documents, then returns the outcome for direct
/// inspection (tests, or a caller building its own summary).
pub fn check_section(section: &[u8], now_pts: u64) -> Scte35Sanity {
    let Ok(sis) = SpliceInfoSection::parse(section) else {
        record_counter!(crate::metric_names::SCTE35_MALFORMED_TOTAL);
        return Scte35Sanity::Malformed;
    };
    let Some(clear) = &sis.clear else {
        // `encrypted_packet` with no accessible clear view: nothing in the
        // command can be inspected.
        record_counter!(crate::metric_names::SCTE35_MALFORMED_TOTAL);
        return Scte35Sanity::Malformed;
    };
    let AnyCommand::SpliceInsert(si) = &clear.command else {
        record_counter!(crate::metric_names::SCTE35_CUES_TOTAL, "kind" => "other");
        return Scte35Sanity::NotSpliceInsert;
    };

    record_counter!(
        crate::metric_names::SCTE35_CUES_TOTAL,
        "kind" => kind_label(si.out_of_network_indicator)
    );

    let Some(pts_time) = si.splice_time.as_ref().and_then(|st| st.pts_time) else {
        record_counter!(crate::metric_names::SCTE35_IMMEDIATE_TOTAL);
        return Scte35Sanity::Immediate;
    };

    judge(pts_time, now_pts)
}

/// The future-vs-past judgement shared by both entry points: a forward
/// wrap-aware distance from `now` to `target`, both already on the same
/// 33-bit tick clock.
pub(crate) fn judge(target_pts: u64, now_pts: u64) -> Scte35Sanity {
    let forward_distance =
        broadcast_common::clock33::wrapping_forward_distance(now_pts, target_pts);
    if forward_distance > PTS_HALF {
        record_counter!(crate::metric_names::SCTE35_PTS_IN_PAST_TOTAL);
        Scte35Sanity::InPast
    } else {
        Scte35Sanity::InFuture
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Serialize;
    use scte35_splice::SpliceInfoSection;
    use scte35_splice::commands::SpliceInsert;
    use scte35_splice::time::{PTS_MODULUS, SpliceTime};

    fn splice_insert_section(pts_time: Option<u64>, out_of_network: bool) -> alloc::vec::Vec<u8> {
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
        section.serialize_into(&mut buf).unwrap();
        buf
    }

    #[test]
    fn malformed_bytes_are_reported_malformed() {
        assert_eq!(
            check_section(&[0xFF, 0xFF, 0xFF], 0),
            Scte35Sanity::Malformed
        );
    }

    #[test]
    fn immediate_splice_has_no_time_to_judge() {
        let bytes = splice_insert_section(None, true);
        assert_eq!(check_section(&bytes, 0), Scte35Sanity::Immediate);
    }

    #[test]
    fn cue_target_ahead_of_now_is_in_future() {
        let now = 90_000; // 1s @ 90kHz
        let target = now + 90_000 * 5; // 5s ahead
        let bytes = splice_insert_section(Some(target), true);
        assert_eq!(check_section(&bytes, now), Scte35Sanity::InFuture);
    }

    #[test]
    fn cue_target_behind_now_is_in_past() {
        let now = 90_000 * 10;
        let target = 90_000 * 3; // 7s behind
        let bytes = splice_insert_section(Some(target), true);
        assert_eq!(check_section(&bytes, now), Scte35Sanity::InPast);
    }

    /// The bite: a target exactly at "now" must not flip to `InPast` from an
    /// off-by-one in the half-modulus comparison.
    #[test]
    fn cue_target_equal_to_now_is_in_future() {
        let now = 12_345u64;
        let bytes = splice_insert_section(Some(now), false);
        assert_eq!(check_section(&bytes, now), Scte35Sanity::InFuture);
    }

    /// Wrap correctness: `now` near the top of the 33-bit range and a target
    /// shortly after wrap must still read as "in future", not a huge past
    /// distance.
    #[test]
    fn judgement_is_correct_across_the_33_bit_wrap() {
        let now = PTS_MODULUS - 1_000;
        let target = 2_000; // 3000 ticks after `now`, having wrapped once
        assert_eq!(judge(target, now), Scte35Sanity::InFuture);
    }

    #[test]
    fn not_splice_insert_command_is_reported() {
        use scte35_splice::commands::SpliceNull;
        let section = SpliceInfoSection::new_clear(AnyCommand::SpliceNull(SpliceNull), &[]);
        let mut buf = alloc::vec![0u8; section.serialized_len()];
        section.serialize_into(&mut buf).unwrap();
        assert_eq!(check_section(&buf, 0), Scte35Sanity::NotSpliceInsert);
    }
}
