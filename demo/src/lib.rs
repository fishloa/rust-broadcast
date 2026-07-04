//! WASM browser "Wireshark for broadcast" analyzer — GitHub issue #602.
//!
//! Exposes a single [`analyze`] function that feeds a raw MPEG-TS capture
//! through:
//!
//! - [`dvb_si::demux::SiDemux`] — SI/PSI table reassembly + a service list.
//! - A PID map (packet counts + a kind label derived from PAT/PMT).
//! - PCR/PTS/DTS timing collection ([`mpeg_ts`] adaptation fields,
//!   [`mpeg_pes`] PES reassembly).
//! - [`dvb_conformance::ConformanceMonitor`] — ETSI TR 101 290.
//! - [`scte35_splice::SpliceInfoSection`] — a splice-command timeline on
//!   every PMT-declared SCTE-35 PID.
//!
//! ...and returns ONE JSON object driving every panel of the demo UI. Every
//! sub-object reuses that crate's own serde shape (e.g. `serde_json::to_value`
//! on an `AnyTableSection` or a `SpliceInfoSection`) — nothing here invents a
//! second schema for wire data. The exceptions are the PID map, timing arrays
//! and conformance report, which have no crate-native serde shape of their
//! own and are this module's design.
//!
//! The whole point: parse containers/protocols/SI, never the codec
//! bitstream; 100% client-side (nothing uploaded, no transcode).

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use broadcast_common::Parse;
use dvb_conformance::{ConformanceEvent, ConformanceMonitor, Priority};
use dvb_si::demux::SiDemux;
use dvb_si::tables::AnyTableSection;
use dvb_si::tables::pmt::StreamType;
use mpeg_pes::{PesAssembler, PesPacket};
use mpeg_ts::pid::well_known as wk;
use mpeg_ts::ts::{SectionReassembler, TS_PACKET_SIZE, TsPacket};
use scte35_splice::SpliceInfoSection;
use serde::Serialize;
use wasm_bindgen::prelude::*;

// ───────────────────────────── caps ───────────────────────────────────────
//
// Engineering limits (not spec values) that keep the JSON payload — and the
// browser tab's memory — bounded on very long captures. Each cap is paired
// with a `truncated` flag in the output so the UI can say so honestly.

/// Maximum PCR samples collected.
const MAX_PCR_SAMPLES: usize = 20_000;
/// Maximum PTS/DTS samples collected.
const MAX_TIMESTAMP_SAMPLES: usize = 20_000;
/// Maximum SCTE-35 splice events collected.
const MAX_SCTE35_EVENTS: usize = 5_000;
/// Maximum detail strings kept per TR 101 290 indicator.
const MAX_CONFORMANCE_SAMPLES_PER_INDICATOR: usize = 8;

// ───────────────────────────── PID roles ──────────────────────────────────

/// Coarse classification of a PID's purpose, derived from PAT/PMT.
///
/// Ordering matters: [`upgrade_role`] only replaces a PID's current role with
/// a *more specific* one (lower [`PidRole::rank`]), so e.g. a video PID that
/// also happens to be the program's PCR PID stays labelled `Video` — its
/// PCR-bearing-ness is reported separately via [`PidEntry::has_pcr`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum PidRole {
    Pat,
    Pmt,
    Scte35,
    Video,
    Audio,
    Si,
    Pcr,
    Null,
    Other,
}

impl PidRole {
    /// Lower rank wins when merging roles for the same PID.
    fn rank(self) -> u8 {
        match self {
            Self::Pat => 0,
            Self::Pmt => 1,
            Self::Scte35 => 2,
            Self::Video => 3,
            Self::Audio => 4,
            Self::Si => 5,
            Self::Pcr => 6,
            Self::Null => 7,
            Self::Other => 8,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Pat => "PAT",
            Self::Pmt => "PMT",
            Self::Scte35 => "SCTE-35",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Si => "SI",
            Self::Pcr => "PCR",
            Self::Null => "Null",
            Self::Other => "Other",
        }
    }
}

