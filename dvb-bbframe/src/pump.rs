//! [`BbframePump`] — per-PLP BBFrame→inner-TS pump.
//!
//! BBFrame user-packet framing per EN 302 755 §5.1.7 (BBHEADER) / §5.1.8
//! (user-packet carriage, SYNCD); composes [`crate::header::Bbheader`] and the
//! [`crate::packet::CarryOverExtractor`].
//!
//! Packages the BBFrame→inner-TS extraction chain that consumers otherwise
//! hand-wire: parse the BBHEADER, detect the mode, run carry-over extraction
//! keyed by PLP id, and return the completed 188-byte TS packets per frame.
//!
//! Zero dependencies on `dvb-t2mi` — the pump takes already-unwrapped BBFrame
//! data-field bytes (`df_bytes` = BBHEADER + data field, as
//! `dvb_t2mi::AnyPayload::Bbframe`'s `bbframe` field yields).

use crate::header::{Bbheader, Mode, TsGs, BBHEADER_LEN};
use crate::packet::{CarryOverExtractor, CarryOverStats, NM_UP_SIZE};

const MAX_PLPS: usize = 256;

/// Per-feed diagnostic counters for a [`BbframePump`].
///
/// The pump stays resilient (it never errors or panics on malformed input —
/// bad frames are skipped so a stream keeps flowing). These counters make the
/// otherwise-silent skips observable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct BbframePumpStats {
    /// Frames whose BBHEADER could not be parsed or was too short.
    pub header_parse_failures: u64,
    /// Frames with a non-TS MATYPE (GSE, GFPS, GCS) — the pump only extracts
    /// TS user packets.
    pub non_ts_payloads: u64,
    /// Aggregated carry-over stats from all PLPs.
    pub carry_over: CarryOverStats,
}

/// Per-PLP BBFrame→inner-TS pump.
///
/// Feed one PLP's BBFrame data-field bytes at a time with [`feed`](Self::feed);
/// the pump parses the BBHEADER, detects NM vs HEM, and runs the per-PLP
/// [`CarryOverExtractor`] — interleaved PLPs keep independent carry-over state.
///
/// The returned slice borrows an internal buffer that is cleared on every call;
/// copy out anything you need to keep.
///
/// ```no_run
/// # fn main() {
/// use dvb_bbframe::pump::BbframePump;
///
/// let mut pump = BbframePump::new();
/// // Receive a BBFrame data-field byte slice (BBHEADER + data field)
/// // for PLP 5:
/// let df_bytes = vec![0u8; 200];
/// let inner_ts_packets = pump.feed(5, &df_bytes);
/// for pkt in inner_ts_packets {
///     println!("inner TS packet recovered");
/// }
/// # }
/// ```
pub struct BbframePump {
    /// Per-PLP extractors, indexed by `plp_id`.  A `None` entry means that PLP
    /// has not been seen yet (lazily created on first `feed`).
    extractors: [Option<CarryOverExtractor>; MAX_PLPS],
    /// Output buffer — cleared per `feed` call.
    out: Vec<[u8; NM_UP_SIZE]>,
    /// Per-PLP temporary buffer reused across frames.
    up_buf: Vec<[u8; NM_UP_SIZE]>,
    /// Diagnostic counters.
    stats: BbframePumpStats,
}

