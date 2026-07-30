//! ISO/IEC 13818-1 (H.222.0) Transport-System Target Decoder (T-STD) buffer model.
//!
//! Implements the buffer-simulation engine that backs conformance indicators
//! 3.3 `Buffer_error`, 3.9 `Empty_buffer_error`, and 3.10 `Data_delay_error`
//! (ETSI TR 101 290 v1.4.1 Table 5.0c). Every buffer size, leak rate, and
//! threshold is a **named constant** citing its H.222.0 clause — transcribed
//! from the vendored PDF at `private/specs/itu_t_h222_0_202308_mpeg2_systems.pdf`.
//! The transcription lives at [`docs/h222_0-tstd-buffer-model.md`].
//!
//! # Architecture
//!
//! The monitor is **sans-IO** with a caller-supplied clock and no independent
//! hardware arrival timing. This model simulates the T-STD buffers using the
//! packet arrival time `t` provided in each [`crate::ConformanceMonitor::feed`]
//! call as the input clock. The caller is responsible for supplying timestamps
//! that are consistent with the transport rate; the model does **not** verify
//! that the inter-packet spacing matches any particular bitrate.
//!
//! Because arrival times are caller-supplied (not hardware), indicator 2.4
//! `PCR_accuracy_error` (±500 ns, H.222.0 §2.4.2.2) remains unimplemented — a
//! packet-index-derived arrival estimate cannot honestly resolve 500 ns.
//!
//! # References
//!
//! - ITU-T H.222.0 v9 (08/2023) | ISO/IEC 13818-1:2023 §2.4.2.1–§2.4.2.4 —
//!   T-STD buffer model (symbols, sizes, leak rates)
//! - ITU-T H.222.0 §2.4.2.6 — T-STD for audio
//! - ISO/IEC 13818-4 §9.11.2 — TBsys_buffering_error
//! - ISO/IEC 13818-4 §9.1.4 — Buffer_error checks
//! - ISO/IEC 13818-9 Annex E — Extension for Real Time Interface
//! - ETSI TR 101 290 v1.4.1 Table 5.0c — indicators 3.3, 3.9, 3.10
//!
//! # Buffer-size grounding
//!
//! All buffer sizes and rates are grounded in H.222.0 §2.4.2.4 ("Buffering"),
//! PDF pages 42–43 of the 2023-08 edition. See
//! [`docs/h222_0-tstd-buffer-model.md`] for the full transcription.
//!
//! | Constant | Value | H.222.0 clause |
//! |----------|-------|----------------|
//! | TB_SIZE | 512 bytes | §2.4.2.4, p.42 l.20: "The transport buffer size is fixed at 512 bytes." |
//! | TB_SYS_SIZE | 512 bytes | §2.4.2.4 (same line; TBSsys defined at §2.4.2.1 p.39 l.23) |
//! | TB_SYS_LEAK_RATE | 1 000 000 bps | §2.4.2.4, p.42 l.10: "For systems data: Rxn = 1×10⁶ bits per second." Rxsys (§2.4.2.1 p.39 l.35) is TBsys's drain rate; the spec states no separate numeric value, so it is the systems-data Rxn. |
//! | TB_LEAK_RATE_FLOOR | 15 625 bytes/s | Not from the spec — a conservative floor below any real DVB stream's coded rate, used only during bitrate convergence |
//! | DATA_DELAY_LIMIT_SECS | 1 s | TR 101 290 v1.4.1 Table 5.0c indicator 3.10 |
//! | STILL_PICTURE_DELAY_LIMIT_SECS | 60 s | TR 101 290 v1.4.1 Table 5.0c indicator 3.10 |
//! | TB_EMPTY_INTERVAL_SECS | 1 s | TR 101 290 v1.4.1 Table 5.0c indicator 3.9 |
//! | TB_SYS_EMPTY_INTERVAL_SECS | 1 s | TR 101 290 v1.4.1 Table 5.0c indicator 3.9 |
//!
//! # Known limitations
//!
//! - **MBn/EBn/Bn/Bsys not modelled**: these require codec-level buffer sizes
//!   from descriptors (multiplex_buffer_descriptor, smoothing_buffer_descriptor,
//!   etc.) and the full leak/vbv_delay methods. The current implementation
//!   models only the front-end transport buffers (TBn, TBsys), which are common
//!   to all stream types.
//! - **Bsys** (1536 bytes, H.222.0 §2.4.2.4 p.43 l.37) is deferred — the PSI
//!   input buffer is downstream of TBsys and not currently modelled.
//! - **No arrival-time validation**: the model trusts the caller's timestamps.
//!   If the caller supplies timestamps inconsistent with the transport rate,
//!   buffer behaviour will not reflect real hardware.

use core::time::Duration;

// ── T-STD buffer constants (ISO/IEC 13818-1 §2.4.2.3 / §2.4.2.6) ──────────

/// Size of a single elementary-stream transport buffer TBn.
///
/// H.222.0 §2.4.2.4, PDF page 42 line 20:
/// "The transport buffer size is fixed at **512 bytes**."
/// Defined at §2.4.2.1 (PDF p.39 l.24–25) as TBSn.
pub(crate) const TB_SIZE: u64 = 512;

