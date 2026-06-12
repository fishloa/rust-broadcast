//! Section → TS packetizer (the byte-exact inverse of
//! [`SectionReassembler::feed`](crate::ts::SectionReassembler::feed)).
//!
//! Per the PSI carriage rules of ISO/IEC 13818-1:2007 §2.4.4
//! (`docs/iso_13818_1_systems.md`): sections are packed into 188-byte packets
//! with a `pointer_field` where sections begin, concatenated contiguously, and
//! 0xFF-stuffed at the batch tail.

use core::time::Duration;

use crate::pid::well_known;
use crate::ts::{TsHeader, CC_MASK, TS_PACKET_SIZE};

/// Mask for the 4 most-significant `section_length` bits in a section's second
/// byte (ISO/IEC 13818-1 §2.4.4.1 — `section_length` is 12 bits).
const SECTION_LENGTH_HI_MASK: u8 = 0x0F;

/// Maximum data bytes in a PUSI=1 packet (188 − 4 header − 1 pointer_field). §2.4.4.
const PUSI_PAYLOAD_CAP: usize = 183;
/// Maximum data bytes in a continuation packet (188 − 4 header). §2.4.4.
const PAYLOAD_CAP: usize = 184;
/// Stuffing byte for unused TS payload bytes (ISO/IEC 13818-1 §2.4.4).
const STUFFING_BYTE: u8 = 0xFF;

/// Packetizes PSI/SI sections into 188-byte TS packets.
///
/// This is the byte-exact inverse of
/// [`SectionReassembler::feed`](crate::ts::SectionReassembler::feed): packets
/// produced here, when fed back through the reassembler, yield the same
/// sections in order.
///
/// ISO/IEC 13818-1:2007 §2.4.4 (`docs/iso_13818_1_systems.md`).
pub struct SectionPacketizer {
    pid: u16,
    continuity_counter: u8,
}

impl SectionPacketizer {
    /// Start a packetizer for `pid` with continuity_counter = 0.
    pub fn new(pid: u16) -> Self {
        Self {
            pid,
            continuity_counter: 0,
        }
    }

    /// Start at a specific continuity_counter (0..=15) — for resuming a stream.
    pub fn with_continuity(pid: u16, cc: u8) -> Self {
        Self {
            pid,
            continuity_counter: cc & CC_MASK,
        }
    }

    /// The PID this packetizer emits packets for.
    pub fn pid(&self) -> u16 {
        self.pid
    }

    /// The continuity_counter for the next emitted packet.
    pub fn continuity_counter(&self) -> u8 {
        self.continuity_counter
    }

    /// Packetize a batch of complete sections into 188-byte TS packets,
    /// appended to `out` (cleared first).
    ///
    /// Returns the number of packets appended.
    pub fn packetize_into(
        &mut self,
        sections: &[&[u8]],
        out: &mut Vec<[u8; TS_PACKET_SIZE]>,
    ) -> usize {
        out.clear();

        if sections.is_empty() {
            return 0;
        }

        // Concatenate all sections and record section-start byte offsets.
        let total_len: usize = sections.iter().map(|s| s.len()).sum();
        if total_len == 0 {
            return 0;
        }
        let mut data = Vec::with_capacity(total_len);
        let mut starts = Vec::with_capacity(sections.len());
        for s in sections {
            starts.push(data.len());
            data.extend_from_slice(s);
        }

        let count_before = out.len();
        let mut pos = 0usize;

        while pos < data.len() {
            // Smallest section-start offset ≥ pos.
            let next_start = starts.iter().copied().find(|&s| s >= pos);

            let pusi: bool;
            let pointer_field: u8;
            let cap: usize;

            if let Some(ns) = next_start {
                let diff = ns.saturating_sub(pos);
                if diff <= PUSI_PAYLOAD_CAP {
                    pusi = true;
                    pointer_field = diff as u8;
                    cap = PUSI_PAYLOAD_CAP;
                } else {
                    pusi = false;
                    pointer_field = 0;
                    cap = PAYLOAD_CAP;
                }
            } else {
                pusi = false;
                pointer_field = 0;
                cap = PAYLOAD_CAP;
            }

            let mut pkt = [0u8; TS_PACKET_SIZE];

            let header = TsHeader {
                tei: false,
                pusi,
                pid: self.pid,
                scrambling: 0,
                has_adaptation: false,
                has_payload: true,
                continuity_counter: self.continuity_counter,
            };
            header
                .serialize_into(&mut pkt[..4])
                .expect("4-byte header buffer");

            self.continuity_counter = (self.continuity_counter + 1) & CC_MASK;

            let mut write_pos = 4usize;

            if pusi {
                pkt[write_pos] = pointer_field;
                write_pos += 1;
            }

            let remaining = data.len() - pos;
            let to_copy = remaining.min(cap);
            pkt[write_pos..write_pos + to_copy].copy_from_slice(&data[pos..pos + to_copy]);
            pos += to_copy;
            write_pos += to_copy;

            // 0xFF-stuff remaining payload bytes.
            for b in &mut pkt[write_pos..] {
                *b = STUFFING_BYTE;
            }

            out.push(pkt);
        }

        out.len() - count_before
    }

    /// Allocating convenience wrapper over [`packetize_into`](Self::packetize_into).
    pub fn packetize(&mut self, sections: &[&[u8]]) -> Vec<[u8; TS_PACKET_SIZE]> {
        let mut out = Vec::new();
        self.packetize_into(sections, &mut out);
        out
    }
}

// ── Default interval constants ────────────────────────────────────────────────