/// Well-known DVB/MPEG-2 SI PIDs (ETSI EN 300 468 §5.1.3 Table 1), other than
/// PAT (0x0000, handled separately) and the NULL stuffing PID (also handled
/// separately). Every value here is `mpeg_ts::pid::well_known`'s own const —
/// no re-derived magic numbers.
const WELL_KNOWN_SI_PIDS: [u16; 12] = [
    wk::CAT.value(),
    wk::TSDT.value(),
    wk::IPMP_CIT.value(),
    wk::NIT.value(),
    wk::SDT_BAT.value(),
    wk::EIT.value(),
    wk::RST.value(),
    wk::TDT_TOT.value(),
    wk::NETWORK_SYNC.value(),
    wk::RNT.value(),
    wk::SAT.value(),
    wk::ATSC_PSIP.value(),
];

fn classify_stream_type(st: StreamType) -> PidRole {
    if st == StreamType::Scte35 {
        PidRole::Scte35
    } else if st.is_video() {
        PidRole::Video
    } else if st.is_audio() {
        PidRole::Audio
    } else {
        PidRole::Other
    }
}

/// Per-PID accumulator built up over the whole packet pass.
#[derive(Default)]
struct PidAccum {
    packets: u64,
    role: Option<PidRole>,
    has_pcr: bool,
    stream_type_name: Option<&'static str>,
}

/// Replace `accum`'s role only if `candidate` is more specific.
fn upgrade_role(accum: &mut PidAccum, candidate: PidRole) {
    let better = match accum.role {
        None => true,
        Some(existing) => candidate.rank() < existing.rank(),
    };
    if better {
        accum.role = Some(candidate);
    }
}

// ───────────────────────────── output types ──────────────────────────────

/// One entry in the `tables` array — the serde-serialised `AnyTableSection`
/// value with its PID prepended for context.
#[derive(Serialize)]
struct TableEntry {
    /// PID the section was carried on (decimal).
    pid: u16,
    /// Parsed section, using dvb-si's own serde shape (camelCase external tag).
    /// This is NOT an invented schema — it is the same JSON that
    /// `serde_json::to_value(&AnyTableSection::…)` produces.
    section: serde_json::Value,
}

/// One service extracted from the SDT.
#[derive(Serialize)]
struct ServiceEntry {
    service_id: u16,
    provider_name: String,
    service_name: String,
    service_type: String,
}

/// One row of the PID map panel.
#[derive(Serialize)]
struct PidEntry {
    pid: u16,
    packets: u64,
    /// "PAT" | "PMT" | "SI" | "PCR" | "Video" | "Audio" | "SCTE-35" | "Null" | "Other".
    kind: &'static str,
    /// PMT stream_type name (ITU-T H.222.0 Table 2-34), when this PID is an
    /// elementary stream declared by a PMT.
    stream_type: Option<&'static str>,
    /// Whether this PID has been observed carrying an adaptation-field PCR.
    has_pcr: bool,
}

/// One PCR observation (adaptation field, ISO/IEC 13818-1 §2.4.3.5).
#[derive(Serialize)]
struct PcrSample {
    packet_index: u64,
    pid: u16,
    /// Full 27 MHz value: `base * 300 + extension`.
    pcr_27mhz: u64,
    seconds: f64,
}

/// One PTS or DTS observation from a reassembled PES header.
#[derive(Serialize)]
struct TimestampSample {
    packet_index: u64,
    pid: u16,
    /// "pts" | "dts".
    kind: &'static str,
    /// Raw 33-bit 90 kHz value.
    ticks: u64,
    seconds: f64,
    /// The PCR-derived reference clock at this packet, in seconds (or a
    /// synthetic monotonic ramp before the first PCR is observed).
    clock_seconds: f64,
    /// `seconds - clock_seconds` — the value to chart as PTS/DTS drift.
    drift_seconds: f64,
}