/// Size of the system-information transport buffer TBsys.
///
/// H.222.0 §2.4.2.4 (same "fixed at 512 bytes" line as TBn).
/// Defined at §2.4.2.1 (PDF p.39 l.22–23) as TBSsys.
pub(crate) const TB_SYS_SIZE: u64 = 512;

/// Leak rate from TBsys (bytes per second).
///
/// H.222.0 §2.4.2.4, PDF page 42 line 10:
/// "For systems data: Rxn = **1×10⁶ bits per second**."
/// Rxsys (§2.4.2.1, PDF p.39 l.35) is the rate data leave TBsys;
/// the spec gives no separate numeric value for Rxsys, so it is the
/// systems-data Rxn. 1 000 000 bits/s ÷ 8 = 125 000 bytes/s.
pub(crate) const TB_SYS_LEAK_RATE: u64 = 1_000_000 / 8;

/// Floor leak rate for per-PID transport buffers TBn (bytes per second).
///
/// When the effective bitrate cannot be estimated (e.g. first packets of a
/// new PID), the TBn drain rate floors at 125 kbit/s = 15 625 bytes/s.
/// This is a **conservative modelling floor**, not a spec value — the real
/// Rxn for any video/audio PID is at least 2×10⁶ bps (§2.4.2.4).
///
/// For a 512-byte buffer draining at 15 625 bytes/s, the buffer empties
/// in ~32.8 ms — well within the 1 s empty-buffer check interval.
pub(crate) const TB_LEAK_RATE_FLOOR: u64 = 15_625;

/// Maximum data delay through the T-STD buffers for non-still-picture data.
///
/// ETSI TR 101 290 v1.4.1 Table 5.0c indicator 3.10: "Delay of data (except
/// still picture video data) through the T-STD buffers superior to 1 second".
pub(crate) const DATA_DELAY_LIMIT_SECS: u64 = 1;

/// Maximum data delay through the T-STD buffers for still-picture video data.
///
/// ETSI TR 101 290 v1.4.1 Table 5.0c indicator 3.10: "delay of still picture
/// video data through the T-STD buffers superior to 60 s".
#[allow(dead_code)]
pub(crate) const STILL_PICTURE_DELAY_LIMIT_SECS: u64 = 60;

/// Interval over which TBn must empty at least once.
///
/// ETSI TR 101 290 v1.4.1 Table 5.0c indicator 3.9: "Transport buffer (TBn)
/// not empty at least once per second".
pub(crate) const TB_EMPTY_INTERVAL_SECS: u64 = 1;

/// Interval over which TBsys must empty at least once.
///
/// ETSI TR 101 290 v1.4.1 Table 5.0c indicator 3.9: "transport buffer for
/// system information (TBsys) not empty at least once per second".
pub(crate) const TB_SYS_EMPTY_INTERVAL_SECS: u64 = 1;

// ── Buffer model state types ─────────────────────────────────────────────────

/// A single T-STD buffer with fixed size, tracking occupancy and drain timing.
///
/// Models the ISO/IEC 13818-1 §2.4.2.3 transport buffer: data arrives
/// instantaneously at the packet's arrival time and drains at a constant
/// leak rate. Overflow is detected when the sum of incoming data exceeds
/// the buffer capacity.
#[derive(Debug, Clone)]
pub(crate) struct StdBuffer {
    /// Current occupancy in bytes.
    occupancy: u64,
    /// Buffer capacity in bytes.
    capacity: u64,
    /// Drain rate in bytes per second.
    leak_rate: u64,
    /// Time of the last drain update.
    last_drain: Duration,
    /// Time the first byte of any data still in this buffer arrived.
    /// `None` when the buffer is empty.
    first_byte_arrival: Option<Duration>,
    /// Whether this buffer has been empty at least once since the last
    /// empty-interval check (indicator 3.9).
    #[allow(dead_code)]
    empty_since_check: bool,
    /// When the last empty-interval check was performed (indicator 3.9).
    #[allow(dead_code)]
    last_empty_check: Duration,
}

impl StdBuffer {
    /// Create a new buffer with the given capacity and leak rate in
    /// **bytes per second**.
    pub(crate) fn new(capacity: u64, leak_rate: u64, now: Duration) -> Self {
        Self {
            occupancy: 0,
            capacity,
            leak_rate,
            last_drain: now,
            first_byte_arrival: None,
            empty_since_check: false,
            last_empty_check: now,
        }
    }

    /// Drain the buffer up to the current time `now`.
    ///
    /// Returns the number of bytes drained (for data-delay tracking).
    pub(crate) fn drain_to(&mut self, now: Duration) {
        let elapsed_us = now.saturating_sub(self.last_drain).as_micros() as u64;
        if elapsed_us == 0 {
            return;
        }
        // drain_bytes = elapsed_us * leak_rate / 1_000_000
        let drain_bytes = elapsed_us * self.leak_rate / 1_000_000;
        if drain_bytes >= self.occupancy {
            self.occupancy = 0;
            self.first_byte_arrival = None;
        } else {
            self.occupancy -= drain_bytes;
        }
        self.last_drain = now;
    }