impl BbframePump {
    /// Create a fresh pump with no PLP state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            extractors: std::array::from_fn(|_| None),
            out: Vec::new(),
            up_buf: Vec::new(),
            stats: BbframePumpStats::default(),
        }
    }

    /// Feed one PLP's BBFrame data-field bytes (`df_bytes` = 10-byte BBHEADER
    /// + data field).
    ///
    /// Returns the inner 188-byte TS packets completed by this frame.
    ///
    /// Per-PLP carry-over state is keyed by `plp_id` so interleaved PLPs don't
    /// corrupt each other's carry-over.  A new `plp_id` lazily creates a
    /// fresh extractor.
    ///
    /// This method is **infallible**: malformed input, bad BBHEADERs, and
    /// non-TS payloads emit no packets and bump a stat counter — never panics,
    /// never errors out.
    pub fn feed(&mut self, plp_id: u8, df_bytes: &[u8]) -> &[[u8; NM_UP_SIZE]] {
        self.out.clear();

        // ── Validate BBHEADER length ──────────────────────────────────────
        if df_bytes.len() < BBHEADER_LEN {
            self.stats.header_parse_failures += 1;
            return &self.out;
        }

        // ── Parse BBHEADER ────────────────────────────────────────────────
        let hdr = match Bbheader::parse(df_bytes) {
            Ok(h) => h,
            Err(_) => {
                self.stats.header_parse_failures += 1;
                return &self.out;
            }
        };

        // ── Non-TS payload → skip ─────────────────────────────────────────
        if hdr.matype.ts_gs != TsGs::Ts {
            self.stats.non_ts_payloads += 1;
            return &self.out;
        }

        let header_bytes: [u8; BBHEADER_LEN] = match df_bytes[..BBHEADER_LEN].try_into() {
            Ok(b) => b,
            Err(_) => {
                self.stats.header_parse_failures += 1;
                return &self.out;
            }
        };
        let data_field = &df_bytes[BBHEADER_LEN..];

        // ── Get or create per-PLP extractor ───────────────────────────────
        let idx = plp_id as usize;
        let extractor = self.extractors[idx].get_or_insert_with(CarryOverExtractor::new);

        // ── Dispatch by mode ──────────────────────────────────────────────
        match hdr.mode {
            Mode::Normal => {
                extractor.feed_nm_into(&header_bytes, data_field, &mut self.up_buf);
            }
            Mode::HighEfficiency => {
                // HEM with NPD is handled by CarryOverExtractor (npd_unsupported
                // stat bump); we pass npd through and the extractor skips.
                extractor.feed_hem_into(
                    &header_bytes,
                    data_field,
                    hdr.matype.npd,
                    &mut self.up_buf,
                );
            }
        }

        self.out.append(&mut self.up_buf);
        &self.out
    }

    /// Diagnostic counters accumulated across all `feed` calls.
    ///
    /// The per-PLP `carry_over` field aggregates the stats from all
    /// PLP extractors; check `carry_over.npd_unsupported` in particular —
    /// a non-zero value means valid HEM frames were dropped (NPD reinsertion
    /// unsupported).
    #[must_use]
    pub fn stats(&self) -> BbframePumpStats {
        let mut carry_over = CarryOverStats::default();
        for ext in self.extractors.iter().flatten() {
            let s = ext.stats();
            carry_over.npd_unsupported += s.npd_unsupported;
            carry_over.header_parse_failures += s.header_parse_failures;
            carry_over.mode_mismatches += s.mode_mismatches;
            carry_over.partial_discards += s.partial_discards;
        }
        BbframePumpStats {
            header_parse_failures: self.stats.header_parse_failures,
            non_ts_payloads: self.stats.non_ts_payloads,
            carry_over,
        }
    }
}

impl Default for BbframePump {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crc::crc8;
    use crate::header::{Matype, TsGs};

    const TS_SYNC: u8 = 0x47;
    const TS_LEN: usize = NM_UP_SIZE;

    /// One inner TS packet: PID 0x0100, PUSI, all-0xAA payload (distinguishable).
    fn inner_packet() -> [u8; TS_LEN] {
        let mut p = [0xAAu8; TS_LEN];
        p[0] = TS_SYNC;
        p[1] = 0x41; // PUSI | PID hi = 0x0100
        p[2] = 0x00;
        p[3] = 0x10; // payload only
        p
    }

    /// Build a Normal-Mode BBFrame (BBHEADER + data field) containing one TS packet.
    fn nm_bbframe(inner: &[u8; TS_LEN]) -> Vec<u8> {
        let hdr = Bbheader {
            matype: Matype {
                ts_gs: TsGs::Ts,
                sis: true,
                ccm: true,
                issyi: false,
                npd: false,
                ext: 0,
                isi: 0,
            },
            upl: 1504,
            sync: TS_SYNC,
            dfl: 1504,
            syncd: 0,
            mode: Mode::Normal,
            issy_in_header: None,
        };
        let mut frame = hdr.serialize().to_vec();
        let mut data = [0u8; TS_LEN];
        data[0] = crc8(&[0u8; TS_LEN]);
        data[1..].copy_from_slice(&inner[1..]);
        frame.extend_from_slice(&data);
        frame
    }

    /// Build a HEM BBFrame (no NPD) containing one TS packet (187 bytes + prepend sync).
    fn hem_bbframe(inner: &[u8; TS_LEN]) -> Vec<u8> {
        let hdr = Bbheader {
            matype: Matype {
                ts_gs: TsGs::Ts,
                sis: true,
                ccm: true,
                issyi: false,
                npd: false,
                ext: 0,
                isi: 0,
            },
            upl: 0,
            sync: 0,
            dfl: (crate::packet::HEM_UP_SIZE * 8) as u16,
            syncd: 0,
            mode: Mode::HighEfficiency,
            issy_in_header: None,
        };
        let mut frame = hdr.serialize().to_vec();
        // HEM data: 187 bytes = inner[1..188] (no sync byte)
        frame.extend_from_slice(&inner[1..]);
        frame
    }

    // ══════════════════════════════════════════════════════════════════════
    // NM frame tests
    // ══════════════════════════════════════════════════════════════════════

    #[test]
    fn nm_frame_yields_inner_ts_packet() {
        let inner = inner_packet();
        let frame = nm_bbframe(&inner);

        let mut pump = BbframePump::new();
        let pkts = pump.feed(0, &frame);
        assert_eq!(pkts.len(), 1, "exactly one inner TS packet expected");
        assert_eq!(pkts[0][0], TS_SYNC, "sync byte restored");
        assert_eq!(&pkts[0][1..], &inner[1..]);
    }