/// PCR/PTS/DTS timing arrays for the Timing panel.
#[derive(Serialize)]
struct Timing {
    pcr_samples: Vec<PcrSample>,
    pts_samples: Vec<TimestampSample>,
    /// True if any of the arrays above hit its cap and further samples were
    /// dropped.
    truncated: bool,
    /// PES packets that failed to parse on a tracked video/audio PID.
    pes_parse_errors: u64,
}

/// One SCTE-35 `splice_info_section`, in the crate's own serde shape.
#[derive(Serialize)]
struct Scte35Entry {
    pid: u16,
    packet_index: u64,
    section: serde_json::Value,
}

/// The SCTE-35 splice timeline.
#[derive(Serialize)]
struct Scte35Report {
    events: Vec<Scte35Entry>,
    truncated: bool,
    parse_errors: u64,
}

/// One TR 101 290 indicator's aggregated counts.
#[derive(Serialize)]
struct IndicatorGroup {
    indicator: &'static str,
    clause: &'static str,
    count: u64,
    pids: Vec<u16>,
    sample_details: Vec<String>,
}

/// All indicators for one TR 101 290 priority tier.
#[derive(Serialize)]
struct PriorityGroup {
    priority: &'static str,
    indicators: Vec<IndicatorGroup>,
}

#[derive(Serialize)]
struct ConformanceStats {
    packets: u64,
    events: u64,
    in_sync: bool,
}

/// The TR 101 290 conformance report.
#[derive(Serialize)]
struct ConformanceReport {
    stats: ConformanceStats,
    by_priority: Vec<PriorityGroup>,
}

/// Top-level JSON object returned by [`analyze`].
#[derive(Serialize)]
struct AnalysisResult {
    /// Well-formed, 0x47-synced 188-byte packets processed.
    packets_fed: u64,
    /// Chunks that were not a well-formed 188-byte sync'd TS packet.
    parse_errors: u64,
    /// Sections that failed CRC validation (from [`SiDemux`]).
    crc_errors: u64,
    pid_map: Vec<PidEntry>,
    timing: Timing,
    tables: Vec<TableEntry>,
    services: Vec<ServiceEntry>,
    conformance: ConformanceReport,
    scte35: Scte35Report,
}

// ───────────────────────────── conformance grouping ───────────────────────

/// Per-indicator accumulator for the conformance report.
struct IndicatorAccum {
    clause: &'static str,
    priority: Priority,
    count: u64,
    pids: BTreeSet<u16>,
    samples: Vec<String>,
}

fn record_conformance_event(
    acc: &mut BTreeMap<&'static str, IndicatorAccum>,
    ev: &ConformanceEvent,
) {
    let name = ev.indicator.name();
    let entry = acc.entry(name).or_insert_with(|| IndicatorAccum {
        clause: ev.indicator.clause(),
        priority: ev.priority,
        count: 0,
        pids: BTreeSet::new(),
        samples: Vec::new(),
    });
    entry.count += 1;
    if let Some(pid) = ev.pid {
        entry.pids.insert(pid);
    }
    if entry.samples.len() < MAX_CONFORMANCE_SAMPLES_PER_INDICATOR {
        entry
            .samples
            .push(format!("t={:.3}s: {}", ev.at.as_secs_f64(), ev.detail));
    }
}

fn build_conformance_report(
    acc: BTreeMap<&'static str, IndicatorAccum>,
    stats: dvb_conformance::Stats,
) -> ConformanceReport {
    let mut by_priority_name: BTreeMap<&'static str, Vec<IndicatorGroup>> = BTreeMap::new();
    for (name, ia) in acc {
        let group = IndicatorGroup {
            indicator: name,
            clause: ia.clause,
            count: ia.count,
            pids: ia.pids.into_iter().collect(),
            sample_details: ia.samples,
        };
        by_priority_name
            .entry(ia.priority.name())
            .or_default()
            .push(group);
    }

    let mut by_priority = Vec::new();
    for priority in [Priority::First, Priority::Second, Priority::Third] {
        if let Some(mut indicators) = by_priority_name.remove(priority.name()) {
            indicators.sort_by(|a, b| a.indicator.cmp(b.indicator));
            by_priority.push(PriorityGroup {
                priority: priority.name(),
                indicators,
            });
        }
    }

    ConformanceReport {
        stats: ConformanceStats {
            packets: stats.packets,
            events: stats.events,
            in_sync: stats.in_sync,
        },
        by_priority,
    }
}

