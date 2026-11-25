//! Container/stream structural checks, delegated to `media-doctor`'s
//! [`media_doctor::Diagnostic`] harness rather than reimplemented.
//!
//! # Honesty note: a whole window, re-scanned
//!
//! `media_doctor::Diagnostic` is incremental — it is scanned one packet at a
//! time — but each of its checks keeps per-stream state (a version-change
//! history, a wire-order-before-first-IDR scan, a per-`splice_event_id`
//! balance check) that is only meaningful over the bytes it was given. This
//! module therefore does **not** offer a `feed_ts_packet`-shaped incremental
//! API: [`run_structural_checks`] re-scans whatever buffer it is given, from
//! the start, every time it is called. A caller wiring this to a live feed
//! (e.g. a periodic snapshot of a `media_plane::Trunk` `SegmentCursor`'s
//! recent bytes) chooses its own re-check cadence and window size.
//!
//! `media_doctor::CcAnomalyCheck` and `media_doctor::PcrCheck` overlap with
//! [`crate::Probe::feed_ts_packet`]'s own TR 101 290
//! `Continuity_count_error`/`PCR_repetition_error`/
//! `PCR_discontinuity_indicator_error` indicators and this crate's own PCR
//! drift tracker — they are included here anyway because `run_all` scans the
//! *whole buffer at once* on independent state, not because they add new
//! information; a caller who has already fed every packet through
//! [`crate::Probe::feed_ts_packet`] is not obligated to also call this
//! function with the same bytes.

use media_doctor::{
    CcAnomalyCheck, CodecSignallingCheck, Diagnostic, PatPmtVersionCheck, PcrCheck, PtsCheck,
    Report, Scte35Check, SyncByteCheck,
};

use crate::record::record_counter;

/// Number of `media-doctor` structural diagnostics driven by
/// [`run_structural_checks`].
const STRUCTURAL_DIAGNOSTIC_COUNT: usize = 7;

/// Run `media-doctor`'s structural [`Diagnostic`] set over `ts` (a
/// contiguous, whole-number-of-188-byte-packets TS byte buffer — one window
/// of the stream), recording
/// `compliance_probe_structural_findings_total` for every [`media_doctor::Finding`]
/// raised, then returning the full [`Report`] for direct inspection.
pub fn run_structural_checks(ts: &[u8]) -> Report {
    let checks: [&dyn Diagnostic; STRUCTURAL_DIAGNOSTIC_COUNT] = [
        &SyncByteCheck,
        &PatPmtVersionCheck,
        &CcAnomalyCheck,
        &PcrCheck,
        &PtsCheck,
        &CodecSignallingCheck,
        &Scte35Check,
    ];
    let mut report = Report::new();
    media_doctor::run_all(ts, &checks, &mut report);

    for finding in report.findings() {
        record_counter!(
            crate::metric_names::STRUCTURAL_FINDINGS_TOTAL,
            "rule_id" => finding.rule_id.clone(),
            "severity" => finding.severity.name(),
        );
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A whole 188-byte packet with a bad sync byte must be flagged — proves
    /// this module is actually wired to `media-doctor`'s checks, not a stub.
    /// `SyncByteCheck` only examines whole packets (`ts.len() / 188`), so the
    /// buffer must be a full packet, not a short fragment, to bite.
    #[test]
    fn bad_sync_byte_is_flagged() {
        let bad_packet = [0x00u8; 188];
        let report = run_structural_checks(&bad_packet);
        assert!(
            !report.findings().is_empty(),
            "expected at least one finding for a non-0x47 sync byte"
        );
        assert!(report.findings().iter().any(|f| f.rule_id == "sync-byte"));
    }

    /// An empty buffer must not panic and must report a clean (or at least
    /// non-crashing) empty result.
    #[test]
    fn empty_buffer_never_panics() {
        let report = run_structural_checks(&[]);
        assert_eq!(report.len(), 0);
    }
}
