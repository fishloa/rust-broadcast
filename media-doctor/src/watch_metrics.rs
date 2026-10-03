//! Prometheus exposition for [`WatchState`] through the `metrics` facade and
//! `metrics-exporter-prometheus` (de-hand-roll W1-P, SP2.3) — replaces the
//! hand-written `render_prometheus` text builder.
//!
//! Every publish builds a **fresh** recorder: the `metrics` registry cannot
//! unregister a series, and `watch` must stop exposing a PID's gauge once the
//! PID is released, exactly as the old snapshot renderer did. Counters carry
//! the accumulated total (`absolute`), not a delta.

use metrics::{counter, describe_counter, describe_gauge, gauge, with_local_recorder};
use metrics_exporter_prometheus::PrometheusBuilder;

use crate::{WatchSnapshot, WatchState};

const PACKETS_TOTAL: &str = "media_doctor_packets_total";
const DATAGRAMS_TOTAL: &str = "media_doctor_datagrams_total";
const RESYNC_EVENTS_TOTAL: &str = "media_doctor_resync_events_total";
const DROPPED_BYTES_TOTAL: &str = "media_doctor_dropped_bytes_total";
const IN_SYNC: &str = "media_doctor_conformance_in_sync";
const CONFORMANCE_EVENTS_TOTAL: &str = "media_doctor_conformance_events_total";
const SCTE35_EVENTS_TOTAL: &str = "media_doctor_scte35_events_total";
const SCTE35_OPEN_EVENTS: &str = "media_doctor_scte35_open_events";
const PTS_DTS_ANOMALIES_TOTAL: &str = "media_doctor_pts_dts_anomalies_total";
const CODEC_SIGNALLING_MISMATCH: &str = "media_doctor_codec_signalling_mismatch";
const PTS_DTS_ANOMALY: &str = "media_doctor_pts_dts_anomaly";
const LAST_PACKET_CLOCK_SECONDS: &str = "media_doctor_last_packet_clock_seconds";

/// Render `state` as Prometheus text exposition format (a `GET /metrics` body).
#[must_use]
pub fn render(state: &WatchState) -> String {
    let snapshot = state.snapshot();
    let recorder = PrometheusBuilder::new().build_recorder();
    with_local_recorder(&recorder, || publish(&snapshot));
    recorder.handle().render()
}