    #[test]
    fn two_nm_frames_yield_two_packets() {
        let inner = inner_packet();
        let frame = nm_bbframe(&inner);

        let mut pump = BbframePump::new();
        let pkts1 = pump.feed(0, &frame).to_vec();
        let pkts2 = pump.feed(0, &frame).to_vec();
        assert_eq!(pkts1.len(), 1);
        assert_eq!(pkts2.len(), 1);
    }

    // ══════════════════════════════════════════════════════════════════════
    // HEM frame tests
    // ══════════════════════════════════════════════════════════════════════

    #[test]
    fn hem_frame_yields_inner_ts_packet() {
        let inner = inner_packet();
        let frame = hem_bbframe(&inner);

        let mut pump = BbframePump::new();
        let pkts = pump.feed(0, &frame);
        assert_eq!(pkts.len(), 1, "exactly one inner TS packet expected");
        assert_eq!(pkts[0][0], TS_SYNC, "sync byte prepended");
        // Bytes 1..188 match the original (minus the sync byte which is prepended).
        assert_eq!(&pkts[0][1..], &inner[1..]);
    }

    // ══════════════════════════════════════════════════════════════════════
    // Per-PLP interleaving
    // ══════════════════════════════════════════════════════════════════════

    #[test]
    fn interleaved_plps_keep_independent_carry_over() {
        let inner = inner_packet();
        let nm = nm_bbframe(&inner);

        let mut pump = BbframePump::new();

        // Feed PLP 0 twice, PLP 5 once.
        let pkts_0a = pump.feed(0, &nm).to_vec();
        let pkts_5 = pump.feed(5, &nm).to_vec();
        let pkts_0b = pump.feed(0, &nm).to_vec();

        assert_eq!(pkts_0a.len(), 1);
        assert_eq!(pkts_5.len(), 1);
        assert_eq!(pkts_0b.len(), 1);

        // Both PLPs produced a packet → independent extractors.
        assert_eq!(pkts_0a[0], pkts_0b[0]);
        assert_eq!(pkts_5[0], inner);
    }

    // ══════════════════════════════════════════════════════════════════════
    // Malformed input → stat bump, no panic
    // ══════════════════════════════════════════════════════════════════════

    #[test]
    fn short_df_bytes_bumps_header_parse_failures() {
        let mut pump = BbframePump::new();
        let pkts = pump.feed(0, &[0u8; 5]); // shorter than BBHEADER_LEN
        assert!(pkts.is_empty());
        assert_eq!(pump.stats().header_parse_failures, 1);
    }

    #[test]
    fn bad_bbheader_bumps_header_parse_failures() {
        let mut pump = BbframePump::new();
        let bad = [0xFFu8; BBHEADER_LEN + 10]; // CRC will fail → Bbheader::parse returns Err
        let pkts = pump.feed(0, &bad);
        assert!(pkts.is_empty());
        assert_eq!(pump.stats().header_parse_failures, 1);
    }

    #[test]
    fn non_ts_matype_bumps_non_ts_payloads() {
        // Build a NM BBFrame with GSE (non-TS) MATYPE.
        let hdr = Bbheader {
            matype: Matype {
                ts_gs: TsGs::Gse,
                sis: true,
                ccm: true,
                issyi: false,
                npd: false,
                ext: 0,
                isi: 0,
            },
            upl: 0,
            sync: 0,
            dfl: 100,
            syncd: 0,
            mode: Mode::Normal,
            issy_in_header: None,
        };
        let mut frame = hdr.serialize().to_vec();
        frame.extend_from_slice(&[0u8; 50]);

        let mut pump = BbframePump::new();
        let pkts = pump.feed(0, &frame);
        assert!(pkts.is_empty());
        assert_eq!(pump.stats().non_ts_payloads, 1);
        assert_eq!(pump.stats().header_parse_failures, 0); // header itself parsed fine
    }

    #[test]
    fn garbage_no_panic_no_output() {
        let mut pump = BbframePump::new();
        // Byte 9 is the CRC-8 XOR MODE byte; crc8([0u8; 9]) = 0, so 0xFF
        // gives MODE = 0xFF which is neither 0 (NM) nor 1 (HEM) →
        // Bbheader::parse fails → header_parse_failures bumped.
        let mut junk = [0u8; 200];
        junk[9] = 0xFF;
        assert!(pump.feed(0, &junk).is_empty());
        assert!(pump.stats().header_parse_failures > 0);
    }

    // ══════════════════════════════════════════════════════════════════════
    // Stats aggregation
    // ══════════════════════════════════════════════════════════════════════

    #[test]
    fn stats_aggregates_across_plps() {
        let inner = inner_packet();
        let nm = nm_bbframe(&inner);

        let mut pump = BbframePump::new();

        // Feed PLP 0 and PLP 1
        pump.feed(0, &nm);
        pump.feed(1, &nm);

        let s = pump.stats();
        // Each CarryOverExtractor fed one NM frame with no issues.
        assert_eq!(s.carry_over.header_parse_failures, 0);
        assert_eq!(s.carry_over.mode_mismatches, 0);
    }
}
