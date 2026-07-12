//! Continuous, packet-incremental ingest + Prometheus metrics — the
//! `media-doctor watch` live compliance probe (GitHub issue #665,
//! `docs/IDEAS.md` item #4).
//!
//! # Scope (v1)
//!
//! The full product-vision idea is "watch an SRT/UDP feed ... Prometheus /
//! Grafana". This story deliberately covers **UDP only** (plain
//! unicast/multicast raw-MPEG-TS-over-UDP, the transport countless real
//! broadcast/IPTV chains already use) — SRT needs `srt-runtime`'s sans-IO
//! handshake/ARQ engine and is a natural follow-up issue, not this one.
//!
//! # Design: testable without a socket
//!
//! Everything in this module is pure, `no_std`+`alloc` ingest/accounting
//! logic: [`WatchState::feed_datagram`] takes a raw byte slice (a UDP
//! payload, or any other chunking of a byte stream) and updates the
//! accumulated metrics; [`WatchState::render_prometheus`] renders the current
//! snapshot in Prometheus text exposition format. Neither function touches a
//! socket. The `cli`-gated binary (`src/bin/media-doctor.rs`) is a thin shell
//! around this: a `UdpSocket`/`TcpListener` glue loop that calls these two
//! methods from an `Arc<Mutex<WatchState>>` shared with the metrics HTTP
//! thread. This mirrors `rtsp-runtime`'s sans-IO split: the protocol/ingest
//! logic is a driveable state machine, and the I/O is a separate, thin,
//! swappable layer.
//!
//! # Pipeline
//!
//! Each UDP datagram (some whole number of 188-byte TS packets — commonly
//! but not always 7, ~1316 bytes) is split into sync-byte-aligned 188-byte
//! packets by [`mpeg_ts::resync::TsResync`] (which also tolerates a
//! datagram that doesn't start on a packet boundary, or drops/reorders).
//! Each recovered packet is then fed to:
//!
//! - [`dvb_conformance::ConformanceMonitor`] — the full ETSI TR 101 290
//!   indicator set (sync loss, continuity-count, PCR repetition/discontinuity,
//!   PTS repetition, CRC, transport_error, PAT/PMT/PID/CAT/SI-repetition
//!   errors), timed against **wall-clock arrival time** (not stream-embedded
//!   PCR) — for a live probe the question is "is this indicator's real-time
//!   deadline being met on the wire *now*", which wall-clock answers more
//!   directly than a PCR-derived clock.
//! - [`dvb_si::demux::SiDemux`] — PAT/PMT discovery, so PMT-declared
//!   SCTE-35 (`stream_type` `0x86`), H.264/HEVC, and AAC-ADTS PIDs are found
//!   dynamically rather than assumed at a fixed PID.
//! - A per-PID SCTE-35 `splice_insert` open/closed tracker (same shape as
//!   [`crate::Scte35Check`], simplified for a running probe: no
//!   end-of-stream-only "unbalanced" report — a live probe never reaches
//!   "end of stream", so "currently open" is exposed as a live gauge
//!   instead).
//! - A per-PID decode-timestamp (DTS, else PTS) backward-jump tracker (same
//!   check as [`crate::PtsCheck`], restructured to hold state across calls
//!   instead of scanning a whole buffer).
//! - A per-PID declared-codec-vs-bitstream-framing tracker (same check as
//!   [`crate::CodecSignallingCheck`]: PMT says H.264/HEVC/AAC-ADTS but the
//!   elementary stream never once looks like that codec's framing).
//!
//! # What's NOT separately re-wired here, and why
//!
//! [`crate::PcrCheck`] and [`crate::CcAnomalyCheck`] are **not** duplicated
//! as separate incremental trackers: `ConformanceMonitor` already computes
//! the TR 101 290 `PCR_repetition_error`/`PCR_discontinuity_indicator_error`
//! and `Continuity_count_error` indicators from the same per-packet data —
//! re-implementing the same arithmetic a second time would only add drift
//! risk for no new information. Their signal is exposed via
//! `media_doctor_conformance_events_total{indicator=...}`.
//!
//! [`crate::PatPmtVersionCheck`], [`crate::FpsCadenceCheck`],
//! [`crate::ParamSetsCheck`], [`crate::InterlaceCheck`], and
//! [`crate::SyncByteCheck`] are whole-capture-shaped (version-change history,
//! VUI-vs-measured-cadence, wire-order-before-first-IDR, a content fact, and
//! "no sync byte at all in this file" respectively) and are left as
//! one-shot `check` diagnostics for v1.

