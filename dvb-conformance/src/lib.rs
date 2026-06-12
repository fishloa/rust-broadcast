//! ETSI TR 101 290 v1.4.1 transport-stream conformance monitor.
//!
//! Implements the **first-priority** (Table 5.0a, indicators 1.1–1.6) and
//! **second-priority** (Table 5.0b, indicators 2.1–2.3b, 2.5–2.6) indicator
//! sets. Indicator 2.4 (PCR_accuracy_error) is intentionally excluded — it
//! requires hardware arrival timestamps not available under the caller-supplied-
//! time model. SI-repetition (Priority 3) indicators will be added in a future
//! release.
//!
//! # Caller-supplied time
//!
//! [`ConformanceMonitor::feed`] takes a [`core::time::Duration`] timestamp
//! alongside each TS packet. All presence/absence timeout checks (1.3.a, 1.5.a,
//! 1.6, 2.3a, 2.3b, 2.5) are evaluated against this clock. The caller must
//! ensure that timestamps are **monotonic non-decreasing** across calls; the
//! monitor does not enforce this but non-monotonic timestamps will produce
//! spurious events.
//!
//! # References
//!
//! - ETSI TR 101 290 v1.4.1 (2023-05), §5.2.1, Table 5.0a
//! - ETSI TR 101 290 v1.4.1 (2023-05), §5.2.2, Table 5.0b
//! - ISO/IEC 13818-1 (MPEG-2 Systems)

use core::time::Duration;
use std::collections::HashMap;

use dvb_common::Parse;
use dvb_si::section::Section;
use dvb_si::tables::pat::{PatSection, TABLE_ID as PAT_TABLE_ID};
use dvb_si::tables::pmt::PmtSection;
use dvb_si::ts::{SectionReassembler, TsPacket};

// ── Named PID constants ─────────────────────────────────────────────────────

/// PID 0x0000 — Program Association Table (ISO/IEC 13818-1 §2.4.4.3).
const PID_PAT: u16 = 0x0000;
/// PID 0x0001 — Conditional Access Table (ISO/IEC 13818-1 §2.4.4.5).
const PID_CAT: u16 = 0x0001;
/// PID 0x0010 — Network Information Table (EN 300 468 §5.2.1).
const PID_NIT: u16 = 0x0010;
/// PID 0x0011 — SDT/BAT (EN 300 468 §5.2.2 / §5.2.3).
const PID_SDT_BAT: u16 = 0x0011;
/// PID 0x0012 — Event Information Table (EN 300 468 §5.2.4).
const PID_EIT: u16 = 0x0012;
/// PID 0x0014 — TDT/TOT (EN 300 468 §5.2.5 / §5.2.6).
const PID_TDT_TOT: u16 = 0x0014;
/// PID 0x1FFF — Null/padding packets (ISO/IEC 13818-1 §2.4.3.3).
const PID_NULL: u16 = 0x1FFF;

/// Sync byte value (ISO/IEC 13818-1 §2.4.3.3).
const SYNC_BYTE: u8 = 0x47;

/// Well-known SI/PSI PIDs on which CRC-checked long-form sections appear.
const SI_PIDS: [u16; 6] = [PID_PAT, PID_CAT, PID_NIT, PID_SDT_BAT, PID_EIT, PID_TDT_TOT];

// ── Default timing constants ────────────────────────────────────────────────

/// TR 101 290 v1.4.1 Table 5.0a note 3 / TS 101 154 §4.1.7 — PAT maximum
/// interval (0.5 s per Table 5.0a row 1.3.a; TS 101 154 recommends ≤ 100 ms).
const DEFAULT_PAT_MAX_INTERVAL_MS: u64 = 500;

/// TR 101 290 v1.4.1 Table 5.0a row 1.5.a / note 3 — PMT maximum interval.
const DEFAULT_PMT_MAX_INTERVAL_MS: u64 = 500;

/// TR 101 290 v1.4.1 §5.2.1 accompanying text (1.6) — PID_error period.
const DEFAULT_PID_ERROR_PERIOD_SECS: u64 = 5;

/// TR 101 290 v1.4.1 §5.2.1 accompanying text (1.1) — sync acquisition
/// threshold: five consecutive correct sync bytes.
const DEFAULT_SYNC_ACQUIRE_PACKETS: u8 = 5;

/// TR 101 290 v1.4.1 §5.2.1 accompanying text (1.1) — sync loss threshold:
/// two or more consecutive corrupted sync bytes.
const DEFAULT_SYNC_LOSS_PACKETS: u8 = 2;

/// TR 101 290 v1.4.1 Table 5.0b indicator 2.3a / note 2 — PCR maximum
/// repetition interval (100 ms; note 2 removed the 40 ms limit).
const DEFAULT_PCR_REPETITION_LIMIT_MS: u64 = 100;

/// TR 101 290 v1.4.1 Table 5.0b indicator 2.3b — PCR discontinuity indicator
/// maximum interval (100 ms).
const DEFAULT_PCR_DISCONTINUITY_LIMIT_MS: u64 = 100;

/// TR 101 290 v1.4.1 Table 5.0b indicator 2.5 / note 3 — PTS maximum
/// repetition interval (700 ms; not applied to still pictures).
const DEFAULT_PTS_REPETITION_LIMIT_MS: u64 = 700;