/// dvb-si/docs/tr_101_211.md §4.4.1/§4.4.2 — NIT maximum interval.
pub const NIT_MAX_INTERVAL: Duration = Duration::from_secs(10);
/// dvb-si/docs/tr_101_211.md §4.4.1/§4.4.2 — BAT maximum interval.
pub const BAT_MAX_INTERVAL: Duration = Duration::from_secs(10);
/// dvb-si/docs/tr_101_211.md §4.4.1/§4.4.2 — SDT actual maximum interval.
pub const SDT_ACTUAL_MAX_INTERVAL: Duration = Duration::from_secs(2);
/// dvb-si/docs/tr_101_211.md §4.4.1/§4.4.2 — SDT other maximum interval.
pub const SDT_OTHER_MAX_INTERVAL: Duration = Duration::from_secs(10);
/// dvb-si/docs/tr_101_211.md §4.4.1/§4.4.2 — EIT p/f actual maximum interval.
pub const EIT_PF_ACTUAL_MAX_INTERVAL: Duration = Duration::from_secs(2);
/// dvb-si/docs/tr_101_211.md §4.4.1 — EIT p/f other maximum interval (sat/cable;
/// terrestrial is 20 s per §4.4.2).
pub const EIT_PF_OTHER_MAX_INTERVAL: Duration = Duration::from_secs(10);
/// dvb-si/docs/tr_101_211.md §4.4.1 — EIT schedule first 8 days maximum interval.
pub const EIT_SCHED_MAX_INTERVAL: Duration = Duration::from_secs(10);
/// dvb-si/docs/tr_101_211.md §4.4.1 — EIT schedule beyond 8 days maximum interval.
pub const EIT_SCHED_EXT_MAX_INTERVAL: Duration = Duration::from_secs(30);
/// dvb-si/docs/tr_101_211.md §4.4.1/§4.4.2 — TDT maximum interval.
pub const TDT_MAX_INTERVAL: Duration = Duration::from_secs(30);
/// dvb-si/docs/tr_101_211.md §4.4.1/§4.4.2 — TOT maximum interval.
pub const TOT_MAX_INTERVAL: Duration = Duration::from_secs(30);

/// dvb-si/docs/ts_101_154_av_coding.md §4.1.7 — PAT maximum interval (shall ≤ 100 ms).
pub const PAT_MAX_INTERVAL: Duration = Duration::from_millis(100);
/// dvb-si/docs/ts_101_154_av_coding.md §4.1.7 — PMT maximum interval (shall ≤ 100 ms).
pub const PMT_MAX_INTERVAL: Duration = Duration::from_millis(100);

/// dvb-si/docs/en_300_468.md §5.1.4.1 — minimum inter-section interval floor
/// (≤100 Mbit/s TSs).
pub const MIN_SECTION_INTERVAL: Duration = Duration::from_millis(25);

// ── SiMux ────────────────────────────────────────────────────────────────────

/// Section-repetition scheduler that builds TS packets on a caller-supplied clock.
///
/// Each entry is a PID + concatenated complete-section bytes + an emission
/// interval. Call [`poll_into`](Self::poll_into) with monotonically-increasing
/// `now` values to get 188-byte TS packets for every entry whose interval has
/// elapsed since its last emission.
///
/// The scheduler owns its section bytes — call [`upsert`](Self::upsert) to
/// update them when SI changes. Continuity counters are continuous per-PID
/// across poll cycles.
///
/// # 25 ms floor
///
/// [`MIN_SECTION_INTERVAL`] is the minimum valid interval (EN 300 468 §5.1.4.1).
/// Supplying an interval below this in [`upsert`](Self::upsert) triggers a
/// `debug_assert` — in release builds the assertion compiles out; the caller
/// must ensure compliance.
pub struct SiMux {
    entries: Vec<Entry>,
}

struct Entry {
    pid: u16,
    sections: Vec<u8>,
    interval: Duration,
    last_emit: Option<Duration>,
    packetizer: SectionPacketizer,
}

impl SiMux {
    /// Create an empty scheduler (no entries, no pending emissions).
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Register or replace the sections emitted on `pid` at `interval`.
    ///
    /// `sections` is the concatenated complete-section bytes for one emission
    /// cycle. Re-calling for the same `pid` updates the bytes and interval
    /// while preserving the continuity counter.
    ///
    /// # Panics (debug only)
    ///
    /// `debug_assert!(interval >= MIN_SECTION_INTERVAL)` — EN 300 468 §5.1.4.1
    /// requires a minimum inter-section interval of 25 ms.
    pub fn upsert(&mut self, pid: u16, sections: Vec<u8>, interval: Duration) {
        debug_assert!(
            interval >= MIN_SECTION_INTERVAL,
            "interval {interval:?} is below the 25 ms minimum (EN 300 468 §5.1.4.1)"
        );

        if let Some(entry) = self.entries.iter_mut().find(|e| e.pid == pid) {
            entry.sections = sections;
            entry.interval = interval;
        } else {
            self.entries.push(Entry {
                pid,
                sections,
                interval,
                last_emit: None,
                packetizer: SectionPacketizer::new(pid),
            });
        }
    }

    /// PAT at [`PAT_MAX_INTERVAL`] on PID 0x0000.
    ///
    /// Interval cites dvb-si/docs/ts_101_154_av_coding.md §4.1.7.
    pub fn upsert_pat(&mut self, sections: Vec<u8>) {
        self.upsert(well_known::PAT.value(), sections, PAT_MAX_INTERVAL);
    }