use alloc::collections::btree_map::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;
use core::time::Duration;

use broadcast_common::Parse;
use dvb_conformance::ConformanceMonitor;
use dvb_si::demux::SiDemux;
use dvb_si::tables::AnyTableSection;
use dvb_si::tables::pmt::StreamType;
use mpeg_pes::{PesAssembler, PesPacket};
use mpeg_ts::resync::TsResync;
use mpeg_ts::ts::{SectionReassembler, TS_PACKET_SIZE, TsPacket};
use transmux::{iter_annexb_nals, parse_adts_header};

/// `table_id` of a SCTE-35 `splice_info_section` (ANSI/SCTE 35 §9.6.1).
const SCTE35_TABLE_ID: u8 = 0xFC;

/// 33-bit PTS/DTS modulus (ISO/IEC 13818-1 §2.4.3.7) — 2^33.
const PTS_MODULUS: u64 = 1u64 << 33;
/// Half of the 33-bit range: the threshold distinguishing a genuine backward
/// jump from a legal wrap.
const PTS_HALF: u64 = 1u64 << 32;

/// Which framing/timing rules apply to a tracked elementary-stream PID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EsKind {
    /// PMT declared H.264 (`0x1B`) or HEVC (`0x24`) — Annex B NAL framing.
    Video,
    /// PMT declared AAC-ADTS (`0x0F`) — ADTS sync framing.
    AudioAdts,
}

/// Per-PID elementary-stream tracking: PES reassembly + the codec-framing and
/// decode-timestamp state carried across packets.
struct EsTrack {
    kind: EsKind,
    assembler: PesAssembler,
    /// At least one access unit has been reassembled at all.
    any_au: bool,
    /// At least one access unit looked like the declared codec's framing.
    structured: bool,
    /// Previous decode timestamp (DTS, else PTS), 33-bit raw ticks.
    prev_decode: Option<u64>,
    /// A backward decode-timestamp jump has been observed on this PID.
    decode_anomaly: bool,
}

impl EsTrack {
    fn new(kind: EsKind) -> Self {
        Self {
            kind,
            assembler: PesAssembler::new(),
            any_au: false,
            structured: false,
            prev_decode: None,
            decode_anomaly: false,
        }
    }
}

/// Per-PID SCTE-35 tracking: section reassembly + open/closed `splice_insert`
/// state per `splice_event_id` (`true` = "out" seen, no matching "in" yet).
#[derive(Default)]
struct Scte35Track {
    reassembler: SectionReassembler,
    events: BTreeMap<u32, bool>,
}

/// Accumulated count for one TR 101 290 [`dvb_conformance::Indicator`].
struct ConformanceCount {
    priority: &'static str,
    clause: &'static str,
    count: u64,
}

/// The continuous ingest + metrics accumulator driving `media-doctor watch`.
///
/// Feed raw byte chunks (UDP datagrams, or any other framing of a live TS
/// byte stream) with [`feed_datagram`](Self::feed_datagram); read the current
/// accumulated state at any time with [`render_prometheus`](Self::render_prometheus).
/// Never touches a socket — see the module docs for the split with the `cli`
/// glue that does.
pub struct WatchState {
    resync: TsResync,
    conformance: ConformanceMonitor,
    demux: SiDemux,
    es_tracks: BTreeMap<u16, EsTrack>,
    scte35_tracks: BTreeMap<u16, Scte35Track>,
    conformance_counts: BTreeMap<&'static str, ConformanceCount>,
    datagrams_total: u64,
    scte35_events_total: u64,
    pts_dts_anomalies_total: u64,
    last_clock: Duration,
}

impl Default for WatchState {
    fn default() -> Self {
        Self::new()
    }
}

impl WatchState {
    /// Create an empty accumulator.
    #[must_use]
    pub fn new() -> Self {
        Self {
            resync: TsResync::new(),
            conformance: ConformanceMonitor::new(),
            demux: SiDemux::builder().build(),
            es_tracks: BTreeMap::new(),
            scte35_tracks: BTreeMap::new(),
            conformance_counts: BTreeMap::new(),
            datagrams_total: 0,
            scte35_events_total: 0,
            pts_dts_anomalies_total: 0,
            last_clock: Duration::ZERO,
        }
    }