// ── PCR / PES constants ─────────────────────────────────────────────────────

/// PCR modulus on the 27 MHz clock: `2^33 × 300` (33-bit base × 300 ticks).
/// ISO/IEC 13818-1 §2.4.3.5 — PCR wraps modulo this value.
const PCR_MODULUS_27MHZ: u64 = (1u64 << 33) * 300;

/// 27 MHz clock rate (ticks per second).
const CLOCK_27MHZ: u64 = 27_000_000;

/// PES start-code prefix byte 0 (ISO/IEC 13818-1 §2.4.3.7 Table 2-18).
const PES_PREFIX_0: u8 = 0x00;
/// PES start-code prefix byte 1.
const PES_PREFIX_1: u8 = 0x00;
/// PES start-code prefix byte 2.
const PES_PREFIX_2: u8 = 0x01;

/// Offset of the PES header `marker_bits + flags` byte relative to the PES
/// packet start (byte 6: `'10' + PES_scrambling_control + …`).
const PES_FLAGS_OFFSET: usize = 6;

/// Mask for the `PTS_DTS_flags` field within the PES header byte at offset 7
/// (bits `[7:6]` — `0b10` means PTS present, `0b11` means PTS+DTS).
const PES_PTS_DTS_FLAGS_MASK: u8 = 0b1100_0000;

/// Value indicating PTS is present in `PTS_DTS_flags` (bit 7 set).
const PES_PTS_PRESENT: u8 = 0b1000_0000;

/// CAT `table_id` value (ISO/IEC 13818-1 §2.4.4.5).
const CAT_TABLE_ID: u8 = dvb_si::table_id::TableId::Cat as u8;

// ── Public types ─────────────────────────────────────────────────────────────

/// Severity tier per TR 101 290 §5.2 (Tables 5.0a/5.0b/5.0c).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Priority {
    /// Table 5.0a — necessary for de-codability.
    First,
    /// Table 5.0b — recommended for continuous or periodic monitoring.
    Second,
    /// Table 5.0c — application-dependant monitoring.
    Third,
}

/// A TR 101 290 measurement indicator.
///
/// `#[non_exhaustive]` — SI-repetition variants may be added later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Indicator {
    // ── Priority 1 (Table 5.0a) ──────────────────────────────────────────
    /// TR 101 290 v1.4.1 Table 5.0a indicator 1.1 — loss of synchronisation
    /// with hysteresis.
    TsSyncLoss,
    /// TR 101 290 v1.4.1 Table 5.0a indicator 1.2 — sync_byte not equal 0x47.
    SyncByteError,
    /// TR 101 290 v1.4.1 Table 5.0a indicator 1.3.a — PAT_error_2.
    PatError2,
    /// TR 101 290 v1.4.1 Table 5.0a indicator 1.4 — Continuity_count_error.
    ContinuityCountError,
    /// TR 101 290 v1.4.1 Table 5.0a indicator 1.5.a — PMT_error_2.
    PmtError2,
    /// TR 101 290 v1.4.1 Table 5.0a indicator 1.6 — PID_error.
    PidError,

    // ── Priority 2 (Table 5.0b) ──────────────────────────────────────────
    /// TR 101 290 v1.4.1 Table 5.0b indicator 2.1 — Transport_error.
    TransportError,
    /// TR 101 290 v1.4.1 Table 5.0b indicator 2.2 — CRC_error.
    CrcError,
    /// TR 101 290 v1.4.1 Table 5.0b indicator 2.3a — PCR_repetition_error.
    PcrRepetitionError,
    /// TR 101 290 v1.4.1 Table 5.0b indicator 2.3b —
    /// PCR_discontinuity_indicator_error.
    PcrDiscontinuityError,
    /// TR 101 290 v1.4.1 Table 5.0b indicator 2.5 — PTS_error.
    PtsError,
    /// TR 101 290 v1.4.1 Table 5.0b indicator 2.6 — CAT_error.
    CatError,
}

impl Indicator {
    /// The priority tier this indicator belongs to.
    #[must_use]
    pub fn priority(self) -> Priority {
        match self {
            Self::TsSyncLoss
            | Self::SyncByteError
            | Self::PatError2
            | Self::ContinuityCountError
            | Self::PmtError2
            | Self::PidError => Priority::First,
            Self::TransportError
            | Self::CrcError
            | Self::PcrRepetitionError
            | Self::PcrDiscontinuityError
            | Self::PtsError
            | Self::CatError => Priority::Second,
        }
    }

