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

use crate::diagnostics::codec_common::has_adts_sync;
use crate::diagnostics::scte35_track::SpliceTracker;
use dvb_conformance::ConformanceMonitor;
use dvb_si::demux::SiDemux;
use dvb_si::tables::AnyTableSection;
use dvb_si::tables::pmt::StreamType;
use mpeg_pes::{PesAssembler, PesPacket};
use mpeg_ts::resync::TsResync;
use mpeg_ts::ts::{TS_PACKET_SIZE, TsPacket};
use transmux::iter_annexb_nals;

/// Half of the 33-bit PTS/DTS range: the threshold distinguishing a genuine
/// backward jump from a legal wrap (ISO/IEC 13818-1 §2.4.3.7). The modulus
/// itself comes from [`broadcast_common::clock33`] — the shared owner of this
/// math, rather than a second hand-rolled `1 << 33` (audit MD-W10).
const PTS_HALF: u64 = broadcast_common::clock33::WRAP_33BIT_HALF;

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
    scte35_tracks: BTreeMap<u16, SpliceTracker>,
    /// Each program's current declaration: `program_number ->
    /// (version_number, elementary_pid -> stream_type)`. A new
    /// `version_number` means the program's stream set may have changed, so
    /// its tracks are reconciled against the new PMT (audit MD-W8).
    pmt_generations: BTreeMap<u16, (u8, BTreeMap<u16, StreamType>)>,
    /// How many programs currently declare each elementary PID. A PID shared
    /// by two programs must stay tracked while either still declares it.
    pid_refs: BTreeMap<u16, usize>,
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
            pmt_generations: BTreeMap::new(),
            pid_refs: BTreeMap::new(),
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
        //
        // A PMT's `version_number` changes whenever its contents change
        // (ETSI EN 300 468 §5.1), so a new version means the program's stream
        // set may be different. The old code only ever *added* tracks
        // (`entry().or_insert…`), so a moved PID kept its old `EsKind` and
        // reported `codec-signalling-mismatch` forever, a removed SCTE-35 PID
        // kept being tracked, and a stale `prev_decode` survived the codec
        // change (audit MD-W8).
        //
        // Reconciling naively — dropping and recreating the whole program —
        // is also wrong: a version bump that changes nothing about a PID
        // would reset its open SCTE-35 events, its decode-timestamp baseline
        // and its codec-framing evidence, and would drop a PID that another
        // program still declares. So the update is per PID:
        //
        // - a PID whose declared `stream_type` is unchanged keeps its state;
        // - a PID whose `stream_type` moved is re-created (its framing and
        //   timestamp state belonged to the old codec);
        // - a PID no longer declared by ANY program stops being tracked.
        //
        // `pid_refs` is what makes the last rule safe for a PID two programs
        // share: it is dropped only when the count reaches zero.
        let events: Vec<_> = self.demux.feed(packet).collect();
        for ev in events {
            match ev.table_section() {
                Ok(AnyTableSection::PmtSection(pmt)) => {
                    let declared: alloc::vec::Vec<(u16, StreamType)> = pmt
                        .streams
                        .iter()
                        .map(|s| (s.elementary_pid, s.stream_type))
                        .collect();
                    self.apply_pmt_declaration(pmt.program_number, pmt.version_number, &declared);
                }
                // A PAT's program list is what says which programs still
                // exist: a program that disappears from it must release its
                // PIDs, or its tracks are held for the life of the process.
                Ok(AnyTableSection::PatSection(pat)) => {
                    let present: alloc::vec::Vec<u16> = pat
                        .entries
                        .iter()
                        .map(|e| e.program_number)
                        .filter(|&p| p != dvb_si::tables::pat::PROGRAM_NUMBER_NIT)
                        .collect();
                    self.retain_programs(&present);
                }
                _ => {}
            }
        }

        if self.scte35_tracks.contains_key(&pid) {
            self.feed_scte35(pid, &ts_packet);
        }
        if self.es_tracks.contains_key(&pid) {
            self.feed_es(pid, &ts_packet);
        }
    }

    /// Apply one program's declaration directly, the way a PMT section does.
    ///
    /// Exposed (crate-internal) so the reconciliation can be driven without
    /// going through `SiDemux`'s version gate — a mux can re-declare a PID
    /// without the gate having to see a new version.
    fn apply_pmt_declaration(
        &mut self,
        program_number: u16,
        version_number: u8,
        declared: &[(u16, StreamType)],
    ) {
        let declared: BTreeMap<u16, StreamType> = declared.iter().copied().collect();
        let previous = self
            .pmt_generations
            .insert(program_number, (version_number, declared.clone()));
        if previous
            .as_ref()
            .is_some_and(|(version, old)| *version == version_number && *old == declared)
        {
            return;
        }
        let old_declared = previous.map(|(_, old)| old).unwrap_or_default();
        for old_pid in old_declared.keys() {
            if !declared.contains_key(old_pid) {
                self.release_pid(*old_pid);
            }
        }
        for (&elem_pid, &stream_type) in &declared {
            if !old_declared.contains_key(&elem_pid) {
                self.pid_refs
                    .entry(elem_pid)
                    .and_modify(|n| *n += 1)
                    .or_insert(1);
            }
            if old_declared.get(&elem_pid) == Some(&stream_type) {
                continue;
            }
            self.retrack(elem_pid, stream_type);
        }
    }

    /// (Re-)create the tracker for `elem_pid` under `stream_type`, clearing
    /// any tracker of the other kind.
    fn retrack(&mut self, elem_pid: u16, stream_type: StreamType) {
        match stream_type {
            StreamType::Scte35 => {
                self.es_tracks.remove(&elem_pid);
                self.scte35_tracks.entry(elem_pid).or_default();
            }
            StreamType::H264 | StreamType::Hevc => {
                self.scte35_tracks.remove(&elem_pid);
                self.es_tracks.insert(elem_pid, EsTrack::new(EsKind::Video));
            }
            StreamType::AacAdts => {
                self.scte35_tracks.remove(&elem_pid);
                self.es_tracks
                    .insert(elem_pid, EsTrack::new(EsKind::AudioAdts));
            }
            _ => {
                self.es_tracks.remove(&elem_pid);
                self.scte35_tracks.remove(&elem_pid);
            }
        }
    }

    /// Drop every program not in `present`, releasing the PIDs it declared.
    ///
    /// Clearing a program's `pmt_generations` entry is what lets a later
    /// re-appearance of the same `program_number` be treated as a fresh
    /// declaration (and so be re-tracked) rather than compared against a
    /// stale generation.
    fn retain_programs(&mut self, present: &[u16]) {
        let gone: alloc::vec::Vec<u16> = self
            .pmt_generations
            .keys()
            .copied()
            .filter(|p| !present.contains(p))
            .collect();
        for program_number in gone {
            let Some((_, declared)) = self.pmt_generations.remove(&program_number) else {
                continue;
            };
            for elem_pid in declared.keys() {
                self.release_pid(*elem_pid);
            }
        }
    }

    /// Drop one reference to `pid`. When the count reaches zero the PID's
    /// trackers go; while it is still non-zero the PID stays tracked, but its
    /// *kind* is restored to that of a remaining declarer.
    ///
    /// The restore matters because two programs may declare the same PID with
    /// different `stream_type`s: `retrack` is last-declarer-wins, so when that
    /// declarer leaves, the PID would otherwise keep a codec nothing declares
    /// any more.
    fn release_pid(&mut self, pid: u16) {
        let Some(count) = self.pid_refs.get_mut(&pid) else {
            return;
        };
        *count = count.saturating_sub(1);
        if *count > 0 {
            if let Some(stream_type) = self.remaining_declared_type(pid) {
                self.retrack(pid, stream_type);
            }
            return;
        }
        self.pid_refs.remove(&pid);
        self.es_tracks.remove(&pid);
        self.scte35_tracks.remove(&pid);
    }

    /// The `stream_type` some still-registered program declares for `pid`.
    /// `pmt_generations` is a `BTreeMap` keyed by program number, so this
    /// takes the lowest-numbered declarer and is deterministic.
    fn remaining_declared_type(&self, pid: u16) -> Option<StreamType> {
        self.pmt_generations
            .values()
            .filter_map(|(_, declared)| declared.get(&pid).copied())
            .next()
    }

    /// Reassemble + parse `splice_info_section`s on a PMT-declared SCTE-35
    /// PID, tracking each `splice_event_id`'s open/closed state.
    fn feed_scte35(&mut self, pid: u16, ts_packet: &TsPacket<'_>) {
        let Some(payload) = ts_packet.payload else {
            return;
        };
        let Some(track) = self.scte35_tracks.get_mut(&pid) else {
            return;
        };
        // The shared tracker keeps only events that are still open (an "out"
        // that expects a separate return cue); a matching "in" — or an "out"
        // carrying `break_duration.auto_return = true`, which closes at its
        // own duration (ANSI/SCTE 35 §9.8.2, §9.9.2.2) — drops the entry, so
        // memory is one entry per *currently* open break, not per event ever
        // seen (audit MD-W3).
        let events_total = &mut self.scte35_events_total;
        track.feed(payload, ts_packet.header.pusi, |_| *events_total += 1);
    }

    /// Reassemble PES on a PMT-declared video/audio PID, checking codec
    /// framing and decode-timestamp monotonicity on each completed unit.
    fn feed_es(&mut self, pid: u16, ts_packet: &TsPacket<'_>) {
        let mut discontinuity = false;
        if ts_packet.header.has_adaptation
            && let Some(Ok(af)) = ts_packet.adaptation_field()
        {
            discontinuity = af.discontinuity_indicator;
        }

        let Some(track) = self.es_tracks.get_mut(&pid) else {
            return;
        };
        if discontinuity {
            // A signalled TS-layer discontinuity resets the decode-timestamp
            // baseline (mirrors PtsCheck) — the jump across it is legitimate,
            // not an anomaly. Codec-framing state (any_au/structured) is a
            // content fact and is not reset.
            //
            // Reset only: this packet's payload is still fed below. The
            // discontinuity packet is normally the start of the first PES
            // after the break, so returning here would drop that access unit
            // and leave the first post-break timestamp unchecked (audit
            // MD-W5).
            track.assembler = PesAssembler::new();
            track.prev_decode = None;
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
        let Some(track) = self.es_tracks.get_mut(&pid) else {
            return;
        };
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
            let delta = broadcast_common::clock33::wrapping_forward_distance(prev, raw);
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
            .map(|t| u64::try_from(t.open_count()).unwrap_or(u64::MAX))
            .sum();
        metric_header(
            &mut out,
            "media_doctor_scte35_open_events",
            "Currently-unmatched (\"out\" with no \"in\" yet, and no auto-return) SCTE-35 splice_insert events.",
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

#[cfg(test)]
mod tests {
    use super::*;

    /// PMT PID every synthetic PMT fixture is carried on.
    const PAT_PMT_PID: u16 = 0x0100;
    /// Elementary PID the synthetic fixtures declare.
    const SHARED_PID: u16 = 0x0101;
    /// SCTE-35 cue PID the synthetic fixtures declare.
    const CUE_PID: u16 = 0x0102;
    /// An unrelated elementary PID, used where a program must stop
    /// referencing `SHARED_PID`.
    const OTHER_PID: u16 = 0x0103;

    /// PMT declaring one H.264 video PID.
    const SINGLE_H264_PMT: u8 = 0;
    /// PMT declaring one AAC-ADTS PID.
    const SINGLE_AAC_PMT: u8 = 1;
    /// PMT declaring an H.264 PID and an SCTE-35 cue PID.
    const WITH_CUE_PMT: u8 = 2;

    /// Advance the resynchroniser past its lock threshold with null packets,
    /// then declare the synthetic PMT PID with a real PAT so `SiDemux` will
    /// forward that PID's sections.
    fn lock_stride(
        state: &mut WatchState,
        feed: &mut impl FnMut(&mut WatchState, &[u8; TS_PACKET_SIZE]),
    ) {
        use broadcast_common::Serialize;
        use dvb_si::tables::pat::{PatEntry, PatSection};
        let pat = PatSection {
            transport_stream_id: 1,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            entries: alloc::vec![PatEntry {
                program_number: 1,
                pid: PAT_PMT_PID,
            }],
        };
        let mut pat_bytes = alloc::vec![0u8; pat.serialized_len()];
        let n = pat.serialize_into(&mut pat_bytes).expect("serialize PAT");
        pat_bytes.truncate(n);
        feed(
            state,
            &section_packet(dvb_si::tables::pat::PID, 0, &pat_bytes),
        );
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS {
            let mut null = [0xFFu8; TS_PACKET_SIZE];
            let header = mpeg_ts::ts::TsHeader {
                tei: false,
                pusi: false,
                pid: mpeg_ts::pid::well_known::NULL.value(),
                scrambling: 0,
                has_adaptation: false,
                has_payload: true,
                continuity_counter: 0,
            };
            header.serialize_into(&mut null).unwrap();
            feed(state, &null);
        }
    }

    /// A PAT section packet listing `programs`, at `version`.
    fn pat_packet(programs: &[u16], version: u8) -> [u8; TS_PACKET_SIZE] {
        use broadcast_common::Serialize;
        use dvb_si::tables::pat::{PatEntry, PatSection};
        let pat = PatSection {
            transport_stream_id: 1,
            version_number: version,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            entries: programs
                .iter()
                .map(|&program_number| PatEntry {
                    program_number,
                    pid: PAT_PMT_PID,
                })
                .collect(),
        };
        let mut bytes = alloc::vec![0u8; pat.serialized_len()];
        let n = pat.serialize_into(&mut bytes).expect("serialize PAT");
        bytes.truncate(n);
        section_packet(dvb_si::tables::pat::PID, version, &bytes)
    }

    /// A section-bearing TS packet.
    fn section_packet(pid: u16, cc: u8, section: &[u8]) -> [u8; TS_PACKET_SIZE] {
        use mpeg_ts::ts::TsHeader;
        let mut pkt = [0xFFu8; TS_PACKET_SIZE];
        let header = TsHeader {
            tei: false,
            pusi: true,
            pid,
            scrambling: 0,
            has_adaptation: false,
            has_payload: true,
            continuity_counter: cc & 0x0F,
        };
        header.serialize_into(&mut pkt).unwrap();
        pkt[4] = 0x00; // pointer_field
        pkt[5..5 + section.len()].copy_from_slice(section);
        pkt
    }

    /// Build a `(elementary_pid, stream_type)` declaration from one of the
    /// `SINGLE_*`/`WITH_CUE_PMT` shapes.
    fn pmt_shape(shape: u8) -> alloc::vec::Vec<(u16, StreamType)> {
        match shape {
            SINGLE_H264_PMT => alloc::vec![(SHARED_PID, StreamType::H264)],
            SINGLE_AAC_PMT => alloc::vec![(SHARED_PID, StreamType::AacAdts)],
            WITH_CUE_PMT => alloc::vec![
                (SHARED_PID, StreamType::H264),
                (CUE_PID, StreamType::Scte35),
            ],
            other => panic!("unknown PMT shape {other}"),
        }
    }

    /// A PMT section packet for program 1, version `version`, declaring
    /// `shape`.
    fn pmt_section_packet(shape: u8, pmt_pid: u16, version: u8, cc: &[u8]) -> [u8; TS_PACKET_SIZE] {
        use broadcast_common::Serialize;
        use dvb_si::descriptors::any::DescriptorLoop;
        use dvb_si::tables::pmt::{PmtSection, PmtStream};
        let streams: alloc::vec::Vec<PmtStream<'_>> = pmt_shape(shape)
            .into_iter()
            .map(|(pid, stream_type)| PmtStream {
                stream_type,
                elementary_pid: pid,
                es_info: DescriptorLoop::new(&[]),
            })
            .collect();
        let pmt = PmtSection::new(
            1,
            version,
            true,
            0,
            0,
            streams.first().map(|s| s.elementary_pid).unwrap_or(0x1FFF),
            DescriptorLoop::new(&[]),
            streams,
        );
        let mut bytes = alloc::vec![0u8; pmt.serialized_len()];
        let n = pmt.serialize_into(&mut bytes).expect("serialize PMT");
        bytes.truncate(n);
        section_packet(pmt_pid, cc.first().copied().unwrap_or(0), &bytes)
    }

    /// A PMT section packet for a specific `program_number`.
    fn pmt_section_packet_for_program(
        program_number: u16,
        pmt_pid: u16,
        version: u8,
        elem_pid: u16,
        cc: &[u8],
    ) -> [u8; TS_PACKET_SIZE] {
        use broadcast_common::Serialize;
        use dvb_si::descriptors::any::DescriptorLoop;
        use dvb_si::tables::pmt::{PmtSection, PmtStream};
        let pmt = PmtSection::new(
            program_number,
            version,
            true,
            0,
            0,
            elem_pid,
            DescriptorLoop::new(&[]),
            alloc::vec![PmtStream {
                stream_type: StreamType::H264,
                elementary_pid: elem_pid,
                es_info: DescriptorLoop::new(&[]),
            }],
        );
        let mut bytes = alloc::vec![0u8; pmt.serialized_len()];
        let n = pmt.serialize_into(&mut bytes).expect("serialize PMT");
        bytes.truncate(n);
        section_packet(pmt_pid, cc.first().copied().unwrap_or(0), &bytes)
    }

    /// A PES packet carrying `payload` on `pid`, wrapped in one TS packet.
    fn pes_packet(pid: u16, cc: u8, stream_id: u8, payload: &[u8]) -> [u8; TS_PACKET_SIZE] {
        use mpeg_ts::ts::TsHeader;
        let pes_len = 9 + payload.len();
        let mut pes = alloc::vec![0x00, 0x00, 0x01, stream_id];
        pes.extend_from_slice(&((pes_len - 6) as u16).to_be_bytes());
        pes.push(0x80);
        pes.push(0x00); // no PTS/DTS
        pes.push(0x00);
        pes.extend_from_slice(payload);
        let mut pkt = [0xFFu8; TS_PACKET_SIZE];
        let header = TsHeader {
            tei: false,
            pusi: true,
            pid,
            scrambling: 0,
            has_adaptation: false,
            has_payload: true,
            continuity_counter: cc & 0x0F,
        };
        header.serialize_into(&mut pkt).unwrap();
        pkt[4..4 + pes.len()].copy_from_slice(&pes);
        pkt
    }

    /// A PES packet on `pid` whose payload is an Annex B access unit (a NAL
    /// start code), i.e. video framing.
    fn video_packet(pid: u16, cc: u8) -> [u8; TS_PACKET_SIZE] {
        pes_packet(pid, cc, 0xE0, &[0x00, 0x00, 0x00, 0x01, 0x65, 0xAA])
    }

    /// A PES packet on `pid` whose payload is an ADTS frame, i.e. audio
    /// framing.
    fn adts_packet(pid: u16, cc: u8) -> [u8; TS_PACKET_SIZE] {
        pes_packet(pid, cc, 0xC0, &[0xFF, 0xF1, 0x50, 0x80, 0x00, 0x1F, 0xFC])
    }

    /// A `splice_info_section` with a `splice_insert` on `pid`.
    fn scte35_packet(pid: u16, event_id: u32, out: bool) -> [u8; TS_PACKET_SIZE] {
        use broadcast_common::Serialize;
        use scte35_splice::SpliceInfoSection;
        use scte35_splice::commands::{AnyCommand, SpliceInsert};
        let sis = SpliceInfoSection::new_clear(
            AnyCommand::SpliceInsert(SpliceInsert {
                splice_event_id: event_id,
                out_of_network_indicator: out,
                program_splice_flag: true,
                splice_immediate_flag: true,
                ..SpliceInsert::default()
            }),
            &[],
        );
        let mut bytes = alloc::vec![0u8; sis.serialized_len()];
        let n = sis.serialize_into(&mut bytes).expect("serialize SCTE-35");
        bytes.truncate(n);
        section_packet(pid, 0, &bytes)
    }

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

    /// Audit MD-W3, `watch` side: an auto-return "out" (`break_duration`
    /// with `auto_return = true`) closes at its own duration, and a matched
    /// out->in pair closes too, so neither leaves an entry behind. The
    /// open-events gauge must read 0, and repeatedly cycling both kinds of
    /// event must not grow the per-PID map.
    ///
    /// Before the fix the map held one entry per `splice_event_id` for the
    /// life of the process, and an auto-return out was counted as open
    /// forever (`media_doctor_scte35_open_events` only ever grew).
    #[test]
    fn scte35_auto_return_and_matched_pairs_do_not_stay_open() {
        use broadcast_common::Serialize;
        use dvb_si::descriptors::DescriptorLoop;
        use dvb_si::tables::pat::{PatEntry, PatSection};
        use dvb_si::tables::pmt::{PmtSection, PmtStream};
        use mpeg_ts::pid::well_known as wk;
        use mpeg_ts::ts::TsHeader;
        use scte35_splice::SpliceInfoSection;
        use scte35_splice::commands::{AnyCommand, SpliceInsert};
        use scte35_splice::time::BreakDuration;

        const PMT_PID: u16 = 0x0100;
        const SCTE35_PID: u16 = 0x0101;
        const CYCLES: u32 = 20;

        fn ts_packet(pid: u16, cc: u8, section: &[u8]) -> [u8; TS_PACKET_SIZE] {
            let mut pkt = [0xFFu8; TS_PACKET_SIZE];
            let header = TsHeader {
                tei: false,
                pusi: true,
                pid,
                scrambling: 0,
                has_adaptation: false,
                has_payload: true,
                continuity_counter: cc & 0x0F,
            };
            header.serialize_into(&mut pkt).unwrap();
            pkt[4] = 0x00; // pointer_field
            pkt[5..5 + section.len()].copy_from_slice(section);
            pkt
        }

        fn section(event_id: u32, out: bool, auto_return: Option<bool>) -> alloc::vec::Vec<u8> {
            SpliceInfoSection::new_clear(
                AnyCommand::SpliceInsert(SpliceInsert {
                    splice_event_id: event_id,
                    out_of_network_indicator: out,
                    program_splice_flag: true,
                    splice_immediate_flag: true,
                    break_duration: auto_return.map(|auto_return| BreakDuration {
                        auto_return,
                        duration: 90_000,
                    }),
                    ..SpliceInsert::default()
                }),
                &[],
            )
            .to_bytes()
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

        let mut state = WatchState::new();
        let mut clock = Duration::ZERO;
        let mut cc = 0u8;
        let mut feed = |state: &mut WatchState, pid: u16, sec: &[u8], clock: &mut Duration| {
            state.feed_datagram(&ts_packet(pid, cc, sec), *clock);
            *clock += Duration::from_millis(1);
            cc = cc.wrapping_add(1) & 0x0F;
        };

        // Lock needs 5 consecutive sync bytes at the stride first.
        for _ in 0..5 {
            state.feed_datagram(&ts_packet(wk::NULL.value(), 0, &[]), clock);
            clock += Duration::from_millis(1);
        }
        feed(&mut state, wk::PAT.value(), &pat_bytes, &mut clock);
        feed(&mut state, PMT_PID, &pmt_bytes, &mut clock);

        for cycle in 0..CYCLES {
            // A distinct auto-return break per cycle: each must close itself.
            feed(
                &mut state,
                SCTE35_PID,
                &section(1000 + cycle, true, Some(true)),
                &mut clock,
            );
            // A distinct explicit out/in pair per cycle: each must close.
            let event_id = 2000 + cycle;
            feed(
                &mut state,
                SCTE35_PID,
                &section(event_id, true, None),
                &mut clock,
            );
            feed(
                &mut state,
                SCTE35_PID,
                &section(event_id, false, None),
                &mut clock,
            );
        }

        let text = state.render_prometheus();
        assert_eq!(
            metric_value(&text, "media_doctor_scte35_events_total"),
            Some(f64::from(CYCLES * 3)),
            "every section must be counted:
{text}",
        );
        assert_eq!(
            metric_value(&text, "media_doctor_scte35_open_events"),
            Some(0.0),
            "auto-return breaks and matched out/in pairs leave nothing open, and the map must not retain closed events:
{text}",
        );
    }

    /// Audit MD-W5, `watch` side: a TS-layer discontinuity resets the
    /// decode-timestamp baseline but must still feed the discontinuity
    /// packet's own payload — that packet normally carries the first PES
    /// after the break, and its timestamp baselines every later comparison.
    #[test]
    fn discontinuity_packet_payload_is_still_fed() {
        use broadcast_common::Serialize;
        use dvb_si::descriptors::DescriptorLoop;
        use dvb_si::tables::pat::{PatEntry, PatSection};
        use dvb_si::tables::pmt::{PmtSection, PmtStream};
        use mpeg_ts::pid::well_known as wk;
        use mpeg_ts::ts::TsHeader;

        const PMT_PID: u16 = 0x0100;
        const VIDEO_PID: u16 = 0x0101;

        /// A PES packet with the given PTS (`pts_dts_flags = 10`) — encoded
        /// with `mpeg-pes`'s own field serializer, never a hand-rolled one.
        fn pes_with_pts(stream_id: u8, pts: u64, payload: &[u8]) -> alloc::vec::Vec<u8> {
            const HDR_LEN: usize = 5; // 5-byte PTS, no stuffing
            let pes_len = 9 + HDR_LEN + payload.len();
            let mut pes = alloc::vec![0x00, 0x00, 0x01, stream_id];
            pes.extend_from_slice(&((pes_len - 6) as u16).to_be_bytes());
            pes.push(0x80); // flags1: marker + no special flags
            pes.push(0x80); // flags2: PTS_DTS_flags = 10
            pes.push(HDR_LEN as u8);
            pes.extend_from_slice(&mpeg_pes::Pts(pts).to_field_bytes());
            pes.extend_from_slice(payload);
            pes
        }

        fn ts_packet(pid: u16, cc: u8, pes: &[u8], discontinuity: bool) -> [u8; TS_PACKET_SIZE] {
            let mut pkt = [0xFFu8; TS_PACKET_SIZE];
            let header = TsHeader {
                tei: false,
                pusi: true,
                pid,
                scrambling: 0,
                has_adaptation: true,
                has_payload: true,
                continuity_counter: cc & 0x0F,
            };
            header.serialize_into(&mut pkt).unwrap();
            pkt[4] = 1; // adaptation_field_length: flags byte only
            pkt[5] = if discontinuity { 0x80 } else { 0x00 };
            pkt[6..6 + pes.len()].copy_from_slice(pes);
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
            VIDEO_PID,
            DescriptorLoop::new(&[]),
            alloc::vec![PmtStream {
                stream_type: StreamType::H264,
                elementary_pid: VIDEO_PID,
                es_info: DescriptorLoop::new(&[]),
            }],
        );
        let mut pmt_bytes = alloc::vec![0u8; pmt.serialized_len()];
        pmt.serialize_into(&mut pmt_bytes).unwrap();

        fn section_packet(pid: u16, cc: u8, section: &[u8]) -> [u8; TS_PACKET_SIZE] {
            let mut pkt = [0xFFu8; TS_PACKET_SIZE];
            let header = TsHeader {
                tei: false,
                pusi: true,
                pid,
                scrambling: 0,
                has_adaptation: false,
                has_payload: true,
                continuity_counter: cc & 0x0F,
            };
            header.serialize_into(&mut pkt).unwrap();
            pkt[4] = 0x00; // pointer_field
            pkt[5..5 + section.len()].copy_from_slice(section);
            pkt
        }

        let mut state = WatchState::new();
        let mut clock = Duration::ZERO;
        // Reach packet-stride lock first.
        for _ in 0..5 {
            let mut null = [0xFFu8; TS_PACKET_SIZE];
            let header = TsHeader {
                tei: false,
                pusi: false,
                pid: wk::NULL.value(),
                scrambling: 0,
                has_adaptation: false,
                has_payload: true,
                continuity_counter: 0,
            };
            header.serialize_into(&mut null).unwrap();
            state.feed_datagram(&null, clock);
            clock += Duration::from_millis(1);
        }
        state.feed_datagram(&section_packet(wk::PAT.value(), 0, &pat_bytes), clock);
        clock += Duration::from_millis(1);
        state.feed_datagram(&section_packet(PMT_PID, 0, &pmt_bytes), clock);
        clock += Duration::from_millis(1);

        // PTS 90_000, then a discontinuity carrying PTS 500_000, then a PTS
        // stepping BACKWARD from it. A PUSI packet completes the PES *before*
        // it, so the discontinuity is placed on the packet that follows a
        // clean 100_000 / 400_000 pair — the reset it performs must still
        // leave its own PTS 400_000 as the baseline for the backward step.
        for (cc, pts, disc) in [
            (0u8, 50_000u64, false),
            (1, 500_000, true),
            (2, 400_000, false),
            (3, 100_000, false),
        ] {
            let pes = pes_with_pts(0xE0, pts, &[0xAA; 16]);
            state.feed_datagram(&ts_packet(VIDEO_PID, cc, &pes, disc), clock);
            clock += Duration::from_millis(1);
        }

        let text = state.render_prometheus();
        assert_eq!(
            metric_value(&text, "media_doctor_pts_dts_anomalies_total"),
            Some(1.0),
            "the discontinuity packet's PTS must baseline the next comparison, so the backward step after it is reported:
{text}",
        );
    }

    /// Audit MD-W8: a PMT version change rebuilds the program's track set. A
    /// PID that moves from H.264 to AAC must be re-tracked as audio — before
    /// the fix it kept `EsKind::Video` forever and reported
    /// `codec-signalling-mismatch` on every audio access unit.
    #[test]
    fn pmt_version_change_retracks_a_pid_whose_codec_moved() {
        use broadcast_common::Serialize;
        use dvb_si::descriptors::DescriptorLoop;
        use dvb_si::tables::pat::{PatEntry, PatSection};
        use dvb_si::tables::pmt::{PmtSection, PmtStream};
        use mpeg_ts::pid::well_known as wk;
        use mpeg_ts::ts::TsHeader;

        const PMT_PID: u16 = 0x0100;
        const MIXED_PID: u16 = 0x0101;

        fn section_packet(pid: u16, cc: u8, section: &[u8]) -> [u8; TS_PACKET_SIZE] {
            let mut pkt = [0xFFu8; TS_PACKET_SIZE];
            let header = TsHeader {
                tei: false,
                pusi: true,
                pid,
                scrambling: 0,
                has_adaptation: false,
                has_payload: true,
                continuity_counter: cc & 0x0F,
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

        let pmt_for = |version: u8, stream_type: StreamType| -> alloc::vec::Vec<u8> {
            let pmt = PmtSection::new(
                1,
                version,
                true,
                0,
                0,
                MIXED_PID,
                DescriptorLoop::new(&[]),
                alloc::vec![PmtStream {
                    stream_type,
                    elementary_pid: MIXED_PID,
                    es_info: DescriptorLoop::new(&[]),
                }],
            );
            let mut bytes = alloc::vec![0u8; pmt.serialized_len()];
            pmt.serialize_into(&mut bytes).unwrap();
            bytes
        };

        /// One ADTS access unit (0xFFF… sync) in a PES packet.
        fn audio_pes() -> alloc::vec::Vec<u8> {
            let payload = [0xFFu8, 0xF1, 0x50, 0x80, 0x00, 0x1F, 0xFC];
            let pes_len = 9 + payload.len();
            let mut pes = alloc::vec![0x00, 0x00, 0x01, 0xC0];
            pes.extend_from_slice(&((pes_len - 6) as u16).to_be_bytes());
            pes.push(0x80);
            pes.push(0x00);
            pes.push(0x00);
            pes.extend_from_slice(&payload);
            pes
        }

        fn audio_packet(cc: u8) -> [u8; TS_PACKET_SIZE] {
            let mut pkt = [0xFFu8; TS_PACKET_SIZE];
            let header = TsHeader {
                tei: false,
                pusi: true,
                pid: MIXED_PID,
                scrambling: 0,
                has_adaptation: false,
                has_payload: true,
                continuity_counter: cc & 0x0F,
            };
            header.serialize_into(&mut pkt).unwrap();
            let pes = audio_pes();
            pkt[4..4 + pes.len()].copy_from_slice(&pes);
            pkt
        }

        let mut state = WatchState::new();
        let mut clock = Duration::ZERO;
        let mut feed = |state: &mut WatchState, packet: &[u8; TS_PACKET_SIZE]| {
            state.feed_datagram(packet, clock);
            clock += Duration::from_millis(1);
        };

        for _ in 0..5 {
            let mut null = [0xFFu8; TS_PACKET_SIZE];
            let header = TsHeader {
                tei: false,
                pusi: false,
                pid: wk::NULL.value(),
                scrambling: 0,
                has_adaptation: false,
                has_payload: true,
                continuity_counter: 0,
            };
            header.serialize_into(&mut null).unwrap();
            feed(&mut state, &null);
        }

        // Generation 0: the PID is declared H.264, so one video-framed unit
        // is not "structured" and the mismatch gauge reads 1.
        feed(&mut state, &section_packet(wk::PAT.value(), 0, &pat_bytes));
        feed(
            &mut state,
            &section_packet(PMT_PID, 0, &pmt_for(0, StreamType::H264)),
        );
        // A PES unit on the PID: the pre-change track treats it as video.
        feed(&mut state, &audio_packet(0));
        feed(&mut state, &audio_packet(1));

        let before = state.render_prometheus();
        assert!(
            before.contains(&alloc::format!(
                "media_doctor_codec_signalling_mismatch{{pid=\"0x{MIXED_PID:04X}\"}} 1"
            )),
            "the PID is declared H.264 and its unit is not video-framed:
{before}",
        );

        // Generation 1: the same PID is now declared AAC ADTS. Its earlier
        // state must be discarded — the audio framing now matches.
        feed(
            &mut state,
            &section_packet(PMT_PID, 1, &pmt_for(1, StreamType::AacAdts)),
        );
        feed(&mut state, &audio_packet(2));
        feed(&mut state, &audio_packet(3));
        // A PUSI packet completes the previous unit, so the last one needs a
        // following packet: reuse an audio packet.
        feed(&mut state, &audio_packet(4));

        let after = state.render_prometheus();
        assert!(
            after.contains(&alloc::format!(
                "media_doctor_codec_signalling_mismatch{{pid=\"0x{MIXED_PID:04X}\"}} 0"
            )),
            "after the PMT version change the PID is re-tracked as audio and its ADTS framing matches:
{after}",
        );
    }

    /// A PMT version bump whose PID declaration is unchanged must NOT reset
    /// the PID's running state (audit MD-W8 follow-up): the open SCTE-35
    /// event and the decode-timestamp baseline both survive.
    #[test]
    fn pmt_version_bump_with_unchanged_pids_keeps_state() {
        let mut state = WatchState::new();
        let mut clock = Duration::ZERO;
        let mut feed = |state: &mut WatchState, packet: &[u8; TS_PACKET_SIZE]| {
            state.feed_datagram(packet, clock);
            clock += Duration::from_millis(1);
        };
        lock_stride(&mut state, &mut feed);

        // Generation 0: an SCTE-35 cue PID and a video PID.
        feed(
            &mut state,
            &pmt_section_packet(WITH_CUE_PMT, PAT_PMT_PID, 0, &[0xC0]),
        );
        // An open SCTE-35 event on the cue PID.
        feed(&mut state, &scte35_packet(CUE_PID, 7, true));

        let before = state.render_prometheus();
        assert!(
            before.contains("media_doctor_scte35_open_events 1"),
            "the out event must be open:
{before}",
        );

        // Generation 1: identical declaration, version bumped.
        feed(
            &mut state,
            &pmt_section_packet(WITH_CUE_PMT, PAT_PMT_PID, 1, &[0xC0]),
        );

        let after = state.render_prometheus();
        assert!(
            after.contains("media_doctor_scte35_open_events 1"),
            "a version bump that changes no PID must not close the open              event:
{after}",
        );
    }

    /// A PID shared by two programs must stay tracked while either still
    /// declares it: one program dropping it must not remove the other's
    /// track.
    #[test]
    fn pid_shared_by_two_programs_survives_one_dropping_it() {
        let mut state = WatchState::new();
        let mut clock = Duration::ZERO;
        let mut feed = |state: &mut WatchState, packet: &[u8; TS_PACKET_SIZE]| {
            state.feed_datagram(packet, clock);
            clock += Duration::from_millis(1);
        };
        lock_stride(&mut state, &mut feed);

        // Program 1 declares SHARED_PID; program 2 declares it as well.
        feed(
            &mut state,
            &pmt_section_packet_for_program(1, PAT_PMT_PID, 0, SHARED_PID, &[0]),
        );
        feed(
            &mut state,
            &pmt_section_packet_for_program(2, PAT_PMT_PID, 0, SHARED_PID, &[1]),
        );

        // Program 1's next generation drops SHARED_PID entirely. Program 2's
        // declaration is untouched, so the PID must remain tracked.
        // Program 1's new generation declares an unrelated elementary PID, so
        // it no longer references SHARED_PID.
        let empty_pmt = pmt_section_packet_for_program(1, PAT_PMT_PID, 1, OTHER_PID, &[2]);
        feed(&mut state, &empty_pmt);

        // Video access units on the shared PID: it must still be tracked
        // (program 2 declares it), so the mismatch gauge is emitted once the
        // first unit completes (the second PUSI flushes the first).
        feed(&mut state, &video_packet(SHARED_PID, 0));
        feed(&mut state, &video_packet(SHARED_PID, 1));

        let text = state.render_prometheus();
        assert!(
            text.contains(&alloc::format!(
                "media_doctor_codec_signalling_mismatch{{pid=\"0x{SHARED_PID:04X}\"}}"
            )),
            "the PID is still declared by program 2 and must stay tracked:
{text}",
        );
    }

    /// A stream_type change is reconciled even when the version number the
    /// PMT carries did not move — the generation key includes the
    /// declaration, not just the version.
    ///
    /// Bites on the *kind*, not just the gauge: the PID is declared AAC and
    /// fed ADTS, so if it were still tracked as video its framing would never
    /// match. No video is fed, so nothing else can set `structured`.
    #[test]
    fn stream_type_change_without_version_bump_is_reconciled() {
        let mut state = WatchState::new();
        let mut clock = Duration::ZERO;
        let mut feed = |state: &mut WatchState, packet: &[u8; TS_PACKET_SIZE]| {
            state.feed_datagram(packet, clock);
            clock += Duration::from_millis(1);
        };
        lock_stride(&mut state, &mut feed);

        // First declaration: video. Nothing is fed on the PID yet.
        feed(
            &mut state,
            &pmt_section_packet(SINGLE_H264_PMT, PAT_PMT_PID, 3, &[0]),
        );

        // Re-declared as AAC at the SAME version number.
        state.apply_pmt_declaration(1, 3, &[(SHARED_PID, StreamType::AacAdts)]);

        // Only ADTS is fed. A track still holding EsKind::Video would never
        // become `structured`, so the gauge would read 1.
        feed(&mut state, &adts_packet(SHARED_PID, 0));
        feed(&mut state, &adts_packet(SHARED_PID, 1));

        let text = state.render_prometheus();
        assert!(
            text.contains(&alloc::format!(
                "media_doctor_codec_signalling_mismatch{{pid=\"0x{SHARED_PID:04X}\"}} 0"
            )),
            "the PID must be re-tracked as AAC so its ADTS framing matches; got:
{text}",
        );
    }

    /// A program that disappears from the PAT must release its PIDs: they
    /// stop being tracked, and its generation entry is cleared so a later
    /// re-appearance is treated as a fresh declaration.
    #[test]
    fn program_leaving_the_pat_releases_its_pids() {
        let mut state = WatchState::new();
        state.apply_pmt_declaration(1, 0, &[(SHARED_PID, StreamType::H264)]);
        assert!(
            state.es_tracks.contains_key(&SHARED_PID),
            "the PID must be tracked while program 1 declares it",
        );

        // The PAT no longer lists program 1.
        state.retain_programs(&[]);
        assert!(
            !state.es_tracks.contains_key(&SHARED_PID),
            "a program that left the PAT must stop being tracked",
        );
        assert!(
            !state.scte35_tracks.contains_key(&SHARED_PID),
            "and its SCTE-35 tracker, if any, must go too",
        );
        assert!(
            !state.pmt_generations.contains_key(&1),
            "its generation entry must be cleared",
        );
        assert!(
            !state.pid_refs.contains_key(&SHARED_PID),
            "and its reference count released",
        );
    }

    /// Two programs declaring the same PID with different `stream_type`s:
    /// the last declarer wins while both are present, and when it leaves the
    /// PID is restored to the type the remaining declarer gave it.
    #[test]
    fn shared_pid_type_is_restored_when_a_declarer_leaves() {
        let mut state = WatchState::new();

        // Program 1 declares AAC; program 2 then declares the same PID as
        // H.264, so H.264 is current.
        state.apply_pmt_declaration(1, 0, &[(SHARED_PID, StreamType::AacAdts)]);
        state.apply_pmt_declaration(2, 0, &[(SHARED_PID, StreamType::H264)]);
        // Program 2 leaves the PAT.
        state.retain_programs(&[1]);
        assert!(
            state.es_tracks.contains_key(&SHARED_PID),
            "program 1 still declares the PID, so it stays tracked",
        );
        assert_eq!(
            state.es_tracks.get(&SHARED_PID).map(|t| t.kind),
            Some(EsKind::AudioAdts),
            "the PID must be restored to the type program 1 declares",
        );
    }

    /// The same release, driven through the real PAT path: a second PAT
    /// generation that drops the program must release its PIDs via
    /// `feed_datagram`, not only via a direct `retain_programs` call.
    #[test]
    fn pat_dropping_a_program_releases_its_pids() {
        let mut state = WatchState::new();
        let mut clock = Duration::ZERO;
        let mut feed = |state: &mut WatchState, packet: &[u8; TS_PACKET_SIZE]| {
            state.feed_datagram(packet, clock);
            clock += Duration::from_millis(1);
        };
        lock_stride(&mut state, &mut feed);

        // Generation 0: the PAT lists program 1, whose PMT declares the PID.
        feed(&mut state, &pat_packet(&[1], 0));
        feed(
            &mut state,
            &pmt_section_packet_for_program(1, PAT_PMT_PID, 0, SHARED_PID, &[0]),
        );
        assert!(
            state.es_tracks.contains_key(&SHARED_PID),
            "the PID is declared by program 1 and must be tracked",
        );

        // Generation 1: the PAT no longer lists any program.
        feed(&mut state, &pat_packet(&[], 1));

        assert!(
            !state.es_tracks.contains_key(&SHARED_PID),
            "a PAT generation that drops the program must release its PIDs",
        );
        assert!(
            !state.pmt_generations.contains_key(&1),
            "and clear its generation entry",
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