    /// Feed one raw byte chunk (a UDP datagram payload, or any other framing
    /// of a live byte stream) at wall-clock arrival time `clock` (elapsed
    /// time since ingest started — must be monotonically non-decreasing
    /// across calls, matching [`dvb_conformance::ConformanceMonitor::feed`]'s
    /// contract).
    ///
    /// The payload need not be 188-byte-aligned or start on a packet
    /// boundary: [`mpeg_ts::resync::TsResync`] recovers sync-byte-aligned
    /// 188-byte TS packets from whatever bytes are actually present, buffering
    /// any leftover bytes for the next call.
    pub fn feed_datagram(&mut self, payload: &[u8], clock: Duration) {
        self.datagrams_total += 1;
        let packets = self.resync.feed(payload);
        for packet in &packets {
            self.feed_ts_packet(packet, clock);
        }
    }

    /// Feed one already-aligned 188-byte TS packet.
    fn feed_ts_packet(&mut self, packet: &[u8; TS_PACKET_SIZE], clock: Duration) {
        self.last_clock = clock;

        for ev in self.conformance.feed(packet, clock) {
            let entry = self
                .conformance_counts
                .entry(ev.indicator.name())
                .or_insert_with(|| ConformanceCount {
                    priority: ev.priority.name(),
                    clause: ev.indicator.clause(),
                    count: 0,
                });
            entry.count += 1;
        }

        let Ok(ts_packet) = TsPacket::parse(packet) else {
            return;
        };
        let pid = ts_packet.header.pid;

        // PAT/PMT discovery: pick up PMT-declared SCTE-35/video/audio PIDs
        // dynamically rather than assuming a fixed PID.
        let events: Vec<_> = self.demux.feed(packet).collect();
        for ev in events {
            if let Ok(AnyTableSection::PmtSection(pmt)) = ev.table_section() {
                for stream in &pmt.streams {
                    match stream.stream_type {
                        StreamType::Scte35 => {
                            self.scte35_tracks.entry(stream.elementary_pid).or_default();
                        }
                        StreamType::H264 | StreamType::Hevc => {
                            self.es_tracks
                                .entry(stream.elementary_pid)
                                .or_insert_with(|| EsTrack::new(EsKind::Video));
                        }
                        StreamType::AacAdts => {
                            self.es_tracks
                                .entry(stream.elementary_pid)
                                .or_insert_with(|| EsTrack::new(EsKind::AudioAdts));
                        }
                        _ => {}
                    }
                }
            }
        }

        if self.scte35_tracks.contains_key(&pid) {
            self.feed_scte35(pid, &ts_packet);
        }
        if self.es_tracks.contains_key(&pid) {
            self.feed_es(pid, &ts_packet);
        }
    }