    /// Verbatim indicator name from the TR 101 290 tables.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::TsSyncLoss => "TS_sync_loss",
            Self::SyncByteError => "Sync_byte_error",
            Self::PatError2 => "PAT_error_2",
            Self::ContinuityCountError => "Continuity_count_error",
            Self::PmtError2 => "PMT_error_2",
            Self::PidError => "PID_error",
            Self::TransportError => "Transport_error",
            Self::CrcError => "CRC_error",
            Self::PcrRepetitionError => "PCR_repetition_error",
            Self::PcrDiscontinuityError => "PCR_discontinuity_indicator_error",
            Self::PtsError => "PTS_error",
            Self::CatError => "CAT_error",
        }
    }

    /// Clause citation from the spec.
    #[must_use]
    pub fn clause(self) -> &'static str {
        match self {
            Self::TsSyncLoss => "TR 101 290 v1.4.1 Table 5.0a indicator 1.1",
            Self::SyncByteError => "TR 101 290 v1.4.1 Table 5.0a indicator 1.2",
            Self::PatError2 => "TR 101 290 v1.4.1 Table 5.0a indicator 1.3.a",
            Self::ContinuityCountError => "TR 101 290 v1.4.1 Table 5.0a indicator 1.4",
            Self::PmtError2 => "TR 101 290 v1.4.1 Table 5.0a indicator 1.5.a",
            Self::PidError => "TR 101 290 v1.4.1 Table 5.0a indicator 1.6",
            Self::TransportError => "TR 101 290 v1.4.1 Table 5.0b indicator 2.1",
            Self::CrcError => "TR 101 290 v1.4.1 Table 5.0b indicator 2.2",
            Self::PcrRepetitionError => "TR 101 290 v1.4.1 Table 5.0b indicator 2.3a",
            Self::PcrDiscontinuityError => "TR 101 290 v1.4.1 Table 5.0b indicator 2.3b",
            Self::PtsError => "TR 101 290 v1.4.1 Table 5.0b indicator 2.5",
            Self::CatError => "TR 101 290 v1.4.1 Table 5.0b indicator 2.6",
        }
    }
}

/// One raised conformance error.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ConformanceEvent {
    /// The indicator that was raised.
    pub indicator: Indicator,
    /// Priority tier of the indicator.
    pub priority: Priority,
    /// PID the error concerns, when applicable.
    pub pid: Option<u16>,
    /// Caller timestamp of the packet that raised it.
    pub at: Duration,
    /// Human-readable specifics (e.g. "expected cc=5, got 7").
    pub detail: String,
}

/// Diagnostic counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Stats {
    /// Total TS packets fed.
    pub packets: u64,
    /// Total conformance events raised.
    pub events: u64,
    /// Whether the monitor is currently in sync.
    pub in_sync: bool,
}

/// Configurable hysteresis and timeout parameters.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Config {
    /// Maximum interval between PAT sections (Table 5.0a 1.3.a / note 3).
    /// Default: 500 ms.
    pub pat_max_interval: Duration,
    /// Maximum interval between PMT sections per program_map_PID (1.5.a).
    /// Default: 500 ms.
    pub pmt_max_interval: Duration,
    /// Period after which a referenced PID is considered absent (1.6).
    /// Default: 5 s.
    pub pid_error_period: Duration,
    /// Consecutive good sync bytes to acquire sync (1.1).
    /// Default: 5.
    pub sync_acquire_packets: u8,
    /// Consecutive bad sync bytes to declare sync loss (1.1).
    /// Default: 2.
    pub sync_loss_packets: u8,
    /// Maximum interval between consecutive PCR values on a single PID
    /// (Table 5.0b 2.3a / note 2). Default: 100 ms.
    pub pcr_repetition_limit: Duration,
    /// Maximum legal PCR delta (in time) without a signalled discontinuity
    /// (Table 5.0b 2.3b). Default: 100 ms.
    pub pcr_discontinuity_limit: Duration,
    /// Maximum interval between consecutive PTS values on an elementary-stream
    /// PID (Table 5.0b 2.5 / note 3). Default: 700 ms.
    pub pts_repetition_limit: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            pat_max_interval: Duration::from_millis(DEFAULT_PAT_MAX_INTERVAL_MS),
            pmt_max_interval: Duration::from_millis(DEFAULT_PMT_MAX_INTERVAL_MS),
            pid_error_period: Duration::from_secs(DEFAULT_PID_ERROR_PERIOD_SECS),
            sync_acquire_packets: DEFAULT_SYNC_ACQUIRE_PACKETS,
            sync_loss_packets: DEFAULT_SYNC_LOSS_PACKETS,
            pcr_repetition_limit: Duration::from_millis(DEFAULT_PCR_REPETITION_LIMIT_MS),
            pcr_discontinuity_limit: Duration::from_millis(DEFAULT_PCR_DISCONTINUITY_LIMIT_MS),
            pts_repetition_limit: Duration::from_millis(DEFAULT_PTS_REPETITION_LIMIT_MS),
        }
    }
}

// ── Internal per-PID state ───────────────────────────────────────────────────

/// Per-PID continuity-counter tracking state.
struct CcState {
    last_cc: u8,
    had_payload: bool,
    dup_used: bool,
    initialised: bool,
}

/// Timer state for a presence/absence check (shared by 1.3.a, 1.5.a, 1.6).
struct PresenceTimer {
    last_seen: Duration,
    reported: bool,
}

/// State tracked for each program_map_PID signalled by the PAT.
struct PmtTracking {
    timer: PresenceTimer,
    reassembler: SectionReassembler,
}

/// State tracked for each elementary-stream PID referenced by a PMT.
struct EsTracking {
    timer: PresenceTimer,
}

/// Per-PID PCR tracking state (indicators 2.3a, 2.3b).
struct PcrState {
    last_pcr_27mhz: u64,
    last_pcr_time: Duration,
    initialised: bool,
}