/// Write `snapshot` into the current `metrics` recorder.
pub fn publish(s: &WatchSnapshot) {
    describe_counter!(
        PACKETS_TOTAL,
        "Total well-formed 188-byte TS packets processed (ISO/IEC 13818-1 section 2.4.3.2)."
    );
    describe_counter!(
        DATAGRAMS_TOTAL,
        "Total ingest datagrams fed (e.g. UDP payloads)."
    );
    describe_counter!(
        RESYNC_EVENTS_TOTAL,
        "Times TS byte-stream sync was lost and reacquired (mpeg_ts::resync::TsResync)."
    );
    describe_counter!(
        DROPPED_BYTES_TOTAL,
        "Bytes dropped before/while reacquiring TS packet sync."
    );
    describe_gauge!(
        IN_SYNC,
        "Whether the ETSI TR 101 290 monitor currently considers the stream in sync (1) or not (0)."
    );
    describe_counter!(
        CONFORMANCE_EVENTS_TOTAL,
        "ETSI TR 101 290 indicator events observed, by indicator and priority tier."
    );
    describe_counter!(
        SCTE35_EVENTS_TOTAL,
        "Total SCTE-35 splice_insert events observed (ANSI/SCTE 35 section 9.7.3.1), excluding cancelled events."
    );
    describe_gauge!(
        SCTE35_OPEN_EVENTS,
        "Currently-unmatched (\"out\" with no \"in\" yet, and no auto-return) SCTE-35 splice_insert events."
    );
    describe_counter!(
        PTS_DTS_ANOMALIES_TOTAL,
        "Non-monotonic decode-timestamp (DTS, else PTS) events observed on tracked PES PIDs."
    );
    describe_gauge!(
        CODEC_SIGNALLING_MISMATCH,
        "Whether a PMT-declared codec PID has ever shown bitstream framing disagreeing with the declared stream_type (1) or not (0); only emitted once at least one access unit has been observed on that PID (ISO/IEC 13818-1 Table 2-34)."
    );
    describe_gauge!(
        PTS_DTS_ANOMALY,
        "Whether a tracked PES PID has ever shown a non-monotonic decode timestamp (1) or not (0); only emitted once a decode timestamp has been observed on that PID."
    );
    describe_gauge!(
        LAST_PACKET_CLOCK_SECONDS,
        "Elapsed ingest wall-clock time (seconds) of the most recently processed TS packet."
    );

    counter!(PACKETS_TOTAL).absolute(s.packets);
    counter!(DATAGRAMS_TOTAL).absolute(s.datagrams);
    counter!(RESYNC_EVENTS_TOTAL).absolute(s.resync_events);
    counter!(DROPPED_BYTES_TOTAL).absolute(s.dropped_bytes);
    gauge!(IN_SYNC).set(f64::from(u8::from(s.in_sync)));
    for c in &s.conformance {
        counter!(CONFORMANCE_EVENTS_TOTAL, "indicator" => c.indicator, "priority" => c.priority)
            .absolute(c.count);
    }
    counter!(SCTE35_EVENTS_TOTAL).absolute(s.scte35_events);
    gauge!(SCTE35_OPEN_EVENTS).set(f64::from(u32::try_from(s.scte35_open).unwrap_or(u32::MAX)));
    counter!(PTS_DTS_ANOMALIES_TOTAL).absolute(s.pts_dts_anomalies);
    for f in &s.codec_signalling {
        gauge!(CODEC_SIGNALLING_MISMATCH, "pid" => alloc::format!("0x{:04X}", f.pid))
            .set(f64::from(u8::from(f.set)));
    }
    for f in &s.pts_dts_anomaly {
        gauge!(PTS_DTS_ANOMALY, "pid" => alloc::format!("0x{:04X}", f.pid))
            .set(f64::from(u8::from(f.set)));
    }
    gauge!(LAST_PACKET_CLOCK_SECONDS).set(s.last_packet_clock_seconds);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watch::tests::{drop_header_only_families, parse_exposition};
    use core::time::Duration;

    fn assert_matches_main_golden(fixture: &str, golden_name: &str) {
        let bytes = std::fs::read(format!(
            "{}/../fixtures/ts/{fixture}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let mut state = WatchState::new();
        let mut clock = Duration::ZERO;
        for datagram in bytes.chunks(7 * 188) {
            state.feed_datagram(datagram, clock);
            clock += Duration::from_millis(1);
        }
        let golden = std::fs::read_to_string(format!(
            "{}/tests/golden/watch/{golden_name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let (mut golden_meta, golden_series) = parse_exposition(&golden);
        drop_header_only_families(&mut golden_meta, &golden_series);
        let (new_meta, new_series) = parse_exposition(&render(&state));
        assert_eq!(golden_meta, new_meta, "HELP/TYPE differ from {golden_name}");
        assert_eq!(
            golden_series, new_series,
            "series differ from {golden_name}"
        );
    }

    /// The exposition of the committed capture equals, semantically, the
    /// golden the OLD renderer produced on `main` (see
    /// `tests/golden/watch/README.md` for the accepted, neutral differences).
    #[test]
    fn real_fixture_exposition_equals_the_main_golden_semantically() {
        assert_matches_main_golden("m6-single.ts", "m6-single.prom");
        let golden = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/golden/watch/m6-single.prom"
        ))
        .unwrap();
        assert_eq!(
            parse_exposition(&golden)
                .1
                .get("media_doctor_packets_total"),
            Some(&1264.0),
            "the golden itself must carry the fixture's packet count"
        );
    }

    /// Same, for a capture with H.264 + AAC PIDs: covers the per-PID labelled
    /// families (`pid="0x0100"`), two conformance indicators and their label
    /// ordering, none of which `m6-single` has.
    #[test]
    fn per_pid_exposition_equals_the_main_golden_semantically() {
        assert_matches_main_golden("h264_aac.ts", "h264_aac.prom");
        let golden = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/golden/watch/h264_aac.prom"
        ))
        .unwrap();
        assert!(
            golden.contains("media_doctor_codec_signalling_mismatch{pid=\"0x0100\"} 0"),
            "the golden must carry per-PID series"
        );
    }
}