    /// Reassemble + parse `splice_info_section`s on a PMT-declared SCTE-35
    /// PID, tracking each `splice_event_id`'s open/closed state.
    fn feed_scte35(&mut self, pid: u16, ts_packet: &TsPacket<'_>) {
        let Some(payload) = ts_packet.payload else {
            return;
        };
        let track = self.scte35_tracks.get_mut(&pid).expect("checked by caller");
        track.reassembler.feed(payload, ts_packet.header.pusi);

        while let Some(section) = track.reassembler.pop_section() {
            if section.is_empty() || section[0] != SCTE35_TABLE_ID {
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
            if si.splice_event_cancel_indicator {
                continue;
            }

            self.scte35_events_total += 1;
            track
                .events
                .insert(si.splice_event_id, si.out_of_network_indicator);
        }
    }

    /// Reassemble PES on a PMT-declared video/audio PID, checking codec
    /// framing and decode-timestamp monotonicity on each completed unit.
    fn feed_es(&mut self, pid: u16, ts_packet: &TsPacket<'_>) {
        let mut discontinuity = false;
        if ts_packet.header.has_adaptation {
            if let Some(Ok(af)) = ts_packet.adaptation_field() {
                discontinuity = af.discontinuity_indicator;
            }
        }

        let track = self.es_tracks.get_mut(&pid).expect("checked by caller");
        if discontinuity {
            // A signalled TS-layer discontinuity resets the decode-timestamp
            // baseline (mirrors PtsCheck) — the jump across it is legitimate,
            // not an anomaly. Codec-framing state (any_au/structured) is a
            // content fact and is not reset.
            track.assembler = PesAssembler::new();
            track.prev_decode = None;
            return;
        }

        let Some(payload) = ts_packet.payload else {
            return;
        };
        if payload.is_empty() {
            return;
        }

        let pes_bytes = track.assembler.feed(ts_packet.header.pusi, payload);
        let Some(pes_bytes) = pes_bytes else {
            return;
        };
        self.process_es_unit(pid, &pes_bytes);
    }

    /// Check one reassembled PES unit's ES-payload framing and decode
    /// timestamp against the tracked PID's running state.
    fn process_es_unit(&mut self, pid: u16, pes_bytes: &[u8]) {
        let Ok(pes) = PesPacket::parse(pes_bytes) else {
            return;
        };
        let track = self.es_tracks.get_mut(&pid).expect("checked by caller");
        track.any_au = true;
        match track.kind {
            EsKind::Video => {
                if iter_annexb_nals(pes.payload).next().is_some() {
                    track.structured = true;
                }
            }
            EsKind::AudioAdts => {
                if has_adts_sync(pes.payload) {
                    track.structured = true;
                }
            }
        }

        let Some(header) = pes.header else {
            return;
        };
        let (raw, present) = match (header.dts, header.pts) {
            (Some(dts), _) => (dts.ticks(), true),
            (None, Some(pts)) => (pts.ticks(), true),
            (None, None) => (0, false),
        };
        if !present {
            return;
        }

        if let Some(prev) = track.prev_decode {
            let delta = raw.wrapping_sub(prev) & (PTS_MODULUS - 1);
            if delta != 0 && delta > PTS_HALF {
                track.decode_anomaly = true;
                self.pts_dts_anomalies_total += 1;
            }
        }
        track.prev_decode = Some(raw);
    }

    /// Render the current accumulated state in Prometheus text exposition
    /// format (a `GET /metrics` response body).
    ///
    /// Values reflect state as of this call; a real Prometheus scraper
    /// computes rates/deltas itself from successive scrapes of these
    /// monotonic counters.
    #[must_use]
    pub fn render_prometheus(&self) -> String {
        let mut out = String::new();
        let conformance_stats = self.conformance.stats();
        let resync_stats = self.resync.stats();

        metric_header(
            &mut out,
            "media_doctor_packets_total",
            "Total well-formed 188-byte TS packets processed (ISO/IEC 13818-1 section 2.4.3.2).",
            "counter",
        );
        let _ = writeln!(
            out,
            "media_doctor_packets_total {}",
            conformance_stats.packets
        );

        metric_header(
            &mut out,
            "media_doctor_datagrams_total",
            "Total ingest datagrams fed (e.g. UDP payloads).",
            "counter",
        );
        let _ = writeln!(out, "media_doctor_datagrams_total {}", self.datagrams_total);

        metric_header(
            &mut out,
            "media_doctor_resync_events_total",
            "Times TS byte-stream sync was lost and reacquired (mpeg_ts::resync::TsResync).",
            "counter",
        );
        let _ = writeln!(
            out,
            "media_doctor_resync_events_total {}",
            resync_stats.resyncs
        );

        metric_header(
            &mut out,
            "media_doctor_dropped_bytes_total",
            "Bytes dropped before/while reacquiring TS packet sync.",
            "counter",
        );
        let _ = writeln!(
            out,
            "media_doctor_dropped_bytes_total {}",
            resync_stats.dropped_bytes
        );

        metric_header(
            &mut out,
            "media_doctor_conformance_in_sync",
            "Whether the ETSI TR 101 290 monitor currently considers the stream in sync (1) or not (0).",
            "gauge",
        );
        let _ = writeln!(
            out,
            "media_doctor_conformance_in_sync {}",
            u8::from(conformance_stats.in_sync)
        );

        metric_header(
            &mut out,
            "media_doctor_conformance_events_total",
            "ETSI TR 101 290 indicator events observed, by indicator and priority tier.",
            "counter",
        );
        for (name, c) in &self.conformance_counts {
            let _ = writeln!(
                out,
                "media_doctor_conformance_events_total{{indicator=\"{}\",priority=\"{}\"}} {}",
                escape_label(name),
                escape_label(c.priority),
                c.count,
            );
        }
        if !self.conformance_counts.is_empty() {
            out.push_str("# clauses: ");
            let mut first = true;
            for (name, c) in &self.conformance_counts {
                if !first {
                    out.push_str(", ");
                }
                first = false;
                let _ = write!(out, "{name}={}", c.clause);
            }
            out.push('\n');
        }

        metric_header(
            &mut out,
            "media_doctor_scte35_events_total",
            "Total SCTE-35 splice_insert events observed (ANSI/SCTE 35 section 9.7.3.1), excluding cancelled events.",
            "counter",
        );
        let _ = writeln!(
            out,
            "media_doctor_scte35_events_total {}",
            self.scte35_events_total
        );

        let scte35_open: u64 = self
            .scte35_tracks
            .values()
            .flat_map(|t| t.events.values())
            .filter(|&&open| open)
            .count() as u64;
        metric_header(
            &mut out,
            "media_doctor_scte35_open_events",
            "Currently-unmatched (\"out\" with no \"in\" yet) SCTE-35 splice_insert events.",
            "gauge",
        );
        let _ = writeln!(out, "media_doctor_scte35_open_events {scte35_open}");

        metric_header(
            &mut out,
            "media_doctor_pts_dts_anomalies_total",
            "Non-monotonic decode-timestamp (DTS, else PTS) events observed on tracked PES PIDs.",
            "counter",
        );
        let _ = writeln!(
            out,
            "media_doctor_pts_dts_anomalies_total {}",
            self.pts_dts_anomalies_total
        );

        metric_header(
            &mut out,
            "media_doctor_codec_signalling_mismatch",
            "Whether a PMT-declared codec PID has ever shown bitstream framing disagreeing with \
             the declared stream_type (1) or not (0); only emitted once at least one access unit \
             has been observed on that PID (ISO/IEC 13818-1 Table 2-34).",
            "gauge",
        );
        for (&pid, track) in &self.es_tracks {
            if track.any_au {
                let mismatch = u8::from(!track.structured);
                let _ = writeln!(
                    out,
                    "media_doctor_codec_signalling_mismatch{{pid=\"0x{pid:04X}\"}} {mismatch}"
                );
            }
        }

        metric_header(
            &mut out,
            "media_doctor_pts_dts_anomaly",
            "Whether a tracked PES PID has ever shown a non-monotonic decode timestamp (1) or not \
             (0); only emitted once a decode timestamp has been observed on that PID.",
            "gauge",
        );
        for (&pid, track) in &self.es_tracks {
            if track.prev_decode.is_some() {
                let _ = writeln!(
                    out,
                    "media_doctor_pts_dts_anomaly{{pid=\"0x{pid:04X}\"}} {}",
                    u8::from(track.decode_anomaly)
                );
            }
        }

        metric_header(
            &mut out,
            "media_doctor_last_packet_clock_seconds",
            "Elapsed ingest wall-clock time (seconds) of the most recently processed TS packet.",
            "gauge",
        );
        let _ = writeln!(
            out,
            "media_doctor_last_packet_clock_seconds {}",
            self.last_clock.as_secs_f64()
        );

        out
    }
}

/// Append a `# HELP` / `# TYPE` pair for `name` (Prometheus text exposition
/// format).
fn metric_header(out: &mut String, name: &str, help: &str, ty: &str) {
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} {ty}");
}