/// Per-PID PTS tracking state (indicator 2.5).
struct PtsState {
    last_pts_time: Duration,
    armed: bool,
}

/// Per-PID section reassembly state for the well-known SI/PSI PIDs.
struct SiReassembly {
    reassembler: SectionReassembler,
}

// ── ConformanceMonitor ───────────────────────────────────────────────────────

/// ETSI TR 101 290 transport-stream conformance monitor.
///
/// Feed one TS packet at a time via [`feed`](Self::feed); each call returns
/// the events raised by that packet. The monitor is synchronous and
/// single-threaded — no interior mutability, no async.
pub struct ConformanceMonitor {
    config: Config,
    events: Vec<ConformanceEvent>,
    stats: Stats,

    // Sync hysteresis state machine (1.1)
    in_sync: bool,
    good_run: u8,
    bad_run: u8,

    // Per-PID continuity counter (1.4)
    cc_states: HashMap<u16, CcState>,

    // PAT section reassembly + timing (1.3.a)
    pat_reassembler: SectionReassembler,
    pat_timer: PresenceTimer,

    // PMT section reassembly + timing per program_map_PID (1.5.a)
    pmt_trackings: HashMap<u16, PmtTracking>,

    // Referenced ES PID timing (1.6)
    es_trackings: HashMap<u16, EsTracking>,

    // Well-known SI/PSI section reassembly + CRC checking (2.2)
    si_reassemblies: HashMap<u16, SiReassembly>,

    // Per-PID PCR tracking (2.3a, 2.3b)
    pcr_states: HashMap<u16, PcrState>,

    // Per-PID PTS tracking (2.5)
    pts_states: HashMap<u16, PtsState>,

    // CAT tracking (2.6)
    cat_seen: bool,
    scrambled_without_cat_reported: bool,
}

impl ConformanceMonitor {
    /// Create a monitor with default configuration.
    pub fn new() -> Self {
        Self::with_config(Config::default())
    }

    /// Create a monitor with the given configuration.
    pub fn with_config(config: Config) -> Self {
        let mut si_reassemblies = HashMap::new();
        for &pid in &SI_PIDS {
            si_reassemblies.insert(
                pid,
                SiReassembly {
                    reassembler: SectionReassembler::default(),
                },
            );
        }
        Self {
            config,
            events: Vec::new(),
            stats: Stats {
                packets: 0,
                events: 0,
                in_sync: false,
            },
            in_sync: false,
            good_run: 0,
            bad_run: 0,
            cc_states: HashMap::new(),
            pat_reassembler: SectionReassembler::default(),
            pat_timer: PresenceTimer {
                last_seen: Duration::ZERO,
                reported: false,
            },
            pmt_trackings: HashMap::new(),
            es_trackings: HashMap::new(),
            si_reassemblies,
            pcr_states: HashMap::new(),
            pts_states: HashMap::new(),
            cat_seen: false,
            scrambled_without_cat_reported: false,
        }
    }