// ───────────────────────────── PES helpers ────────────────────────────────

fn process_pes(
    bytes: &[u8],
    pid: u16,
    packet_index: u64,
    clock_seconds: f64,
    pts_samples: &mut Vec<TimestampSample>,
    pes_parse_errors: &mut u64,
    truncated: &mut bool,
) {
    match PesPacket::parse(bytes) {
        Ok(pes) => {
            if let Some(header) = &pes.header {
                if let Some(pts) = header.pts {
                    push_timestamp(
                        pts_samples,
                        packet_index,
                        pid,
                        "pts",
                        pts.ticks(),
                        pts.seconds(),
                        clock_seconds,
                        truncated,
                    );
                }
                if let Some(dts) = header.dts {
                    push_timestamp(
                        pts_samples,
                        packet_index,
                        pid,
                        "dts",
                        dts.ticks(),
                        dts.seconds(),
                        clock_seconds,
                        truncated,
                    );
                }
            }
        }
        Err(_) => *pes_parse_errors += 1,
    }
}

#[allow(clippy::too_many_arguments)]
fn push_timestamp(
    samples: &mut Vec<TimestampSample>,
    packet_index: u64,
    pid: u16,
    kind: &'static str,
    ticks: u64,
    seconds: f64,
    clock_seconds: f64,
    truncated: &mut bool,
) {
    if samples.len() < MAX_TIMESTAMP_SAMPLES {
        samples.push(TimestampSample {
            packet_index,
            pid,
            kind,
            ticks,
            seconds,
            clock_seconds,
            drift_seconds: seconds - clock_seconds,
        });
    } else {
        *truncated = true;
    }
}

// ───────────────────────────── service extraction ─────────────────────────