    /// PMT at [`PMT_MAX_INTERVAL`] on the caller-supplied `pid`.
    ///
    /// Interval cites dvb-si/docs/ts_101_154_av_coding.md §4.1.7.
    pub fn upsert_pmt(&mut self, pid: u16, sections: Vec<u8>) {
        self.upsert(pid, sections, PMT_MAX_INTERVAL);
    }

    /// SDT actual at [`SDT_ACTUAL_MAX_INTERVAL`] on PID 0x0011.
    ///
    /// Interval cites dvb-si/docs/tr_101_211.md §4.4.1/§4.4.2.
    pub fn upsert_sdt_actual(&mut self, sections: Vec<u8>) {
        self.upsert(
            well_known::SDT_BAT.value(),
            sections,
            SDT_ACTUAL_MAX_INTERVAL,
        );
    }

    /// NIT at [`NIT_MAX_INTERVAL`] on PID 0x0010.
    ///
    /// Interval cites dvb-si/docs/tr_101_211.md §4.4.1/§4.4.2.
    pub fn upsert_nit(&mut self, sections: Vec<u8>) {
        self.upsert(well_known::NIT.value(), sections, NIT_MAX_INTERVAL);
    }

    /// TDT at [`TDT_MAX_INTERVAL`] on PID 0x0014.
    ///
    /// Interval cites dvb-si/docs/tr_101_211.md §4.4.1/§4.4.2.
    pub fn upsert_tdt(&mut self, sections: Vec<u8>) {
        self.upsert(well_known::TDT_TOT.value(), sections, TDT_MAX_INTERVAL);
    }

    /// TOT at [`TOT_MAX_INTERVAL`] on PID 0x0014.
    ///
    /// Interval cites dvb-si/docs/tr_101_211.md §4.4.1/§4.4.2.
    pub fn upsert_tot(&mut self, sections: Vec<u8>) {
        self.upsert(well_known::TDT_TOT.value(), sections, TOT_MAX_INTERVAL);
    }

    /// Emit every entry due at `now` (i.e. `now - last_emit >= interval`, and
    /// first call always due), packetizing via [`SectionPacketizer`], appended
    /// to `out` (cleared first).
    ///
    /// Updates each emitted entry's `last_emit = now`. Deterministic given the
    /// fed `now` sequence. Returns the packet count appended.
    pub fn poll_into(&mut self, now: Duration, out: &mut Vec<[u8; TS_PACKET_SIZE]>) -> usize {
        out.clear();
        let before = out.len();

        let mut tmp = Vec::new();
        for entry in &mut self.entries {
            let due = match entry.last_emit {
                None => true,
                Some(last) => now.saturating_sub(last) >= entry.interval,
            };
            if due {
                let refs = split_sections(&entry.sections);
                if !refs.is_empty() {
                    entry.packetizer.packetize_into(&refs, &mut tmp);
                    out.append(&mut tmp);
                }
                entry.last_emit = Some(now);
            }
        }

        out.len() - before
    }

    /// Allocating convenience wrapper over [`poll_into`](Self::poll_into).
    pub fn poll(&mut self, now: Duration) -> Vec<[u8; TS_PACKET_SIZE]> {
        let mut out = Vec::new();
        self.poll_into(now, &mut out);
        out
    }
}

impl Default for SiMux {
    fn default() -> Self {
        Self::new()
    }
}