    /// Feed ONE TS packet (any length; 188 expected) with its caller-supplied
    /// arrival time `t`.
    ///
    /// `t` must be monotonic non-decreasing across calls (documented but not
    /// enforced). Returns the events raised by this packet.
    pub fn feed(&mut self, ts_packet: &[u8], t: Duration) -> &[ConformanceEvent] {
        self.events.clear();
        self.stats.packets += 1;

        // ── Step 2: Sync byte check (1.2) ─────────────────────────────────
        let sync_ok = !ts_packet.is_empty() && ts_packet[0] == SYNC_BYTE;
        if !sync_ok {
            self.emit(Indicator::SyncByteError, None, t, "sync_byte != 0x47");
        }

        // ── Step 3: Sync hysteresis state machine (1.1) ──────────────────
        if sync_ok {
            self.good_run = self.good_run.saturating_add(1);
            self.bad_run = 0;
            if !self.in_sync && self.good_run >= self.config.sync_acquire_packets {
                self.in_sync = true;
                self.stats.in_sync = true;
            }
        } else {
            self.bad_run = self.bad_run.saturating_add(1);
            self.good_run = 0;
            if self.in_sync && self.bad_run >= self.config.sync_loss_packets {
                self.in_sync = false;
                self.stats.in_sync = false;
                self.emit(
                    Indicator::TsSyncLoss,
                    None,
                    t,
                    "sync lost after hysteresis threshold",
                );
            }
        }

        // Per the doc: "If indicator 1.1 is activated then all other
        // indicators are invalid." While not in sync, suppress all other
        // indicators.
        if !self.in_sync {
            return &self.events;
        }

        // ── Step 4: Parse TS packet ───────────────────────────────────────
        let packet = match TsPacket::parse(ts_packet) {
            Ok(p) => p,
            Err(_) => return &self.events,
        };
        let header = &packet.header;
        let pid = header.pid;

        // ── 2.1 Transport_error (Table 5.0b indicator 2.1) ──────────────
        if header.tei {
            self.emit(
                Indicator::TransportError,
                Some(pid),
                t,
                format!("transport_error_indicator set on PID 0x{:04X}", pid),
            );
        }

        // ── Step 5: Continuity_count_error (1.4) ─────────────────────────
        if pid != PID_NULL {
            self.check_cc(
                pid,
                header.continuity_counter,
                header.has_payload,
                t,
                ts_packet,
            );
        }

        // ── Step 7: PAT_error_2 — scrambling check (1.3.a) ──────────────
        if pid == PID_PAT && header.scrambling != 0 {
            self.emit(
                Indicator::PatError2,
                Some(PID_PAT),
                t,
                format!(
                    "scrambling_control_field != 00 on PID 0x0000 (got {})",
                    header.scrambling
                ),
            );
        }

        // ── Step 8: PMT_error_2 — scrambling check (1.5.a) ──────────────
        if self.pmt_trackings.contains_key(&pid) && header.scrambling != 0 {
            self.emit(
                Indicator::PmtError2,
                Some(pid),
                t,
                format!(
                    "scrambling_control_field != 00 on program_map_PID 0x{:04X}",
                    pid
                ),
            );
        }

        // ── 2.6 CAT_error — scrambled packet with no CAT (Table 5.0b 2.6)
        if header.scrambling != 0 && !self.cat_seen && !self.scrambled_without_cat_reported {
            self.scrambled_without_cat_reported = true;
            self.emit(
                Indicator::CatError,
                Some(pid),
                t,
                format!(
                    "scrambled packet on PID 0x{:04X} but no CAT seen on PID 0x0001",
                    pid
                ),
            );
        }

        // ── Step 6: Section reassembly — PAT ─────────────────────────────
        if pid == PID_PAT && header.has_payload {
            if let Some(payload) = packet.payload {
                self.pat_reassembler.feed(payload, header.pusi);
            }
            self.pat_timer.last_seen = t;
            self.pat_timer.reported = false;
            while let Some(section_bytes) = self.pat_reassembler.pop_section() {
                self.check_crc_and_process_pat(&section_bytes, pid, t);
            }
        }

        // ── Step 6b: Section reassembly — PMT PIDs ───────────────────────
        if self.pmt_trackings.contains_key(&pid) && header.has_payload {
            if let Some(payload) = packet.payload {
                if let Some(tracking) = self.pmt_trackings.get_mut(&pid) {
                    tracking.reassembler.feed(payload, header.pusi);
                }
            }
            let sections: Vec<_> = if let Some(tracking) = self.pmt_trackings.get_mut(&pid) {
                tracking.timer.last_seen = t;
                tracking.timer.reported = false;
                std::iter::from_fn(|| tracking.reassembler.pop_section()).collect()
            } else {
                Vec::new()
            };
            for section_bytes in &sections {
                self.check_crc_and_process_pmt(section_bytes, pid, t);
            }
        }

        // ── Step 6c: Section reassembly — well-known SI/PSI PIDs (2.2) ───
        // PAT and PMT PIDs are handled above (they have separate reassembly
        // for P1 logic). Only process the non-PAT, non-PMT SI PIDs here.
        if pid != PID_PAT
            && !self.pmt_trackings.contains_key(&pid)
            && self.si_reassemblies.contains_key(&pid)
            && header.has_payload
        {
            if let Some(payload) = packet.payload {
                if let Some(si_ra) = self.si_reassemblies.get_mut(&pid) {
                    si_ra.reassembler.feed(payload, header.pusi);
                }
            }
            let sections: Vec<_> = if let Some(si_ra) = self.si_reassemblies.get_mut(&pid) {
                std::iter::from_fn(|| si_ra.reassembler.pop_section()).collect()
            } else {
                Vec::new()
            };
            for section_bytes in &sections {
                self.check_crc_for_si(section_bytes, pid, t);
                self.check_cat_table_id(section_bytes, pid, t);
            }
        }
        // Also CRC-check completed PAT/PMT sections via the si_reassemblies
        // map (these share the same PID). PAT and PMT already have their own
        // reassemblers above — the si_reassemblies entries for those PIDs are
        // not fed again. CRC checking for PAT/PMT is done inside
        // check_crc_and_process_pat / check_crc_and_process_pmt.

        // ── Step 9: PID_error — update last_seen for referenced PIDs ─────
        if let Some(tracking) = self.es_trackings.get_mut(&pid) {
            tracking.timer.last_seen = t;
            tracking.timer.reported = false;
        }

        // ── 2.3a / 2.3b: PCR checks (Table 5.0b indicators 2.3a, 2.3b) ──
        if let Some(Ok(af)) = packet.adaptation_field() {
            if let Some(pcr) = af.pcr {
                self.check_pcr(pid, pcr.as_27mhz(), af.discontinuity_indicator, t);
            }
        }

        // ── 2.5: PTS check (Table 5.0b indicator 2.5) ───────────────────
        if header.pusi
            && header.scrambling == 0
            && self.es_trackings.contains_key(&pid)
            && header.has_payload
        {
            if let Some(payload) = packet.payload {
                self.check_pts(pid, payload, t);
            }
        }

        // ── Presence-timeout evaluation (1.3.a, 1.5.a, 1.6) ────────────
        self.check_presence_timeouts(t);

        &self.events
    }