    /// Set the leak rate to use for subsequent drains (bytes per second).
    pub(crate) fn set_leak_rate(&mut self, rate: u64) {
        self.leak_rate = rate;
    }

    /// Feed `n` bytes into the buffer at time `now`.
    ///
    /// Drains first, then adds. Returns `true` on overflow.
    /// Updates `first_byte_arrival` for data-delay tracking.
    pub(crate) fn feed(&mut self, bytes: u64, now: Duration) -> bool {
        self.drain_to(now);

        if self.occupancy == 0 {
            self.first_byte_arrival = Some(now);
            self.empty_since_check = true;
        }

        self.occupancy += bytes;
        self.occupancy > self.capacity
    }

    /// Check whether the buffer overflows at time `now` given `n` incoming
    /// bytes (without actually feeding them — a peek).
    #[allow(dead_code)]
    pub(crate) fn would_overflow(&self, bytes: u64, now: Duration) -> bool {
        let elapsed_us = now.saturating_sub(self.last_drain).as_micros() as u64;
        let drain_bytes = elapsed_us * self.leak_rate / 1_000_000;
        let projected = self.occupancy.saturating_sub(drain_bytes);
        projected + bytes > self.capacity
    }

    /// The time at which the oldest byte currently in this buffer arrived.
    /// `None` if the buffer is empty.
    #[allow(dead_code)]
    pub(crate) fn first_byte_arrival(&self) -> Option<Duration> {
        self.first_byte_arrival
    }

    /// Maximum data delay (in seconds) for bytes currently in this buffer.
    pub(crate) fn delay_secs(&self, now: Duration) -> Option<f64> {
        self.first_byte_arrival
            .map(|t| now.saturating_sub(t).as_secs_f64())
    }

    /// Current occupancy in bytes.
    #[allow(dead_code)]
    pub(crate) fn occupancy(&self) -> u64 {
        self.occupancy
    }

    /// Check indicator 3.9: whether this buffer has been empty at least once
    /// in the last `EMPTY_INTERVAL_SECS` seconds. Returns `true` if the check
    /// fails (i.e. the buffer has NOT been empty).
    ///
    /// Resets the interval window on each call.
    pub(crate) fn check_empty_interval(&mut self, now: Duration) -> bool {
        let elapsed = now.saturating_sub(self.last_empty_check).as_secs();
        if elapsed < TB_EMPTY_INTERVAL_SECS {
            // Not yet time for the next check.
            return false;
        }

        let was_empty = core::mem::take(&mut self.empty_since_check);
        self.last_empty_check = now;
        !was_empty
    }
}

/// Per-PID T-STD state: one transport buffer TBn + data-delay tracking.
#[derive(Debug, Clone)]
pub(crate) struct PidStdState {
    /// The transport buffer TBn for this PID (512 bytes capacity).
    pub(crate) tb: StdBuffer,
    /// Time this PID first became tracked.
    #[allow(dead_code)]
    pub(crate) first_seen: Duration,
    /// Accumulated bytes since first_seen (for estimating effective bitrate).
    pub(crate) total_bytes: u64,
    /// Whether an Empty_buffer_error has been reported for this PID in the
    /// current empty-interval window.
    pub(crate) empty_reported: bool,
    /// Whether a Data_delay_error has been reported for this PID.
    pub(crate) delay_reported: bool,
    /// The caller's timestamp at the last packet fed to this PID.
    pub(crate) last_packet_time: Duration,
}

impl PidStdState {
    pub(crate) fn new(tb_leak_rate: u64, now: Duration) -> Self {
        Self {
            tb: StdBuffer::new(TB_SIZE, tb_leak_rate, now),
            first_seen: now,
            total_bytes: 0,
            empty_reported: false,
            delay_reported: false,
            last_packet_time: now,
        }
    }
}

/// The monitors' T-STD buffer model state.
#[derive(Debug)]
pub(crate) struct TstdModel {
    /// System-information transport buffer TBsys (512 bytes, 1 Mbit/s leak).
    pub(crate) tb_sys: StdBuffer,
    /// Per-PID transport buffers TBn for each tracked PID.
    pub(crate) pid_buffers: alloc::collections::BTreeMap<u16, PidStdState>,
    /// TBsys empty-interval check (indicator 3.9).
    pub(crate) tb_sys_empty_reported: bool,
}

impl TstdModel {
    pub(crate) fn new(now: Duration) -> Self {
        Self {
            tb_sys: StdBuffer::new(TB_SYS_SIZE, TB_SYS_LEAK_RATE, now),
            pid_buffers: alloc::collections::BTreeMap::new(),
            tb_sys_empty_reported: false,
        }
    }

    /// Drain the TBsys buffer to the current time. Called once per packet.
    pub(crate) fn drain_tb_sys(&mut self, now: Duration) {
        self.tb_sys.drain_to(now);
    }
}
