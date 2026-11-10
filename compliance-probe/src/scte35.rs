//! SCTE-35 cue sanity: well-formedness, arrival, and future-vs-past `pts_time`
//! judgement (ANSI/SCTE 35:2023 §9.7.3.1 `splice_insert`).
//!
//! The future-vs-past verdict is computed on the adjusted splice time (`pts_time + pts_adjustment` modulo 2^33), as SCTE 35 defines.
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
    /// The `splice_info_section` bytes did not parse. Wire path only.
    Malformed,
    /// A well-formed `encrypted_packet` section: the command is not
    /// inspectable without the key, so there is nothing to judge.
    Encrypted,
    /// A `splice_insert` with `splice_event_cancel_indicator == 1`: it names
    /// an earlier event to cancel and carries no timing or mode fields.
    Cancelled,
    /// Parsed, but not a `splice_insert` (e.g. `splice_null`, `time_signal`,
    /// `splice_schedule`) — this module only judges `splice_insert`'s
    /// `pts_time`, so there is nothing further to check.
    NotSpliceInsert,
    /// A `splice_insert` with `splice_immediate_flag == 1`, or whose
    /// `splice_time()` has `time_specified_flag == 0` (ANSI/SCTE 35
    /// §9.7.3.1) — it has no `pts_time` to judge future-vs-past against.
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
            Self::Encrypted => "encrypted",
            Self::Cancelled => "cancelled",
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
        record_counter!(crate::metric_names::SCTE35_ENCRYPTED_TOTAL);
        return Scte35Sanity::Encrypted;
    };
    let AnyCommand::SpliceInsert(si) = &clear.command else {
        record_counter!(crate::metric_names::SCTE35_CUES_TOTAL, "kind" => "other");
        return Scte35Sanity::NotSpliceInsert;
    };

    if si.splice_event_cancel_indicator {
        record_counter!(crate::metric_names::SCTE35_CUES_TOTAL, "kind" => "cancel");
        return Scte35Sanity::Cancelled;
    }

    record_counter!(
        crate::metric_names::SCTE35_CUES_TOTAL,
        "kind" => kind_label(si.out_of_network_indicator)
    );

    let program_time = si.splice_time.as_ref().and_then(|st| st.pts_time);
    let component_times = si
        .components
        .iter()
        .filter_map(|c| c.splice_time.as_ref().and_then(|st| st.pts_time));
    let mut times: alloc::vec::Vec<u64> = if si.program_splice_flag {
        program_time.into_iter().collect()
    } else {
        component_times.collect()
    };
    if si.splice_immediate_flag || times.is_empty() {
        record_counter!(crate::metric_names::SCTE35_IMMEDIATE_TOTAL);
        return Scte35Sanity::Immediate;
    }

    // §9.6.1 defines the effective splice time as `pts_time + pts_adjustment`
    // (mod 2^33) — the field exists precisely so a downstream splicer/remux
    // can rebase cues without rewriting them (W-CP-1).
    for pts_time in &mut times {
        *pts_time = scte35_splice::time::pts_add_wrapping(*pts_time, sis.pts_adjustment);
    }

    let mut verdict = Scte35Sanity::InFuture;
    for effective_pts in times {
        if judge(effective_pts, now_pts) == Scte35Sanity::InPast {
            verdict = Scte35Sanity::InPast;
        }
    }
    verdict
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
    use scte35_splice::commands::{SpliceInsert, SpliceInsertComponent};
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

    /// W-CP-1: a cue whose `pts_adjustment` rebases a stale-looking
    /// `pts_time` into the future must be judged by the *effective* time
    /// (`pts_time + pts_adjustment`), not the raw wire `pts_time`.
    #[test]
    fn pts_adjustment_is_applied_before_judging() {
        let now = 90_000 * 10; // 10s @ 90kHz
        let raw_pts_time = 90_000 * 3; // 7s behind `now` on its own
        let adjustment = 90_000 * 8; // rebases it to 11s: ahead of `now`

        let si = SpliceInsert {
            splice_event_id: 7,
            out_of_network_indicator: true,
            program_splice_flag: true,
            splice_immediate_flag: false,
            splice_time: Some(SpliceTime::with_pts(raw_pts_time)),
            ..SpliceInsert::default()
        };
        let mut section = SpliceInfoSection::new_clear(AnyCommand::SpliceInsert(si), &[]);
        section.pts_adjustment = adjustment;
        let mut buf = alloc::vec![0u8; section.serialized_len()];
        section.serialize_into(&mut buf).unwrap();

        assert_eq!(
            check_section(&buf, now),
            Scte35Sanity::InFuture,
            "raw pts_time alone is 7s behind `now`, but pts_adjustment rebases \
             it to 11s ahead — the cue must be judged InFuture, not InPast"
        );
    }

    #[test]
    fn pts_adjustment_wrap_edge_large_adjustment_on_zero_pts_is_in_past() {
        let now = 0u64;
        let raw_pts_time = 0u64;
        let adjustment = PTS_MODULUS - 1_000;
        let si = SpliceInsert {
            splice_event_id: 7,
            out_of_network_indicator: true,
            program_splice_flag: true,
            splice_immediate_flag: false,
            splice_time: Some(SpliceTime::with_pts(raw_pts_time)),
            ..SpliceInsert::default()
        };
        let mut section = SpliceInfoSection::new_clear(AnyCommand::SpliceInsert(si), &[]);
        section.pts_adjustment = adjustment;
        let mut buf = alloc::vec![0u8; section.serialized_len()];
        section.serialize_into(&mut buf).unwrap();
        assert_eq!(check_section(&buf, now), Scte35Sanity::InPast);
    }

    #[test]
    fn pts_adjustment_wrap_edge_near_top_pts_wraps_to_future() {
        let now = 0u64;
        let raw_pts_time = PTS_MODULUS - 10;
        let adjustment = 100u64;
        let si = SpliceInsert {
            splice_event_id: 7,
            out_of_network_indicator: true,
            program_splice_flag: true,
            splice_immediate_flag: false,
            splice_time: Some(SpliceTime::with_pts(raw_pts_time)),
            ..SpliceInsert::default()
        };
        let mut section = SpliceInfoSection::new_clear(AnyCommand::SpliceInsert(si), &[]);
        section.pts_adjustment = adjustment;
        let mut buf = alloc::vec![0u8; section.serialized_len()];
        section.serialize_into(&mut buf).unwrap();
        assert_eq!(check_section(&buf, now), Scte35Sanity::InFuture);
    }

    #[test]
    fn not_splice_insert_command_is_reported() {
        use scte35_splice::commands::SpliceNull;
        let section = SpliceInfoSection::new_clear(AnyCommand::SpliceNull(SpliceNull), &[]);
        let mut buf = alloc::vec![0u8; section.serialized_len()];
        section.serialize_into(&mut buf).unwrap();
        assert_eq!(check_section(&buf, 0), Scte35Sanity::NotSpliceInsert);
    }

    fn bytes(section: &SpliceInfoSection<'_>) -> alloc::vec::Vec<u8> {
        let mut buf = alloc::vec![0u8; section.serialized_len()];
        section.serialize_into(&mut buf).unwrap();
        buf
    }

    #[test]
    fn cancelled_splice_insert_is_cancelled_not_immediate() {
        let si = SpliceInsert {
            splice_event_id: 7,
            splice_event_cancel_indicator: true,
            ..SpliceInsert::default()
        };
        let b = bytes(&SpliceInfoSection::new_clear(
            AnyCommand::SpliceInsert(si),
            &[],
        ));
        assert_eq!(check_section(&b, 90_000), Scte35Sanity::Cancelled);
    }

    #[test]
    fn component_mode_cue_is_judged_by_its_component_times() {
        let component_mode = |tags_and_pts: &[(u8, u64)]| {
            let si = SpliceInsert {
                splice_event_id: 8,
                out_of_network_indicator: true,
                program_splice_flag: false,
                splice_immediate_flag: false,
                components: tags_and_pts
                    .iter()
                    .map(|&(component_tag, pts)| SpliceInsertComponent {
                        component_tag,
                        splice_time: Some(SpliceTime::with_pts(pts)),
                    })
                    .collect(),
                ..SpliceInsert::default()
            };
            bytes(&SpliceInfoSection::new_clear(
                AnyCommand::SpliceInsert(si),
                &[],
            ))
        };
        let now = 90_000 * 10;
        assert_eq!(
            check_section(&component_mode(&[(1, 90_000 * 3)]), now),
            Scte35Sanity::InPast
        );
        assert_eq!(
            check_section(&component_mode(&[(1, now + 90_000), (2, 90_000 * 3)]), now),
            Scte35Sanity::InPast,
            "one elapsed component makes the cue late"
        );
        assert_eq!(
            check_section(
                &component_mode(&[(1, now + 90_000), (2, now + 180_000)]),
                now
            ),
            Scte35Sanity::InFuture
        );
    }

    #[test]
    fn immediate_flag_wins_over_any_time_field() {
        let si = SpliceInsert {
            splice_event_id: 9,
            program_splice_flag: true,
            splice_immediate_flag: true,
            ..SpliceInsert::default()
        };
        let b = bytes(&SpliceInfoSection::new_clear(
            AnyCommand::SpliceInsert(si),
            &[],
        ));
        assert_eq!(check_section(&b, 0), Scte35Sanity::Immediate);
    }

    #[test]
    fn well_formed_encrypted_section_is_encrypted_not_malformed() {
        use broadcast_common::Parse;
        let mut s =
            SpliceInfoSection::new_clear(AnyCommand::SpliceInsert(SpliceInsert::default()), &[]);
        s.encrypted_packet = true;
        s.clear = None;
        let payload = [0x5Au8; 24];
        s.encrypted_payload = Some(&payload);
        s.encrypted_splice_command_length = Some(20);
        let b = bytes(&s);
        assert!(SpliceInfoSection::parse(&b).is_ok(), "fixture must parse");
        assert_eq!(check_section(&b, 0), Scte35Sanity::Encrypted);
    }
}