    /// Diagnostic counters.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    fn emit(
        &mut self,
        indicator: Indicator,
        pid: Option<u16>,
        at: Duration,
        detail: impl Into<String>,
    ) {
        let event = ConformanceEvent {
            indicator,
            priority: indicator.priority(),
            pid,
            at,
            detail: detail.into(),
        };
        self.stats.events += 1;
        self.events.push(event);
    }

    /// Continuity_count_error (1.4) check.
    fn check_cc(&mut self, pid: u16, cc: u8, has_payload: bool, t: Duration, raw: &[u8]) {
        // Check for discontinuity_indicator in the adaptation field BEFORE
        // mutating cc_states (avoids holding the entry borrow across self.emit).
        let discontinuity = if raw.len() >= 5 {
            let b3 = raw[3];
            let has_adaptation = (b3 & 0x20) != 0;
            if has_adaptation {
                let af_len = raw[4] as usize;
                if af_len > 0 && raw.len() > 5 {
                    (raw[5] & 0x80) != 0
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        // Compute what we need from the existing state, then decide.
        let (expected, is_duplicate, should_emit_dup, should_emit_cc) = {
            let state = self.cc_states.entry(pid).or_insert_with(|| CcState {
                last_cc: cc,
                had_payload: has_payload,
                dup_used: false,
                initialised: false,
            });

            if !state.initialised {
                state.last_cc = cc;
                state.had_payload = has_payload;
                state.dup_used = false;
                state.initialised = true;
                return;
            }

            if discontinuity {
                // Will update state below — just signal no emit.
                (0u8, false, false, false)
            } else {
                let is_duplicate = cc == state.last_cc && has_payload;
                let mut should_emit_dup = false;
                let mut should_emit_cc = false;

                if is_duplicate {
                    if state.dup_used {
                        should_emit_dup = true;
                    }
                } else {
                    state.dup_used = false;
                    let expected = if has_payload {
                        (state.last_cc.wrapping_add(1)) & 0x0F
                    } else {
                        state.last_cc
                    };
                    if cc != expected {
                        should_emit_cc = true;
                    }
                }

                (
                    if has_payload {
                        (state.last_cc.wrapping_add(1)) & 0x0F
                    } else {
                        state.last_cc
                    },
                    is_duplicate,
                    should_emit_dup,
                    should_emit_cc,
                )
            }
        };

        // Now emit events without holding a borrow on cc_states.
        if should_emit_dup {
            self.emit(
                Indicator::ContinuityCountError,
                Some(pid),
                t,
                format!(
                    "second consecutive duplicate on PID 0x{:04X} (cc={})",
                    pid, cc
                ),
            );
        }
        if should_emit_cc {
            self.emit(
                Indicator::ContinuityCountError,
                Some(pid),
                t,
                format!("expected cc={}, got {} on PID 0x{:04X}", expected, cc, pid),
            );
        }

        // Finally, update state.
        let state = self.cc_states.get_mut(&pid).unwrap();
        if discontinuity {
            state.last_cc = cc;
            state.had_payload = has_payload;
            state.dup_used = false;
        } else if is_duplicate {
            // First duplicate is legal; mark dup_used but do NOT update last_cc.
            state.dup_used = true;
        } else {
            state.dup_used = false;
            state.last_cc = cc;
            state.had_payload = has_payload;
        }
    }

    /// CRC-check a completed section and, if on PID_PAT, process it.
    fn check_crc_and_process_pat(&mut self, section_bytes: &[u8], pid: u16, t: Duration) {
        // 2.2: CRC check on PAT section.
        self.check_crc_for_section(section_bytes, pid, t);

        self.process_pat_section(section_bytes, t);
    }

    /// CRC-check a completed section and, if on a PMT PID, process it.
    fn check_crc_and_process_pmt(&mut self, section_bytes: &[u8], pid: u16, t: Duration) {
        // 2.2: CRC check on PMT section.
        self.check_crc_for_section(section_bytes, pid, t);

        self.process_pmt_section(section_bytes, pid, t);
    }

    /// TR 101 290 v1.4.1 Table 5.0b indicator 2.2 — CRC_error.
    ///
    /// On any tracked PID, if a completed long-form section has a CRC
    /// mismatch, emit `CrcError`.
    fn check_crc_for_section(&mut self, section_bytes: &[u8], pid: u16, t: Duration) {
        let section = match Section::parse(section_bytes) {
            Ok(s) => s,
            Err(_) => return,
        };

        // validate_crc returns Ok for short-form sections (no CRC to check).
        if let Err(dvb_si::error::Error::CrcMismatch { .. }) = section.validate_crc(section_bytes) {
            self.emit(
                Indicator::CrcError,
                Some(pid),
                t,
                format!(
                    "CRC-32 mismatch on PID 0x{:04X} (table_id 0x{:02X})",
                    pid, section.table_id
                ),
            );
        }
    }

    /// CRC-check for SI PIDs that are not PAT/PMT (handled via si_reassemblies).
    fn check_crc_for_si(&mut self, section_bytes: &[u8], pid: u16, t: Duration) {
        self.check_crc_for_section(section_bytes, pid, t);
    }

    /// TR 101 290 v1.4.1 Table 5.0b indicator 2.6 — CAT_error, condition 1:
    /// section with `table_id != 0x01` on PID_CAT.
    fn check_cat_table_id(&mut self, section_bytes: &[u8], pid: u16, t: Duration) {
        if pid != PID_CAT {
            return;
        }
        let section = match Section::parse(section_bytes) {
            Ok(s) => s,
            Err(_) => return,
        };
        if section.table_id == CAT_TABLE_ID {
            // Valid CAT section — mark as seen.
            self.cat_seen = true;
            // Re-arm the "scrambled without CAT" check so that if a CAT was
            // previously absent and then appears, the check resets.
            self.scrambled_without_cat_reported = false;
        } else {
            self.emit(
                Indicator::CatError,
                Some(PID_CAT),
                t,
                format!(
                    "section with table_id 0x{:02X} on PID 0x0001 (expected 0x01 for CAT)",
                    section.table_id
                ),
            );
        }
    }

    /// Process a completed section on PID_PAT.
    fn process_pat_section(&mut self, section_bytes: &[u8], t: Duration) {
        let section = match Section::parse(section_bytes) {
            Ok(s) => s,
            Err(_) => return,
        };

        // 1.3.a: section with table_id other than 0x00 found on PID 0x0000.
        if section.table_id != PAT_TABLE_ID {
            self.emit(
                Indicator::PatError2,
                Some(PID_PAT),
                t,
                format!(
                    "section with table_id 0x{:02X} on PID 0x0000 (expected 0x00)",
                    section.table_id
                ),
            );
            return;
        }

        // Parse the PAT proper.
        let pat = match PatSection::parse(section_bytes) {
            Ok(p) => p,
            Err(_) => return,
        };

        // Discover program_map_PIDs and start tracking them.
        for entry in pat.programmes() {
            let pmt_pid = entry.pid;
            self.pmt_trackings
                .entry(pmt_pid)
                .or_insert_with(|| PmtTracking {
                    timer: PresenceTimer {
                        last_seen: t,
                        reported: false,
                    },
                    reassembler: SectionReassembler::default(),
                });
        }
    }

    /// Process a completed section on a program_map_PID.
    fn process_pmt_section(&mut self, section_bytes: &[u8], _pid: u16, t: Duration) {
        let section = match Section::parse(section_bytes) {
            Ok(s) => s,
            Err(_) => return,
        };

        // 1.5.a only checks presence and scrambling of table_id 0x02 sections.
        // If table_id is not 0x02, skip — we don't emit PMT_error_2 for a
        // wrong table_id on a program_map_PID (that's not in the spec for
        // 1.5.a).
        let pmt_table_id: u8 = dvb_si::tables::pmt::TABLE_ID;
        if section.table_id != pmt_table_id {
            return;
        }

        // Parse the PMT proper.
        let pmt = match PmtSection::parse(section_bytes) {
            Ok(p) => p,
            Err(_) => return,
        };

        // Collect new ES PIDs to add.
        let mut new_es_pids: Vec<u16> = Vec::new();
        if pmt.pcr_pid != PID_NULL && !self.es_trackings.contains_key(&pmt.pcr_pid) {
            new_es_pids.push(pmt.pcr_pid);
        }
        for stream in &pmt.streams {
            let es_pid = stream.elementary_pid;
            if !self.es_trackings.contains_key(&es_pid) {
                new_es_pids.push(es_pid);
            }
        }

        for es_pid in new_es_pids {
            self.es_trackings.insert(
                es_pid,
                EsTracking {
                    timer: PresenceTimer {
                        last_seen: t,
                        reported: false,
                    },
                },
            );
        }
    }

    /// TR 101 290 v1.4.1 Table 5.0b indicators 2.3a / 2.3b — PCR checks.
    fn check_pcr(&mut self, pid: u16, pcr_27mhz: u64, discontinuity: bool, t: Duration) {
        let state = self.pcr_states.entry(pid).or_insert_with(|| PcrState {
            last_pcr_27mhz: 0,
            last_pcr_time: Duration::ZERO,
            initialised: false,
        });

        if !state.initialised {
            state.last_pcr_27mhz = pcr_27mhz;
            state.last_pcr_time = t;
            state.initialised = true;
            return;
        }

        // Snapshot state for decision-making before any emit.
        let last_pcr_time = state.last_pcr_time;
        let last_pcr_27mhz = state.last_pcr_27mhz;

        // 2.3a: PCR_repetition_error — interval between consecutive PCR
        // values exceeds the configured limit.
        let rep_interval = t.saturating_sub(last_pcr_time);
        let should_emit_rep = rep_interval > self.config.pcr_repetition_limit;

        // 2.3b: PCR_discontinuity_indicator_error — PCR delta exceeds 100 ms
        // without a signalled discontinuity.
        let delta =
            (pcr_27mhz.wrapping_add(PCR_MODULUS_27MHZ) - last_pcr_27mhz) % PCR_MODULUS_27MHZ;
        let delta_ms = delta * 1000 / CLOCK_27MHZ;
        let limit_ms = self.config.pcr_discontinuity_limit.as_millis() as u64;
        let should_emit_disc = delta_ms > limit_ms && !discontinuity;

        // Emit outside the HashMap borrow.
        if should_emit_rep {
            self.emit(
                Indicator::PcrRepetitionError,
                Some(pid),
                t,
                format!(
                    "PCR interval {} ms exceeds limit {} ms on PID 0x{:04X}",
                    rep_interval.as_millis(),
                    self.config.pcr_repetition_limit.as_millis(),
                    pid
                ),
            );
        }
        if should_emit_disc {
            self.emit(
                Indicator::PcrDiscontinuityError,
                Some(pid),
                t,
                format!(
                    "PCR delta {} ms exceeds limit {} ms on PID 0x{:04X} without discontinuity_indicator",
                    delta_ms, limit_ms, pid
                ),
            );
        }

        // Update state.
        let state = self.pcr_states.get_mut(&pid).unwrap();
        state.last_pcr_27mhz = pcr_27mhz;
        state.last_pcr_time = t;
    }

    /// TR 101 290 v1.4.1 Table 5.0b indicator 2.5 — PTS_error.
    ///
    /// Peeks the PES header on an elementary-stream PID for PTS_DTS_flags.
    /// Only checks PIDs that have been "armed" by seeing at least one PTS.
    fn check_pts(&mut self, pid: u16, payload: &[u8], t: Duration) {
        // PES start-code prefix: 00 00 01.
        if payload.len() < PES_FLAGS_OFFSET + 2 {
            return;
        }
        if payload[0] != PES_PREFIX_0 || payload[1] != PES_PREFIX_1 || payload[2] != PES_PREFIX_2 {
            return;
        }

        // Byte 6: `'10' + flags` — the top two bits must be `10`.
        let flags_byte = payload[PES_FLAGS_OFFSET];
        if (flags_byte >> 6) != 0b10 {
            return;
        }

        // Byte 7: PTS_DTS_flags in bits `[7:6]`.
        let pts_dts_flags = payload[PES_FLAGS_OFFSET + 1] & PES_PTS_DTS_FLAGS_MASK;
        let pts_present = (pts_dts_flags & PES_PTS_PRESENT) != 0;
        if !pts_present {
            return;
        }

        let state = self.pts_states.entry(pid).or_insert_with(|| PtsState {
            last_pts_time: Duration::ZERO,
            armed: false,
        });

        if !state.armed {
            // First PTS on this PID — arm the check, no error yet.
            state.last_pts_time = t;
            state.armed = true;
            return;
        }

        // Snapshot state for decision-making before any emit.
        let last_pts_time = state.last_pts_time;
        let pts_interval = t.saturating_sub(last_pts_time);
        let should_emit = pts_interval > self.config.pts_repetition_limit;

        if should_emit {
            self.emit(
                Indicator::PtsError,
                Some(pid),
                t,
                format!(
                    "PTS interval {} ms exceeds limit {} ms on PID 0x{:04X}",
                    pts_interval.as_millis(),
                    self.config.pts_repetition_limit.as_millis(),
                    pid
                ),
            );
        }

        // Update state.
        let state = self.pts_states.get_mut(&pid).unwrap();
        state.last_pts_time = t;
    }

    /// Evaluate all presence/absence timeouts against the current time `t`.
    fn check_presence_timeouts(&mut self, t: Duration) {
        // 1.3.a: PAT presence timeout
        if t.saturating_sub(self.pat_timer.last_seen) > self.config.pat_max_interval
            && !self.pat_timer.reported
        {
            self.pat_timer.reported = true;
            self.emit(
                Indicator::PatError2,
                Some(PID_PAT),
                t,
                format!(
                    "no PAT section within {} ms",
                    self.config.pat_max_interval.as_millis()
                ),
            );
        }

        // 1.5.a: PMT presence timeout per program_map_PID
        // Collect PIDs that need events, then emit outside the iteration.
        let pmt_timeouts: Vec<(u16, u64)> = self
            .pmt_trackings
            .iter()
            .filter_map(|(&pid, tracking)| {
                if t.saturating_sub(tracking.timer.last_seen) > self.config.pmt_max_interval
                    && !tracking.timer.reported
                {
                    Some((pid, self.config.pmt_max_interval.as_millis() as u64))
                } else {
                    None
                }
            })
            .collect();
        for (pid, interval_ms) in pmt_timeouts {
            if let Some(tracking) = self.pmt_trackings.get_mut(&pid) {
                tracking.timer.reported = true;
            }
            self.emit(
                Indicator::PmtError2,
                Some(pid),
                t,
                format!(
                    "no PMT section on program_map_PID 0x{:04X} within {} ms",
                    pid, interval_ms
                ),
            );
        }

        // 1.6: PID_error — referenced PID absence
        let pid_timeouts: Vec<(u16, u64)> = self
            .es_trackings
            .iter()
            .filter_map(|(&pid, tracking)| {
                if t.saturating_sub(tracking.timer.last_seen) > self.config.pid_error_period
                    && !tracking.timer.reported
                {
                    Some((pid, self.config.pid_error_period.as_secs()))
                } else {
                    None
                }
            })
            .collect();
        for (pid, period_secs) in pid_timeouts {
            if let Some(tracking) = self.es_trackings.get_mut(&pid) {
                tracking.timer.reported = true;
            }
            self.emit(
                Indicator::PidError,
                Some(pid),
                t,
                format!(
                    "referenced PID 0x{:04X} absent for > {} s",
                    pid, period_secs
                ),
            );
        }
    }
}

impl Default for ConformanceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