/// Escape a Prometheus label value: backslash, double-quote, newline
/// (<https://prometheus.io/docs/instrumenting/exposition_formats/>).
fn escape_label(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

/// Scan `payload` for a byte offset at which a well-formed ADTS header parses
/// (ISO/IEC 13818-7 section 6.2, via [`transmux::parse_adts_header`]) —
/// mirrors [`crate::CodecSignallingCheck`]'s own helper (kept private to each
/// module: both are small and the two checks are otherwise independent).
fn has_adts_sync(payload: &[u8]) -> bool {
    const ADTS_MIN: usize = 7;
    if payload.len() < ADTS_MIN {
        return false;
    }
    (0..=payload.len() - ADTS_MIN).any(|off| {
        payload[off] == 0xFF
            && (payload[off + 1] & 0xF0) == 0xF0
            && parse_adts_header(&payload[off..]).is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read a committed real-capture fixture from the workspace `fixtures/`
    /// directory (shared across crates, one level up from this crate root).
    fn fixture(rel: &str) -> Vec<u8> {
        let path = format!(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/{}"), rel);
        std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {path}: {e}"))
    }

    /// Split `bytes` into fixed-size chunks, simulating UDP datagrams without
    /// opening a socket. The last chunk may be shorter.
    fn chunks(bytes: &[u8], size: usize) -> Vec<&[u8]> {
        bytes.chunks(size).collect()
    }

    /// The exit-gate test: feed a real broadcast capture
    /// (`fixtures/ts/m6-single.ts`, exactly 1264 whole 188-byte packets)
    /// chunked into 1316-byte pieces (the common "7 TS packets per UDP
    /// payload" convention) as if it arrived over UDP, and check the
    /// resulting Prometheus exposition text against fully-known-in-advance
    /// facts about the fixture.
    #[test]
    fn watches_real_fixture_and_produces_sane_metrics() {
        let bytes = fixture("ts/m6-single.ts");
        assert_eq!(
            bytes.len() % TS_PACKET_SIZE,
            0,
            "fixture must be whole TS packets"
        );
        let expected_packets = bytes.len() / TS_PACKET_SIZE;

        let mut state = WatchState::new();
        let mut clock = Duration::ZERO;
        for datagram in chunks(&bytes, 7 * TS_PACKET_SIZE) {
            state.feed_datagram(datagram, clock);
            clock += Duration::from_millis(1);
        }

        let text = state.render_prometheus();

        // Every packet in a whole-packet-count real capture, split on exact
        // TS_PACKET_SIZE multiples, must come out the other end — the
        // resync must not drop or duplicate a single one.
        assert_eq!(
            metric_value(&text, "media_doctor_packets_total"),
            Some(expected_packets as f64),
            "packets_total must match the fixture's real packet count:\n{text}"
        );
        assert_eq!(
            metric_value(&text, "media_doctor_datagrams_total"),
            Some(chunks(&bytes, 7 * TS_PACKET_SIZE).len() as f64)
        );
        // A clean, real, exactly-188-aligned capture fed in exact 7-packet
        // chunks must never need a resync.
        assert_eq!(
            metric_value(&text, "media_doctor_resync_events_total"),
            Some(0.0)
        );
        assert_eq!(
            metric_value(&text, "media_doctor_dropped_bytes_total"),
            Some(0.0)
        );

        // The TR 101 290 monitor must have actually run (packets fed).
        assert!(text.contains("media_doctor_conformance_in_sync"));

        // The metrics text must be non-empty and contain the documented
        // metric family names (a scrape-shape smoke test).
        for name in [
            "media_doctor_packets_total",
            "media_doctor_datagrams_total",
            "media_doctor_scte35_events_total",
            "media_doctor_scte35_open_events",
            "media_doctor_pts_dts_anomalies_total",
            "media_doctor_last_packet_clock_seconds",
        ] {
            assert!(
                text.contains(name),
                "missing metric family {name} in:\n{text}"
            );
        }
    }

    /// A datagram that doesn't start on a packet boundary (leading garbage
    /// before the first sync byte) must still resync and recover packets,
    /// and the resync must be visible in the metrics.
    #[test]
    fn misaligned_datagram_resyncs_and_still_counts_packets() {
        let bytes = fixture("ts/m6-single.ts");
        let mut misaligned = alloc::vec![0xAAu8; 37];
        misaligned.extend_from_slice(&bytes[..20 * TS_PACKET_SIZE]);

        let mut state = WatchState::new();
        state.feed_datagram(&misaligned, Duration::ZERO);
        let text = state.render_prometheus();

        // 20 real packets in, minus whatever partial tail didn't complete a
        // full packet after the 37-byte offset -- most must still come out.
        let packets = metric_value(&text, "media_doctor_packets_total").unwrap();
        assert!(
            packets >= 15.0,
            "expected the resynchroniser to recover most of the 20 fed packets, got {packets}"
        );
        assert_eq!(
            metric_value(&text, "media_doctor_dropped_bytes_total"),
            Some(37.0),
            "the 37 leading garbage bytes must be counted as dropped"
        );
    }

    /// Garbage input must never panic, and must report an honestly empty
    /// snapshot.
    #[test]
    fn garbage_datagram_never_panics() {
        let mut state = WatchState::new();
        state.feed_datagram(&[0u8; 4096], Duration::ZERO);
        let text = state.render_prometheus();
        assert_eq!(metric_value(&text, "media_doctor_packets_total"), Some(0.0));
    }

    /// End-to-end SCTE-35 pipeline: PAT -> PMT (declaring stream_type 0x86 on
    /// a PID) -> a real `splice_info_section` built by scte35-splice's own
    /// serializer, fed as a sequence of "datagrams" (one TS packet each).
    /// None of the committed real captures declare an SCTE-35 stream in
    /// their PMT (see `demo/src/lib.rs`'s equivalent test), so this
    /// synthesizes the PMT-declared-PID scenario while keeping every
    /// section spec-correct by construction.
    #[test]
    fn scte35_declared_in_pmt_is_reassembled_and_counted() {
        use broadcast_common::Serialize;
        use dvb_si::descriptors::DescriptorLoop;
        use dvb_si::tables::pat::{PatEntry, PatSection};
        use dvb_si::tables::pmt::{PmtSection, PmtStream};
        use mpeg_ts::pid::well_known as wk;
        use mpeg_ts::ts::TsHeader;
        use scte35_splice::SpliceInfoSection;
        use scte35_splice::commands::{AnyCommand, SpliceInsert};

        const PMT_PID: u16 = 0x0100;
        const SCTE35_PID: u16 = 0x0101;

        fn ts_packet(pid: u16, section: &[u8]) -> [u8; TS_PACKET_SIZE] {
            let mut pkt = [0xFFu8; TS_PACKET_SIZE];
            let header = TsHeader {
                tei: false,
                pusi: true,
                pid,
                scrambling: 0,
                has_adaptation: false,
                has_payload: true,
                continuity_counter: 0,
            };
            header.serialize_into(&mut pkt).unwrap();
            pkt[4] = 0x00; // pointer_field
            pkt[5..5 + section.len()].copy_from_slice(section);
            pkt
        }

        let pat = PatSection {
            transport_stream_id: 1,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            entries: alloc::vec![PatEntry {
                program_number: 1,
                pid: PMT_PID,
            }],
        };
        let mut pat_bytes = alloc::vec![0u8; pat.serialized_len()];
        pat.serialize_into(&mut pat_bytes).unwrap();

        let pmt = PmtSection::new(
            1,
            0,
            true,
            0,
            0,
            wk::NULL.value(),
            DescriptorLoop::new(&[]),
            alloc::vec![PmtStream {
                stream_type: StreamType::Scte35,
                elementary_pid: SCTE35_PID,
                es_info: DescriptorLoop::new(&[]),
            }],
        );
        let mut pmt_bytes = alloc::vec![0u8; pmt.serialized_len()];
        pmt.serialize_into(&mut pmt_bytes).unwrap();

        let splice_out = SpliceInfoSection::new_clear(
            AnyCommand::SpliceInsert(SpliceInsert {
                splice_event_id: 42,
                out_of_network_indicator: true,
                program_splice_flag: true,
                splice_immediate_flag: true,
                ..SpliceInsert::default()
            }),
            &[],
        );
        let out_bytes = splice_out.to_bytes();

        let mut state = WatchState::new();
        let mut clock = Duration::ZERO;
        // mpeg_ts::resync::TsResync (the same resynchroniser feed_datagram
        // uses) only declares packet-stride lock once it has seen
        // `LOCK_CONFIRMATIONS` (5) consecutive sync bytes at that stride —
        // exactly like a real live probe needs a few clean packets before it
        // starts trusting the byte stream. Two null-PID filler packets after
        // the three meaningful ones bring the total to 5, so lock is
        // reached (and all 5 packets, including the PAT/PMT/SCTE-35 ones,
        // are then emitted together) within this test.
        for packet in [
            ts_packet(wk::PAT.value(), &pat_bytes),
            ts_packet(PMT_PID, &pmt_bytes),
            ts_packet(SCTE35_PID, &out_bytes),
            ts_packet(wk::NULL.value(), &[]),
            ts_packet(wk::NULL.value(), &[]),
        ] {
            state.feed_datagram(&packet, clock);
            clock += Duration::from_millis(1);
        }

        let text = state.render_prometheus();
        assert_eq!(
            metric_value(&text, "media_doctor_scte35_events_total"),
            Some(1.0)
        );
        assert_eq!(
            metric_value(&text, "media_doctor_scte35_open_events"),
            Some(1.0),
            "the 'out' has no matching 'in' yet, so it must show as open:\n{text}"
        );
    }

    /// Extract the bare numeric value of a metric with no labels from
    /// rendered Prometheus text (test-only helper).
    fn metric_value(text: &str, name: &str) -> Option<f64> {
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix(name) {
                let rest = rest.trim_start();
                if rest.starts_with('{') {
                    continue;
                }
                return rest.trim().parse().ok();
            }
        }
        None
    }
}