/// Walk the collected table entries and extract service info from SDT sections.
///
/// The JSON shape produced by dvb-si's serde for an SDT is:
/// `{"sdtSection": { "services": [ { "serviceId": N, "descriptors": [...] } ] }}`
/// We navigate that structure to pull out service_descriptor fields.
fn extract_services(tables: &[TableEntry]) -> Vec<ServiceEntry> {
    let mut services = Vec::new();

    for entry in tables {
        // Only SDT sections (actual or other).
        let sdt_obj = match entry.section.get("sdtSection") {
            Some(v) => v,
            None => continue,
        };

        let svc_list = match sdt_obj.get("services").and_then(|v| v.as_array()) {
            Some(a) => a,
            None => continue,
        };

        for svc in svc_list {
            let service_id = svc
                .get("service_id")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u16;

            let descriptors = match svc.get("descriptors").and_then(|v| v.as_array()) {
                Some(a) => a,
                None => continue,
            };

            for desc in descriptors {
                // dvb-si serializes ServiceDescriptor as `{"service": {...}}`
                // (camelCase external tag from the AnyDescriptor enum).
                let svc_desc = match desc.get("service") {
                    Some(v) => v,
                    None => continue,
                };

                let provider_name = svc_desc
                    .get("provider_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let service_name = svc_desc
                    .get("service_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                // ServiceType serializes as the Rust variant name by dvb-si serde.
                let service_type = svc_desc
                    .get("service_type")
                    .map(|v| {
                        if let Some(s) = v.as_str() {
                            s.to_string()
                        } else {
                            v.to_string()
                        }
                    })
                    .unwrap_or_default();

                services.push(ServiceEntry {
                    service_id,
                    provider_name,
                    service_name,
                    service_type,
                });
                break; // one service_descriptor per service entry is enough
            }
        }
    }

    services
}

// ───────────────────────────── analysis pass ──────────────────────────────

/// Run the full analysis pass over a raw MPEG-TS byte buffer.
///
/// Never panics on malformed input: bad packets, bad sections, bad PES and
/// bad SCTE-35 sections are all counted and skipped, not raised.
fn analyze_impl(bytes: &[u8]) -> AnalysisResult {
    let mut demux = SiDemux::builder().build();
    let mut conformance = ConformanceMonitor::new();

    let mut tables: Vec<TableEntry> = Vec::new();
    let mut pid_stats: BTreeMap<u16, PidAccum> = BTreeMap::new();
    let mut scte35_pids: BTreeMap<u16, SectionReassembler> = BTreeMap::new();
    let mut pes_pids: BTreeMap<u16, PesAssembler> = BTreeMap::new();
    let mut conformance_acc: BTreeMap<&'static str, IndicatorAccum> = BTreeMap::new();

    let mut pcr_samples: Vec<PcrSample> = Vec::new();
    let mut pts_samples: Vec<TimestampSample> = Vec::new();
    let mut scte35_events: Vec<Scte35Entry> = Vec::new();

    let mut parse_errors: u64 = 0;
    let mut packets_fed: u64 = 0;
    let mut scte35_parse_errors: u64 = 0;
    let mut pes_parse_errors: u64 = 0;
    let mut timing_truncated = false;
    let mut scte35_truncated = false;

    // Reference clock: anchored on observed PCR values (27 MHz -> seconds);
    // before the first PCR is seen, a tiny synthetic per-packet increment
    // keeps it strictly monotonic for the conformance monitor.
    let mut clock = Duration::ZERO;
    let mut pcr_anchor: Option<(u64, Duration)> = None;

    for (idx, chunk) in bytes.chunks(TS_PACKET_SIZE).enumerate() {
        let idx = idx as u64;

        let ts_packet = match TsPacket::parse(chunk) {
            Ok(p) => p,
            Err(_) => {
                parse_errors += 1;
                continue;
            }
        };
        packets_fed += 1;
        let pid = ts_packet.header.pid;

        {
            let accum = pid_stats.entry(pid).or_default();
            accum.packets += 1;
        }
        if pid == wk::PAT.value() {
            upgrade_role(pid_stats.entry(pid).or_default(), PidRole::Pat);
        } else if pid == wk::NULL.value() {
            upgrade_role(pid_stats.entry(pid).or_default(), PidRole::Null);
        } else if WELL_KNOWN_SI_PIDS.contains(&pid) {
            upgrade_role(pid_stats.entry(pid).or_default(), PidRole::Si);
        }

        // ── PCR extraction + reference clock ────────────────────────────
        if let Some(Ok(af)) = ts_packet.adaptation_field() {
            if let Some(pcr) = af.pcr {
                let pcr_27mhz = pcr.as_27mhz();
                pid_stats.entry(pid).or_default().has_pcr = true;

                if let Some((anchor_val, anchor_t)) = pcr_anchor {
                    if pcr_27mhz >= anchor_val {
                        let delta_secs = (pcr_27mhz - anchor_val) as f64 / 27_000_000.0;
                        let candidate = anchor_t + Duration::from_secs_f64(delta_secs);
                        if candidate > clock {
                            clock = candidate;
                        }
                    }
                    // Else the PCR moved backward (discontinuity or wrap):
                    // re-anchor at the current clock without moving it back.
                }
                pcr_anchor = Some((pcr_27mhz, clock));

                if pcr_samples.len() < MAX_PCR_SAMPLES {
                    pcr_samples.push(PcrSample {
                        packet_index: idx,
                        pid,
                        pcr_27mhz,
                        seconds: pcr_27mhz as f64 / 27_000_000.0,
                    });
                } else {
                    timing_truncated = true;
                }
            }
        }
        if pcr_anchor.is_none() {
            clock += Duration::from_nanos(1);
        }

        // ── TR 101 290 conformance ───────────────────────────────────────
        for ev in conformance.feed(chunk, clock) {
            record_conformance_event(&mut conformance_acc, ev);
        }

        // ── SI/PSI demux: tables + PID roles from PAT/PMT ───────────────
        let events: Vec<_> = demux.feed(chunk).collect();
        for ev in events {
            let ev_pid = ev.pid().value();
            let section_result = ev.table_section();

            match &section_result {
                Ok(AnyTableSection::PatSection(pat)) => {
                    upgrade_role(pid_stats.entry(ev_pid).or_default(), PidRole::Pat);
                    for pat_entry in &pat.entries {
                        if pat_entry.program_number != 0 {
                            upgrade_role(
                                pid_stats.entry(pat_entry.pid).or_default(),
                                PidRole::Pmt,
                            );
                        } else {
                            // program_number == 0 -> this entry's PID is the
                            // network_PID (NIT), not a PMT.
                            upgrade_role(pid_stats.entry(pat_entry.pid).or_default(), PidRole::Si);
                        }
                    }
                }
                Ok(AnyTableSection::PmtSection(pmt)) => {
                    // 0x1FFF is the spec sentinel for "no PCR for this
                    // program" — don't mislabel the NULL PID as PCR.
                    if pmt.pcr_pid != wk::NULL.value() {
                        upgrade_role(pid_stats.entry(pmt.pcr_pid).or_default(), PidRole::Pcr);
                    }
                    for stream in &pmt.streams {
                        let kind = classify_stream_type(stream.stream_type);
                        let accum = pid_stats.entry(stream.elementary_pid).or_default();
                        upgrade_role(accum, kind);
                        accum.stream_type_name = Some(stream.stream_type.name());

                        if stream.stream_type == StreamType::Scte35 {
                            scte35_pids.entry(stream.elementary_pid).or_default();
                        }
                        if matches!(kind, PidRole::Video | PidRole::Audio) {
                            pes_pids.entry(stream.elementary_pid).or_default();
                        }
                    }
                }
                _ => {}
            }

            let section_json = match section_result {
                Ok(ts) => match serde_json::to_value(&ts) {
                    Ok(v) => v,
                    Err(e) => serde_json::json!({ "serializeError": e.to_string() }),
                },
                Err(e) => serde_json::json!({ "parseError": e.to_string() }),
            };
            tables.push(TableEntry {
                pid: ev_pid,
                section: section_json,
            });
        }

        // ── SCTE-35 splice timeline ──────────────────────────────────────
        if let Some(reasm) = scte35_pids.get_mut(&pid) {
            let payload = ts_packet.payload.unwrap_or(&[]);
            reasm.feed(payload, ts_packet.header.pusi);
            while let Some(section) = reasm.pop_section() {
                match SpliceInfoSection::parse(&section[..]) {
                    Ok(splice) => {
                        let section_json = serde_json::to_value(&splice).unwrap_or_else(|e| {
                            serde_json::json!({ "serializeError": e.to_string() })
                        });
                        if scte35_events.len() < MAX_SCTE35_EVENTS {
                            scte35_events.push(Scte35Entry {
                                pid,
                                packet_index: idx,
                                section: section_json,
                            });
                        } else {
                            scte35_truncated = true;
                        }
                    }
                    Err(_) => scte35_parse_errors += 1,
                }
            }
        }

        // ── PTS/DTS timing on tracked video/audio PIDs ───────────────────
        if let Some(asm) = pes_pids.get_mut(&pid) {
            if let Some(payload) = ts_packet.payload {
                if let Some(completed) = asm.feed(ts_packet.header.pusi, payload) {
                    process_pes(
                        &completed,
                        pid,
                        idx,
                        clock.as_secs_f64(),
                        &mut pts_samples,
                        &mut pes_parse_errors,
                        &mut timing_truncated,
                    );
                }
            }
        }
    }

    // Flush any PES packet still buffered at end of stream.
    let clock_seconds = clock.as_secs_f64();
    for (&pid, asm) in pes_pids.iter_mut() {
        if let Some(completed) = asm.flush() {
            process_pes(
                &completed,
                pid,
                packets_fed.saturating_sub(1),
                clock_seconds,
                &mut pts_samples,
                &mut pes_parse_errors,
                &mut timing_truncated,
            );
        }
    }

    let demux_stats = demux.stats();
    let conformance_stats = conformance.stats();
    let services = extract_services(&tables);

    let pid_map: Vec<PidEntry> = pid_stats
        .into_iter()
        .map(|(pid, acc)| PidEntry {
            pid,
            packets: acc.packets,
            kind: acc.role.unwrap_or(PidRole::Other).label(),
            stream_type: acc.stream_type_name,
            has_pcr: acc.has_pcr,
        })
        .collect();

    AnalysisResult {
        packets_fed,
        parse_errors,
        crc_errors: demux_stats.crc_failures,
        pid_map,
        timing: Timing {
            pcr_samples,
            pts_samples,
            truncated: timing_truncated,
            pes_parse_errors,
        },
        tables,
        services,
        conformance: build_conformance_report(conformance_acc, conformance_stats),
        scte35: Scte35Report {
            events: scte35_events,
            truncated: scte35_truncated,
            parse_errors: scte35_parse_errors,
        },
    }
}

// ───────────────────────────── wasm export ────────────────────────────────

/// Analyze a raw MPEG-TS byte buffer and return a single JSON object driving
/// every panel of the demo UI: PID map, PCR/PTS timing, the SI/PSI table
/// tree + service list, a TR 101 290 conformance report, and an SCTE-35
/// splice timeline.
///
/// Never panics. On any bad packet, section, PES, or SCTE-35 message the
/// error is counted and processing continues.
#[wasm_bindgen]
pub fn analyze(bytes: &[u8]) -> String {
    let result = analyze_impl(bytes);
    serde_json::to_string(&result).unwrap_or_else(|e| format!("{{\"internalError\":\"{e}\"}}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = format!(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/{}"), name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read fixture {path}: {e}"))
    }

    /// The exit-gate test: a real broadcast capture must produce a
    /// non-empty PID map and at least one SI table, and the wasm-facing JSON
    /// string must parse back and carry the documented top-level keys.
    #[test]
    fn analyzes_real_fixture_and_produces_expected_shape() {
        let bytes = fixture("m6-single.ts");

        let result = analyze_impl(&bytes);
        assert!(result.packets_fed > 0, "expected packets to be fed");
        assert!(
            !result.pid_map.is_empty(),
            "pid_map must be non-empty for a real capture"
        );
        assert!(
            !result.tables.is_empty(),
            "tables must be non-empty for a real capture"
        );

        let pat_entry = result
            .pid_map
            .iter()
            .find(|e| e.pid == wk::PAT.value())
            .expect("PID 0x0000 (PAT) must be observed");
        assert_eq!(pat_entry.kind, "PAT");

        let video_entry = result.pid_map.iter().find(|e| e.kind == "Video");
        assert!(
            video_entry.is_some(),
            "expected at least one Video PID classified from the PMT"
        );

        // Round-trip through the actual wasm-facing entry point.
        let json = analyze(&bytes);
        let value: serde_json::Value =
            serde_json::from_str(&json).expect("analyze() must produce valid JSON");
        for key in ["pid_map", "conformance", "tables", "timing", "scte35", "services"] {
            assert!(value.get(key).is_some(), "missing top-level key {key}");
        }
        assert!(
            !value["pid_map"].as_array().unwrap().is_empty(),
            "pid_map JSON array must be non-empty"
        );
        assert!(
            !value["tables"].as_array().unwrap().is_empty(),
            "tables JSON array must be non-empty"
        );
        assert!(value["conformance"]["stats"]["packets"].as_u64().unwrap() > 0);
    }

    /// Garbage input must never panic and must report the failure honestly.
    #[test]
    fn garbage_input_never_panics() {
        let garbage = vec![0u8; 4096];
        let result = analyze_impl(&garbage);
        assert_eq!(result.packets_fed, 0);
        assert!(result.parse_errors > 0);
        assert!(result.pid_map.is_empty());
        assert!(result.tables.is_empty());
    }

    /// End-to-end SCTE-35 pipeline test: PAT -> PMT (declaring stream_type
    /// 0x86 on a PID) -> a real `splice_info_section` on that PID, each
    /// section built and serialized by its *own* crate (not hand-rolled
    /// bytes), fed through the full `analyze_impl` packet loop. None of the
    /// committed real captures happen to declare an SCTE-35 stream in their
    /// PMT, so this synthesizes the PMT-declared-PID scenario the shipped
    /// fixtures don't cover, while keeping every section spec-correct by
    /// construction (parse+serialize round-trip already covered by each
    /// crate's own tests).
    #[test]
    fn scte35_declared_in_pmt_is_reassembled_and_parsed() {
        use broadcast_common::Serialize;
        use dvb_si::descriptors::DescriptorLoop;
        use dvb_si::tables::pat::{PatEntry, PatSection};
        use dvb_si::tables::pmt::{PmtSection, PmtStream};
        use mpeg_ts::ts::TsHeader;
        use scte35_splice::SpliceInfoSection;
        use scte35_splice::commands::AnyCommand;

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
            assert!(5 + section.len() <= TS_PACKET_SIZE, "section too big for one packet");
            pkt[5..5 + section.len()].copy_from_slice(section);
            pkt
        }

        let pat = PatSection {
            transport_stream_id: 1,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            entries: vec![PatEntry {
                program_number: 1,
                pid: PMT_PID,
            }],
        };
        let mut pat_bytes = vec![0u8; pat.serialized_len()];
        pat.serialize_into(&mut pat_bytes).unwrap();

        let pmt = PmtSection::new(
            1,
            0,
            true,
            0,
            0,
            wk::NULL.value(), // no PCR for this program
            DescriptorLoop::new(&[]),
            vec![PmtStream {
                stream_type: StreamType::Scte35,
                elementary_pid: SCTE35_PID,
                es_info: DescriptorLoop::new(&[]),
            }],
        );
        let mut pmt_bytes = vec![0u8; pmt.serialized_len()];
        pmt.serialize_into(&mut pmt_bytes).unwrap();

        let splice = SpliceInfoSection::new_clear(AnyCommand::SpliceNull(Default::default()), &[]);
        let splice_bytes = splice.to_bytes();

        let mut ts = Vec::new();
        ts.extend_from_slice(&ts_packet(wk::PAT.value(), &pat_bytes));
        ts.extend_from_slice(&ts_packet(PMT_PID, &pmt_bytes));
        ts.extend_from_slice(&ts_packet(SCTE35_PID, &splice_bytes));

        let result = analyze_impl(&ts);

        assert_eq!(result.parse_errors, 0);
        assert_eq!(result.scte35.parse_errors, 0);
        assert_eq!(
            result.scte35.events.len(),
            1,
            "the splice_null section on the PMT-declared SCTE-35 PID must be reassembled and parsed"
        );
        assert_eq!(result.scte35.events[0].pid, SCTE35_PID);
        assert!(
            result.scte35.events[0]
                .section
                .get("clear")
                .and_then(|c| c.get("command"))
                .and_then(|c| c.get("spliceNull"))
                .is_some(),
            "expected the crate's own serde shape for a splice_null command, got {:?}",
            result.scte35.events[0].section
        );

        let scte_pid_entry = result
            .pid_map
            .iter()
            .find(|e| e.pid == SCTE35_PID)
            .expect("SCTE-35 PID must appear in the pid map");
        assert_eq!(scte_pid_entry.kind, "SCTE-35");
    }
}