/// Split concatenated complete-section bytes into individual `&[u8]` slices.
///
/// Walks the PSI/SI section headers: byte 0 = table_id, bytes 1-2 =
/// section_length (12 bits). Each slice is 3 + section_length bytes long.
/// Trailing bytes that don't form a complete section header are discarded.
fn split_sections(data: &[u8]) -> Vec<&[u8]> {
    let mut result = Vec::new();
    let mut pos = 0;
    while pos + 3 <= data.len() {
        let section_length =
            (((data[pos + 1] & SECTION_LENGTH_HI_MASK) as usize) << 8) | (data[pos + 2] as usize);
        let end = pos + 3 + section_length;
        if end > data.len() {
            break;
        }
        result.push(&data[pos..end]);
        pos = end;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ts::{SectionReassembler, TsPacket};

    // ── helpers ──────────────────────────────────────────────────────────────

    /// Build a long-form section with the given table_id and body bytes.
    /// Returns the full section including its 3-byte header (no CRC — the
    /// reassembler does not validate CRC).
    fn build_section(table_id: u8, body_after_length: &[u8]) -> Vec<u8> {
        let section_length = body_after_length.len() as u16;
        let mut v = Vec::with_capacity(3 + section_length as usize);
        v.push(table_id);
        // SSI=1, PI=0, reserved=11, length upper 4 bits
        v.push(0xB0 | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(body_after_length);
        v
    }

    fn concat_sections(sections: &[Vec<u8>]) -> Vec<u8> {
        let total: usize = sections.iter().map(|s| s.len()).sum();
        let mut out = Vec::with_capacity(total);
        for s in sections {
            out.extend_from_slice(s);
        }
        out
    }

    /// Round-trip `sections` through packetize → reassembler, asserting
    /// byte-identical output in order and no leftovers.
    fn assert_round_trip(sections: &[Vec<u8>]) {
        let mut packetizer = SectionPacketizer::new(0x0100);
        let refs: Vec<&[u8]> = sections.iter().map(|s| s.as_slice()).collect();
        let packets = packetizer.packetize(&refs);

        let mut reasm = SectionReassembler::default();
        for pkt_raw in &packets {
            let pkt = TsPacket::parse(pkt_raw).expect("parse generated packet");
            let payload = pkt.payload.expect("payload present");
            let pusi = pkt.header.pusi;
            reasm.feed(payload, pusi);
        }

        let got: Vec<_> = std::iter::from_fn(|| reasm.pop_section()).collect();
        assert_eq!(
            got.len(),
            sections.len(),
            "section count mismatch: expected {}, got {}",
            sections.len(),
            got.len()
        );
        for (i, (orig, round)) in sections.iter().zip(got.iter()).enumerate() {
            assert_eq!(
                round.as_ref(),
                orig.as_slice(),
                "section {i} round-trip mismatch"
            );
        }
        assert!(reasm.is_empty(), "reassembler should be empty after drain");
    }

    // ── split_sections ──────────────────────────────────────────────────────

    #[test]
    fn split_single_section() {
        let s = build_section(0x42, &[0xAA; 5]);
        let refs = split_sections(&s);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0], s.as_slice());
    }

    #[test]
    fn split_two_sections() {
        let s1 = build_section(0x42, &[0x01, 0x02]);
        let s2 = build_section(0x46, &[0x03, 0x04, 0x05]);
        let both = concat_sections(&[s1.clone(), s2.clone()]);
        let refs = split_sections(&both);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0], s1.as_slice());
        assert_eq!(refs[1], s2.as_slice());
    }

    #[test]
    fn split_empty_input() {
        let refs = split_sections(&[]);
        assert!(refs.is_empty());
    }

    #[test]
    fn split_trailing_garbage_ignored() {
        let s = build_section(0x42, &[0xAA; 3]);
        let mut data = s.clone();
        data.push(0xFF); // trailing byte that doesn't complete a header
        let refs = split_sections(&data);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0], s.as_slice());
    }

    // ── interval constants pinned to spec ────────────────────────────────────

    #[test]
    fn interval_constants_match_spec() {
        // TR 101 211 §4.4.1/§4.4.2
        assert_eq!(NIT_MAX_INTERVAL, Duration::from_secs(10));
        assert_eq!(BAT_MAX_INTERVAL, Duration::from_secs(10));
        assert_eq!(SDT_ACTUAL_MAX_INTERVAL, Duration::from_secs(2));
        assert_eq!(SDT_OTHER_MAX_INTERVAL, Duration::from_secs(10));
        assert_eq!(EIT_PF_ACTUAL_MAX_INTERVAL, Duration::from_secs(2));
        assert_eq!(EIT_PF_OTHER_MAX_INTERVAL, Duration::from_secs(10));
        assert_eq!(EIT_SCHED_MAX_INTERVAL, Duration::from_secs(10));
        assert_eq!(EIT_SCHED_EXT_MAX_INTERVAL, Duration::from_secs(30));
        assert_eq!(TDT_MAX_INTERVAL, Duration::from_secs(30));
        assert_eq!(TOT_MAX_INTERVAL, Duration::from_secs(30));

        // TS 101 154 §4.1.7
        assert_eq!(PAT_MAX_INTERVAL, Duration::from_millis(100));
        assert_eq!(PMT_MAX_INTERVAL, Duration::from_millis(100));

        // EN 300 468 §5.1.4.1
        assert_eq!(MIN_SECTION_INTERVAL, Duration::from_millis(25));
    }

    // ── SiMux: basic behaviour ───────────────────────────────────────────────

    #[test]
    fn new_simux_is_empty() {
        let mut mux = SiMux::new();
        let pkts = mux.poll(Duration::ZERO);
        assert!(pkts.is_empty());
    }

    #[test]
    fn simux_default_is_empty() {
        let mut mux = SiMux::default();
        let pkts = mux.poll(Duration::ZERO);
        assert!(pkts.is_empty());
    }

    #[test]
    fn first_poll_always_emits() {
        let s = build_section(0x42, &[0xAA; 10]);
        let mut mux = SiMux::new();
        mux.upsert(0x0100, s.clone(), Duration::from_millis(100));
        let pkts = mux.poll(Duration::ZERO);
        assert!(!pkts.is_empty(), "first poll must emit");
    }

    #[test]
    fn first_poll_emits_all_entries() {
        let s1 = build_section(0x42, &[0x01; 5]);
        let s2 = build_section(0x46, &[0x02; 5]);
        let mut mux = SiMux::new();
        mux.upsert(0x0100, s1.clone(), Duration::from_millis(100));
        mux.upsert(0x0200, s2.clone(), Duration::from_millis(200));
        let pkts = mux.poll(Duration::ZERO);
        // Both entries should emit — we count PID occurrences
        let pids: Vec<u16> = pkts
            .iter()
            .map(|raw| TsPacket::parse(raw).unwrap().header.pid)
            .collect();
        assert!(pids.contains(&0x0100));
        assert!(pids.contains(&0x0200));
    }

    // ── deterministic schedule ───────────────────────────────────────────────

    #[test]
    fn entry_emits_only_when_due() {
        let s = build_section(0x42, &[0xAA; 10]);
        let mut mux = SiMux::new();
        mux.upsert(0x0100, s.clone(), Duration::from_millis(100));

        // First poll at t=0: emits
        let pkts0 = mux.poll(Duration::ZERO);
        assert!(!pkts0.is_empty());

        // t=50: not yet due
        let pkts50 = mux.poll(Duration::from_millis(50));
        assert!(pkts50.is_empty(), "should not emit at t=50");

        // t=99: still not due
        let pkts99 = mux.poll(Duration::from_millis(99));
        assert!(pkts99.is_empty(), "should not emit at t=99");

        // t=100: due again
        let pkts100 = mux.poll(Duration::from_millis(100));
        assert!(!pkts100.is_empty(), "should emit at t=100");
    }

    #[test]
    fn two_entries_different_cadence() {
        let s1 = build_section(0x42, &[0x01; 5]);
        let s2 = build_section(0x46, &[0x02; 5]);
        let mut mux = SiMux::new();
        mux.upsert(0x0100, concat_sections(&[s1]), Duration::from_millis(100));
        mux.upsert(0x0200, concat_sections(&[s2]), Duration::from_millis(200));

        // t=0: both emit
        let pkts = mux.poll(Duration::ZERO);
        assert!(!pkts.is_empty());

        // t=100: only 0x0100 emits (due at 100)
        let pkts100 = mux.poll(Duration::from_millis(100));
        assert!(!pkts100.is_empty());
        let pids100: Vec<u16> = pkts100
            .iter()
            .map(|raw| TsPacket::parse(raw).unwrap().header.pid)
            .collect();
        assert!(pids100.contains(&0x0100), "PID 0x0100 should emit at t=100");
        assert!(
            !pids100.contains(&0x0200),
            "PID 0x0200 should NOT emit at t=100"
        );

        // t=150: neither emits
        let pkts150 = mux.poll(Duration::from_millis(150));
        assert!(pkts150.is_empty(), "neither should emit at t=150");

        // t=200: both emit again
        let pkts200 = mux.poll(Duration::from_millis(200));
        let pids200: Vec<u16> = pkts200
            .iter()
            .map(|raw| TsPacket::parse(raw).unwrap().header.pid)
            .collect();
        assert!(pids200.contains(&0x0100), "PID 0x0100 should emit at t=200");
        assert!(pids200.contains(&0x0200), "PID 0x0200 should emit at t=200");
    }

    #[test]
    fn entry_emits_again_after_full_interval() {
        let s = build_section(0x42, &[0xAA; 20]);
        let mut mux = SiMux::new();
        mux.upsert(0x0100, s, Duration::from_secs(1));

        // t=0
        let n0 = mux.poll_into(Duration::ZERO, &mut Vec::new());
        assert!(n0 > 0);

        // t=999ms: not due
        let mut buf = Vec::new();
        let n999 = mux.poll_into(Duration::from_millis(999), &mut buf);
        assert_eq!(n999, 0);

        // t=1s: due
        let mut buf2 = Vec::new();
        let n1000 = mux.poll_into(Duration::from_millis(1000), &mut buf2);
        assert!(n1000 > 0);
    }

    // ── upsert replaces existing entry ───────────────────────────────────────

    #[test]
    fn upsert_updates_existing_entry() {
        let s1 = build_section(0x42, &[0xAA; 5]);
        let s2 = build_section(0x46, &[0xBB; 10]);
        let mut mux = SiMux::new();

        mux.upsert(0x0100, s1, Duration::from_millis(100));
        // Replace sections on same PID
        mux.upsert(0x0100, s2.clone(), Duration::from_millis(200));

        let pkts = mux.poll(Duration::ZERO);

        // Round-trip the output to verify the updated sections
        let mut reasm = SectionReassembler::default();
        for raw in &pkts {
            let pkt = TsPacket::parse(raw).unwrap();
            if pkt.header.pid == 0x0100 {
                reasm.feed(pkt.payload.unwrap(), pkt.header.pusi);
            }
        }
        let got: Vec<_> = std::iter::from_fn(|| reasm.pop_section()).collect();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].as_ref(), s2.as_slice());
    }

    // ── convenience constructors ─────────────────────────────────────────────

    #[test]
    fn upsert_pat_uses_correct_pid() {
        let s = build_section(0x00, &[0x01, 0x02]);
        let mut mux = SiMux::new();
        mux.upsert_pat(s);
        let pkts = mux.poll(Duration::ZERO);
        let pids: Vec<u16> = pkts
            .iter()
            .map(|raw| TsPacket::parse(raw).unwrap().header.pid)
            .collect();
        assert!(pids.iter().all(|&p| p == well_known::PAT.value()));
    }

    #[test]
    fn upsert_sdt_actual_uses_correct_pid() {
        let s = build_section(0x42, &[0x01]);
        let mut mux = SiMux::new();
        mux.upsert_sdt_actual(s);
        let pkts = mux.poll(Duration::ZERO);
        let pids: Vec<u16> = pkts
            .iter()
            .map(|raw| TsPacket::parse(raw).unwrap().header.pid)
            .collect();
        assert!(pids.iter().all(|&p| p == well_known::SDT_BAT.value()));
    }

    #[test]
    fn upsert_nit_uses_correct_pid() {
        let s = build_section(0x40, &[0x01]);
        let mut mux = SiMux::new();
        mux.upsert_nit(s);
        let pkts = mux.poll(Duration::ZERO);
        let pids: Vec<u16> = pkts
            .iter()
            .map(|raw| TsPacket::parse(raw).unwrap().header.pid)
            .collect();
        assert!(pids.iter().all(|&p| p == well_known::NIT.value()));
    }

    #[test]
    fn upsert_tdt_uses_correct_pid() {
        let s = build_section(0x70, &[0x01]);
        let mut mux = SiMux::new();
        mux.upsert_tdt(s);
        let pkts = mux.poll(Duration::ZERO);
        let pids: Vec<u16> = pkts
            .iter()
            .map(|raw| TsPacket::parse(raw).unwrap().header.pid)
            .collect();
        assert!(pids.iter().all(|&p| p == well_known::TDT_TOT.value()));
    }

    #[test]
    fn upsert_tot_uses_correct_pid() {
        let s = build_section(0x73, &[0x01]);
        let mut mux = SiMux::new();
        mux.upsert_tot(s);
        let pkts = mux.poll(Duration::ZERO);
        let pids: Vec<u16> = pkts
            .iter()
            .map(|raw| TsPacket::parse(raw).unwrap().header.pid)
            .collect();
        assert!(pids.iter().all(|&p| p == well_known::TDT_TOT.value()));
    }

    // ── round-trip through reassembler ───────────────────────────────────────

    #[test]
    fn simux_round_trip_single_entry() {
        let s1 = build_section(0x42, &[0xAA; 20]);
        let s2 = build_section(0x46, &[0xBB; 15]);
        let mut mux = SiMux::new();
        mux.upsert(
            0x0100,
            concat_sections(&[s1.clone(), s2.clone()]),
            Duration::from_millis(100),
        );

        let pkts = mux.poll(Duration::ZERO);

        let mut reasm = SectionReassembler::default();
        for raw in &pkts {
            let pkt = TsPacket::parse(raw).unwrap();
            if pkt.header.pid == 0x0100 {
                reasm.feed(pkt.payload.unwrap(), pkt.header.pusi);
            }
        }
        let got: Vec<_> = std::iter::from_fn(|| reasm.pop_section()).collect();
        assert_eq!(got.len(), 2, "round-trip must recover both sections");
        assert_eq!(got[0].as_ref(), s1.as_slice());
        assert_eq!(got[1].as_ref(), s2.as_slice());
    }

    #[test]
    fn simux_round_trip_multi_pid() {
        let s_a = build_section(0x42, &[0xA0; 10]);
        let s_b = build_section(0x46, &[0xB0; 10]);
        let mut mux = SiMux::new();
        mux.upsert(0x0100, s_a.clone(), Duration::from_millis(100));
        mux.upsert(0x0200, s_b.clone(), Duration::from_millis(100));

        let pkts = mux.poll(Duration::ZERO);

        let mut reasm_a = SectionReassembler::default();
        let mut reasm_b = SectionReassembler::default();
        for raw in &pkts {
            let pkt = TsPacket::parse(raw).unwrap();
            match pkt.header.pid {
                0x0100 => reasm_a.feed(pkt.payload.unwrap(), pkt.header.pusi),
                0x0200 => reasm_b.feed(pkt.payload.unwrap(), pkt.header.pusi),
                _ => {}
            }
        }
        let got_a: Vec<_> = std::iter::from_fn(|| reasm_a.pop_section()).collect();
        let got_b: Vec<_> = std::iter::from_fn(|| reasm_b.pop_section()).collect();
        assert_eq!(got_a.len(), 1);
        assert_eq!(got_b.len(), 1);
        assert_eq!(got_a[0].as_ref(), s_a.as_slice());
        assert_eq!(got_b[0].as_ref(), s_b.as_slice());
    }

    // ── CC continuity across polls ───────────────────────────────────────────

    #[test]
    fn continuity_counter_continuous_across_polls() {
        // Section large enough to span at least 2 packets.
        let s = build_section(0x42, &[0xAA; 250]);
        let mut mux = SiMux::new();
        mux.upsert(0x0100, s, Duration::from_millis(100));

        let pkts1 = mux.poll(Duration::ZERO);
        assert!(pkts1.len() >= 2, "need ≥2 packets to test CC continuity");

        let last_cc_pkts1 = TsPacket::parse(&pkts1[pkts1.len() - 1])
            .unwrap()
            .header
            .continuity_counter;

        let pkts2 = mux.poll(Duration::from_millis(100));
        assert!(!pkts2.is_empty(), "second poll must emit");

        let first_cc_pkts2 = TsPacket::parse(&pkts2[0])
            .unwrap()
            .header
            .continuity_counter;

        assert_eq!(
            first_cc_pkts2,
            (last_cc_pkts1 + 1) & 0x0F,
            "CC must continue across poll cycles"
        );
    }

    // ── poll_into clears output ──────────────────────────────────────────────

    #[test]
    fn poll_into_clears_out_before_appending() {
        let s = build_section(0x42, &[0xAA; 5]);
        let mut mux = SiMux::new();
        mux.upsert(0x0100, s, Duration::from_millis(100));

        let mut out = vec![[0u8; TS_PACKET_SIZE]; 42];
        let n = mux.poll_into(Duration::ZERO, &mut out);
        assert_eq!(n, out.len(), "out must contain only new packets");
    }

    // ── debug_assert on sub-25 ms interval ───────────────────────────────────

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "25 ms")]
    fn upsert_rejects_sub_25ms_interval_in_debug() {
        let s = build_section(0x42, &[0xAA; 5]);
        let mut mux = SiMux::new();
        mux.upsert(0x0100, s, Duration::from_millis(10));
    }

    #[test]
    fn upsert_accepts_25ms_interval() {
        let s = build_section(0x42, &[0xAA; 5]);
        let mut mux = SiMux::new();
        mux.upsert(0x0100, s, MIN_SECTION_INTERVAL);
        let pkts = mux.poll(Duration::ZERO);
        assert!(!pkts.is_empty());
    }

    // ── round-trip property (the mandatory acceptance oracle) ────────────────

    #[test]
    fn round_trip_single_short_section() {
        let s = build_section(0x42, &[0xAA; 10]);
        assert_round_trip(&[s]);
    }

    #[test]
    fn round_trip_one_byte_body() {
        let s = build_section(0x46, &[0xBB]); // 4 bytes total
        assert_round_trip(&[s]);
    }

    #[test]
    fn round_trip_section_exactly_pusi_cap_boundary() {
        // A section whose total length is exactly PUSI_PAYLOAD_CAP (183).
        let body = vec![0xCC; PUSI_PAYLOAD_CAP - 3];
        let s = build_section(0x50, &body);
        assert_eq!(s.len(), PUSI_PAYLOAD_CAP);
        assert_round_trip(&[s]);
    }

    #[test]
    fn round_trip_section_just_over_pusi_cap() {
        // One byte more than fits in a PUSI packet → must span to continuation.
        let body = vec![0xDD; PUSI_PAYLOAD_CAP - 3 + 1];
        let s = build_section(0x52, &body);
        assert_eq!(s.len(), PUSI_PAYLOAD_CAP + 1);
        assert_round_trip(&[s]);
    }

    #[test]
    fn round_trip_section_spans_many_packets() {
        // A 2000-byte section spans ~11 packets.
        let body = vec![0xEE; 2000 - 3];
        let s = build_section(0x60, &body);
        assert_round_trip(&[s]);
    }

    #[test]
    fn round_trip_section_at_max_size() {
        // The maximum section (4096 total, the long-form ceiling), whose final
        // continuation packet carries the tail followed by 0xFF stuffing. Since
        // #148 the reassembler ignores that trailing stuffing instead of
        // counting it toward MAX_SECTION_SIZE, so a full-size section round-trips.
        let body = vec![0x11; 4096 - 3];
        let s = build_section(0x80, &body);
        assert_eq!(s.len(), 4096);
        assert_round_trip(&[s]);
    }

    #[test]
    fn round_trip_multiple_short_sections_in_one_batch() {
        let s1 = build_section(0x42, &[0x01, 0x02]); // 5 bytes
        let s2 = build_section(0x46, &[0x03]); // 4 bytes
        let s3 = build_section(0x4A, &[0x04, 0x05, 0x06]); // 6 bytes
        assert_round_trip(&[s1, s2, s3]);
    }

    #[test]
    fn round_trip_section_ends_exactly_at_boundary() {
        // First section is exactly PUSI_PAYLOAD_CAP bytes — ends at packet
        // boundary.  Second section starts fresh in the next packet with
        // PUSI=1, pointer_field=0.
        let body1 = vec![0xA1; PUSI_PAYLOAD_CAP - 3];
        let s1 = build_section(0x50, &body1);
        assert_eq!(s1.len(), PUSI_PAYLOAD_CAP);

        let s2 = build_section(0x52, &[0xB1, 0xB2]);
        assert_round_trip(&[s1, s2]);
    }

    #[test]
    fn round_trip_mix_small_large_sections() {
        // Mix of small and spanning sections that stress pointer_field and
        // concatenation.
        let s1 = build_section(0x10, &[0xAA; 5]);
        let body2 = vec![0xBB; 200];
        let s2 = build_section(0x20, &body2);
        let s3 = build_section(0x30, &[0xCC; 50]);
        let body4 = vec![0xDD; 800];
        let s4 = build_section(0x40, &body4);
        let s5 = build_section(0x50, &[0xEE]); // 1-byte body
        assert_round_trip(&[s1, s2, s3, s4, s5]);
    }

    // ── continuity counter ───────────────────────────────────────────────────

    #[test]
    fn continuity_counter_increments_per_packet() {
        // Use a section large enough to span several packets.
        let body = vec![0xAA; 500];
        let section = build_section(0x42, &body);
        let mut p = SectionPacketizer::new(0x0100);

        let packets = p.packetize(&[&section]);
        assert!(packets.len() >= 3, "need multiple packets to test CC");

        let mut last_cc: Option<u8> = None;
        for pkt_raw in &packets {
            let pkt = TsPacket::parse(pkt_raw).unwrap();
            let cc = pkt.header.continuity_counter;
            if let Some(last) = last_cc {
                assert_eq!(cc, (last + 1) & 0x0F, "CC must increment per packet");
            }
            last_cc = Some(cc);
        }
    }

    #[test]
    fn continuity_counter_wraps_and_continues_across_calls() {
        let mut p = SectionPacketizer::with_continuity(0x0100, 14);
        // Section large enough to span at least 3 packets.
        let body = vec![0xBB; 500];
        let s = build_section(0x42, &body);

        // First call: CC 14, 15, 0, …
        let pkts1 = p.packetize(&[&s]);
        assert!(pkts1.len() >= 3, "section must span ≥3 packets");
        let ccs1: Vec<u8> = pkts1
            .iter()
            .map(|b| TsPacket::parse(b).unwrap().header.continuity_counter)
            .collect();
        assert_eq!(ccs1[0], 14);
        assert_eq!(ccs1[1], 15);
        assert_eq!(ccs1[2], 0);

        // Second call: CC continues from where first left off.
        let pkts2 = p.packetize(&[&s]);
        let cc_first_pkt2 = TsPacket::parse(&pkts2[0])
            .unwrap()
            .header
            .continuity_counter;
        assert_eq!(cc_first_pkt2, ccs1.last().map(|c| (c + 1) & 0x0F).unwrap());
    }

    // ── PUSI placement ──────────────────────────────────────────────────────

    #[test]
    fn pusi_set_when_section_starts() {
        let s = build_section(0x42, &[0xAA; 10]);
        let mut p = SectionPacketizer::new(0x0100);
        let packets = p.packetize(&[&s]);
        assert!(!packets.is_empty());
        let pkt = TsPacket::parse(&packets[0]).unwrap();
        assert!(pkt.header.pusi, "first packet must have PUSI=1");
    }

    #[test]
    fn pusi_not_set_on_mid_section_continuation() {
        let body = vec![0xAA; 500];
        let s = build_section(0x42, &body);
        let mut p = SectionPacketizer::new(0x0100);
        let packets = p.packetize(&[&s]);
        assert!(packets.len() >= 2);
        let pkt1 = TsPacket::parse(&packets[0]).unwrap();
        let pkt2 = TsPacket::parse(&packets[1]).unwrap();
        assert!(pkt1.header.pusi, "first packet must have PUSI=1");
        assert!(
            !pkt2.header.pusi,
            "second packet is continuation, must have PUSI=0"
        );
    }

    #[test]
    fn pointer_field_equals_tail_length_before_new_section() {
        // Section1 = 200 bytes.  Section2 = 50 bytes.
        // Packet 1: PUSI=1, pointer=0, section1 head.
        // Packet 2: PUSI=1, pointer > 0 (tail of section1 before section2).
        let body1 = vec![0xA1; 197]; // 200-byte section
        let s1 = build_section(0x52, &body1);
        assert_eq!(s1.len(), 200);
        let s2 = build_section(0x54, &[0xB1; 47]); // 50-byte section
        assert_eq!(s2.len(), 50);

        let mut p = SectionPacketizer::new(0x0100);
        let packets = p.packetize(&[&s1, &s2]);

        // Find the packet where PUSI=1 and pointer>0.
        let pkt_with_pointer = packets
            .iter()
            .map(|raw| TsPacket::parse(raw).unwrap())
            .find(|pkt| pkt.header.pusi && pkt.payload.is_some_and(|pl| pl.first() != Some(&0)))
            .expect("must have a PUSI packet with non-zero pointer");

        let payload = pkt_with_pointer.payload.unwrap();
        let pointer = payload[0] as usize;
        assert!(pointer > 0, "pointer must be non-zero");
        // The tail bytes should be from the end of section1.
        let tail_start = s1.len() - pointer;
        assert_eq!(&payload[1..1 + pointer], &s1[tail_start..]);
    }

    // ── stuffing ─────────────────────────────────────────────────────────────

    #[test]
    fn final_packet_unused_tail_is_stuffing() {
        let s = build_section(0x42, &[0xAA; 5]); // 8 bytes total
        let mut p = SectionPacketizer::new(0x0100);
        let packets = p.packetize(&[&s]);

        let pkt = TsPacket::parse(&packets[0]).unwrap();
        let payload = pkt.payload.unwrap();
        assert_eq!(payload[0], 0, "pointer_field should be 0");

        let section_end = 1 + s.len(); // after pointer + section
        assert!(
            section_end < payload.len(),
            "must have stuffing after section"
        );
        for &b in &payload[section_end..] {
            assert_eq!(b, STUFFING_BYTE, "all trailing bytes must be 0xFF");
        }
    }

    #[test]
    fn reassembler_discards_stuffing() {
        let s1 = build_section(0x42, &[0xAA; 10]);
        let s2 = build_section(0x46, &[0xBB; 5]);

        let mut p = SectionPacketizer::new(0x0100);
        let packets = p.packetize(&[&s1, &s2]);

        let mut reasm = SectionReassembler::default();
        for pkt_raw in &packets {
            let pkt = TsPacket::parse(pkt_raw).unwrap();
            reasm.feed(pkt.payload.unwrap(), pkt.header.pusi);
        }

        let got: Vec<_> = std::iter::from_fn(|| reasm.pop_section()).collect();
        assert_eq!(got.len(), 2);
        assert!(
            reasm.is_empty(),
            "stuffing tail must be discarded, not buffered"
        );
    }

    // ── misc ─────────────────────────────────────────────────────────────────

    #[test]
    fn empty_batch_produces_no_packets() {
        let mut p = SectionPacketizer::new(0x0100);
        let packets: Vec<[u8; TS_PACKET_SIZE]> = p.packetize(&[]);
        assert!(packets.is_empty());
    }

    #[test]
    fn packetize_into_clears_out_first() {
        let s = build_section(0x42, &[0xAA; 5]);
        let mut p = SectionPacketizer::new(0x0100);

        let mut out = vec![[0u8; TS_PACKET_SIZE]; 99]; // pre-existing junk
        let n = p.packetize_into(&[&s], &mut out);
        assert_eq!(n, out.len(), "out must contain only the new packets");
        // Verify the output is correct (round-trip).
        let mut reasm = SectionReassembler::default();
        for pkt_raw in &out {
            let pkt = TsPacket::parse(pkt_raw).unwrap();
            reasm.feed(pkt.payload.unwrap(), pkt.header.pusi);
        }
        let got = reasm.pop_section().unwrap();
        assert_eq!(got.as_ref(), s.as_slice());
    }

    #[test]
    fn pid_is_correct() {
        let p = SectionPacketizer::new(0x1234);
        assert_eq!(p.pid(), 0x1234);
    }

    #[test]
    fn with_continuity_masks_to_4_bits() {
        let p = SectionPacketizer::with_continuity(0x0100, 0xFE);
        assert_eq!(p.continuity_counter(), 0x0E);
    }

    #[test]
    fn has_payload_always_true_no_adaptation() {
        let s = build_section(0x42, &[0xAA; 50]);
        let mut p = SectionPacketizer::new(0x0100);
        let packets = p.packetize(&[&s]);
        for pkt_raw in &packets {
            let pkt = TsPacket::parse(pkt_raw).unwrap();
            assert!(pkt.header.has_payload, "every packet must carry payload");
            assert!(!pkt.header.has_adaptation, "no adaptation field is emitted");
            assert!(!pkt.header.tei, "TEI must be false");
            assert_eq!(pkt.header.scrambling, 0, "scrambling must be 0");
        }
    }
}
