//! ETSI TR 101 290 v1.4.1 transport-stream conformance monitor.
//!
//! Implements the **first-priority** indicator set (Table 5.0a, indicators
//! 1.1–1.6). Priority 2 and SI-repetition indicators will be added in a future
//! release.
//!
//! # Caller-supplied time
//!
//! [`ConformanceMonitor::feed`] takes a [`core::time::Duration`] timestamp
//! alongside each TS packet. All presence/absence timeout checks (1.3.a, 1.5.a,
//! 1.6) are evaluated against this clock. The caller must ensure that
//! timestamps are **monotonic non-decreasing** across calls; the monitor does
//! not enforce this but non-monotonic timestamps will produce spurious events.
//!
//! # References
//!
//! - ETSI TR 101 290 v1.4.1 (2023-05), §5.2.1, Table 5.0a
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
/// PID 0x1FFF — Null/padding packets (ISO/IEC 13818-1 §2.4.3.3).
const PID_NULL: u16 = 0x1FFF;

/// Sync byte value (ISO/IEC 13818-1 §2.4.3.3).
const SYNC_BYTE: u8 = 0x47;

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
/// `#[non_exhaustive]` — Priority 2 and SI-repetition variants are added later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Indicator {
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            pat_max_interval: Duration::from_millis(DEFAULT_PAT_MAX_INTERVAL_MS),
            pmt_max_interval: Duration::from_millis(DEFAULT_PMT_MAX_INTERVAL_MS),
            pid_error_period: Duration::from_secs(DEFAULT_PID_ERROR_PERIOD_SECS),
            sync_acquire_packets: DEFAULT_SYNC_ACQUIRE_PACKETS,
            sync_loss_packets: DEFAULT_SYNC_LOSS_PACKETS,
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

    // PAT section reassembly (1.3.a)
    pat_reassembler: SectionReassembler,
    pat_timer: PresenceTimer,

    // PMT section reassembly + timing per program_map_PID (1.5.a)
    pmt_trackings: HashMap<u16, PmtTracking>,

    // Referenced ES PID timing (1.6)
    es_trackings: HashMap<u16, EsTracking>,
}

impl ConformanceMonitor {
    /// Create a monitor with default configuration.
    pub fn new() -> Self {
        Self::with_config(Config::default())
    }

    /// Create a monitor with the given configuration.
    pub fn with_config(config: Config) -> Self {
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

        // ── Step 6: Section reassembly ───────────────────────────────────
        if pid == PID_PAT && header.has_payload {
            if let Some(payload) = packet.payload {
                self.pat_reassembler.feed(payload, header.pusi);
            }
            self.pat_timer.last_seen = t;
            self.pat_timer.reported = false;
            while let Some(section_bytes) = self.pat_reassembler.pop_section() {
                self.process_pat_section(&section_bytes, t);
            }
        }

        // PMT section reassembly
        if self.pmt_trackings.contains_key(&pid) && header.has_payload {
            if let Some(payload) = packet.payload {
                if let Some(tracking) = self.pmt_trackings.get_mut(&pid) {
                    tracking.reassembler.feed(payload, header.pusi);
                }
            }
            // Collect sections first, then process — avoids double &mut self.
            let sections: Vec<_> = if let Some(tracking) = self.pmt_trackings.get_mut(&pid) {
                tracking.timer.last_seen = t;
                tracking.timer.reported = false;
                std::iter::from_fn(|| tracking.reassembler.pop_section()).collect()
            } else {
                Vec::new()
            };
            for section_bytes in &sections {
                self.process_pmt_section(section_bytes, pid, t);
            }
        }

        // ── Step 9: PID_error — update last_seen for referenced PIDs ─────
        if let Some(tracking) = self.es_trackings.get_mut(&pid) {
            tracking.timer.last_seen = t;
            tracking.timer.reported = false;
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
