//! [`SiDemux`] — PID-filtered, version-gated SI section pump.
//!
//! Feed 188-byte MPEG-TS packets in with [`SiDemux::feed`]; get back an
//! iterator of [`SectionEvent`]s — one per **changed** complete section.
//! The demux reassembles sections per PID (via
//! [`crate::ts::SectionReassembler`]), validates the CRC of CRC-bearing
//! sections, and suppresses repeats through a version gate so that a steady
//! carousel of unchanging tables produces no events after the first.
//!
//! Events own their bytes ([`bytes::Bytes`]) and are therefore `'static` and
//! cheap to clone; typed views ([`SectionEvent::table_section`],
//! [`SectionEvent::parse`]) borrow the event lazily.
//!
//! ```
//! use dvb_common::Serialize;
//! use dvb_si::demux::SiDemux;
//! use dvb_si::tables::AnyTableSection;
//! use dvb_si::tables::pat::{PatSection, PatEntry};
//!
//! const PMT_PID: u16 = 0x0100;
//! const TS_SYNC_BYTE: u8 = 0x47;
//! const PAYLOAD_UNIT_START_INDICATOR: u8 = 0x40;
//! const PAT_PID_LOW_BYTE: u8 = 0x00;
//! const PAYLOAD_ONLY: u8 = 0x10;
//! const POINTER_FIELD_START: u8 = 0x00;
//! const STUFFING_BYTE: u8 = 0xFF;
//!
//! // Build one PAT section and wrap it in a single 188-byte TS packet so the
//! // example is self-contained. In real code `packet` comes from your source.
//! let pat = PatSection {
//!     transport_stream_id: 1, version_number: 0, current_next_indicator: true,
//!     section_number: 0, last_section_number: 0,
//!     entries: vec![PatEntry { program_number: 1, pid: PMT_PID }],
//! };
//! let mut section = vec![0u8; pat.serialized_len()];
//! pat.serialize_into(&mut section).unwrap();
//! let mut packet = [STUFFING_BYTE; 188];
//! packet[0] = TS_SYNC_BYTE;
//! packet[1] = PAYLOAD_UNIT_START_INDICATOR;
//! packet[2] = PAT_PID_LOW_BYTE;
//! packet[3] = PAYLOAD_ONLY;
//! packet[4] = POINTER_FIELD_START;
//! packet[5..5 + section.len()].copy_from_slice(&section);
//!
//! let mut demux = SiDemux::builder().build();
//! let events: Vec<_> = demux.feed(&packet).collect();
//! assert_eq!(events.len(), 1);
//! match events[0].table_section() {
//!     Ok(AnyTableSection::PatSection(pat)) => {
//!         println!("PAT v{} on {}", events[0].version().unwrap_or(0), events[0].pid());
//!         assert_eq!(pat.entries[0].pid, PMT_PID);
//!     }
//!     other => panic!("expected PAT, got {other:?}"),
//! }
//! ```
//!
//! # Version gate
//!
//! Each `(pid, table_id, table_id_extension, section_number)` tuple is packed
//! into a `u64` key. The stored value is a change detector:
//!
//! - **Long-form** sections (`section_syntax_indicator == 1`, plus the TOT
//!   exception) carry a 5-bit `version_number` and a trailing CRC-32 — the
//!   gate stores `(version, crc32)`. A repeat with the same version *and* CRC
//!   is suppressed.
//! - **Short-form** sections without a CRC (TDT/RST/ST/DIT) have no version;
//!   the gate stores a CRC-32 *computed over the whole section* purely as a
//!   change hash. `table_id_extension` and `section_number` collapse to 0 in
//!   the key.
//!
//! # CRC policy
//!
//! CRC-bearing sections (every long-form section, plus the short-form TOT
//! which uniquely carries a CRC — ETSI EN 300 468 §5.2.6) are validated
//! before gating. Failures are dropped and counted in
//! [`Stats::crc_failures`]; they are never emitted and never update the gate.
//! TDT carries no CRC and is therefore never dropped for CRC reasons.

use std::collections::{HashMap, VecDeque};

use bytes::Bytes;

use crate::pid::Pid;
use crate::ts::{SectionReassembler, TsPacket};

/// table_id of the Program Association Table (PAT) — followed for PMT PIDs.
const PAT_TABLE_ID: u8 = 0x00;
/// table_id of the Time Offset Table — short-form (SSI=0) yet CRC-bearing.
const TOT_TABLE_ID: u8 = 0x73;
/// Minimum bytes required to read a section header (table_id + length field).
const MIN_SECTION_LEN: usize = 3;
/// Long-form extension header bytes (after the 3-byte common header).
const LONG_FORM_EXTRA: usize = 5;
/// Trailing CRC-32 length.
const CRC_LEN: usize = 4;

/// One complete, changed SI section. Owns its bytes — `'static`, cheap clone.
///
/// A `SectionEvent` is only ever constructed for a section that
/// (a) is at least 3 bytes long, and (b) if it carries a CRC, passed CRC
/// validation. So [`SectionEvent::crc_ok`] is always `true` and
/// [`SectionEvent::table_id`] never panics.
#[derive(Debug, Clone)]
pub struct SectionEvent {
    pid: Pid,
    bytes: Bytes,
}

impl SectionEvent {
    /// PID this section was carried on.
    #[must_use]
    pub fn pid(&self) -> Pid {
        self.pid
    }

    /// The full section bytes (header included, CRC included if present).
    #[must_use]
    pub fn bytes(&self) -> &Bytes {
        &self.bytes
    }

    /// The `table_id` (byte 0). Never panics — events are only built for
    /// sections of at least 3 bytes.
    #[must_use]
    pub fn table_id(&self) -> u8 {
        self.bytes[0]
    }

    /// True when this section uses the long-form syntax
    /// (`section_syntax_indicator == 1`).
    #[must_use]
    fn is_long_form(&self) -> bool {
        (self.bytes[1] & 0x80) != 0
    }

    /// 5-bit `version_number`, or `None` for short-form sections (which carry
    /// no version field). Note the TOT, despite being short-form, has no
    /// version field either, so this is `None` for it.
    #[must_use]
    pub fn version(&self) -> Option<u8> {
        if self.is_long_form() && self.bytes.len() > 5 {
            Some((self.bytes[5] >> 1) & 0x1F)
        } else {
            None
        }
    }

    /// 16-bit `table_id_extension`, or `None` for short-form sections.
    #[must_use]
    pub fn table_id_extension(&self) -> Option<u16> {
        if self.is_long_form() && self.bytes.len() > 4 {
            Some(((self.bytes[3] as u16) << 8) | self.bytes[4] as u16)
        } else {
            None
        }
    }

    /// `section_number`, or `None` for short-form sections.
    #[must_use]
    pub fn section_number(&self) -> Option<u8> {
        if self.is_long_form() && self.bytes.len() > 6 {
            Some(self.bytes[6])
        } else {
            None
        }
    }

    /// Always `true`: events are emitted only after CRC validation (or for
    /// CRC-less short-form sections, where there is nothing to validate).
    #[must_use]
    pub fn crc_ok(&self) -> bool {
        true
    }

    /// Typed table-section view (lazy, borrows this event's bytes).
    ///
    /// # Errors
    /// Propagates the parse error from the dispatched table-section type.
    pub fn table_section(&self) -> crate::Result<crate::tables::AnyTableSection<'_>> {
        crate::tables::AnyTableSection::parse(&self.bytes)
    }

    /// Typed table-section view with custom-registry support (lazy, borrows
    /// this event's bytes).
    ///
    /// Precedence: custom-registered parser → built-in dispatch → Unknown.
    /// See [`AnyTableSection::parse_with`][crate::tables::AnyTableSection::parse_with].
    ///
    /// # Errors
    /// Propagates the parse error from the dispatched table-section type.
    pub fn table_section_with(
        &self,
        registry: &crate::tables::registry::TableRegistry,
    ) -> crate::Result<crate::tables::AnyTableSection<'_>> {
        crate::tables::AnyTableSection::parse_with(registry, &self.bytes)
    }

    /// Type-keyed view: `event.parse::<EitSection>()`.
    ///
    /// # Errors
    /// Propagates `T::parse` errors.
    pub fn parse<'s, T: crate::traits::TableDef<'s>>(&'s self) -> crate::Result<T> {
        <T as dvb_common::Parse>::parse(&self.bytes)
    }
}

/// Section statistics, monotonically accumulated across `feed` calls.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// TS packets fed (every `feed` call increments this).
    pub packets: u64,
    /// Complete sections produced by the reassemblers (pre-gate, pre-CRC).
    pub sections_completed: u64,
    /// Sections emitted as events (changed, valid).
    pub emitted: u64,
    /// Sections suppressed by the version gate (unchanged repeats).
    pub suppressed: u64,
    /// Structurally invalid (sub-3-byte; cannot occur from the in-crate
    /// reassembler) and CRC-failed sections share this counter. Sections are
    /// dropped before emission; the gate is never updated for them.
    pub crc_failures: u64,
    /// TS packets that failed to parse (bad sync byte, too short).
    pub malformed_packets: u64,
    /// Gate entries evicted because the gate was at capacity.
    pub gate_evictions: u64,
}

/// What the gate remembers for one key, to decide "changed?".
#[derive(Clone, Copy, PartialEq, Eq)]
struct GateEntry {
    /// Long-form version_number, or 0 for short-form (unused there).
    version: u8,
    /// CRC-32 over the whole section — the change hash. For long-form this is
    /// the trailing CRC; for short-form it is computed over all bytes.
    crc: u32,
}

/// Configuration captured by [`SiDemuxBuilder`].
struct Config {
    follow_pat: bool,
    emit_repeats: bool,
    gate_capacity: usize,
}

/// Builder for [`SiDemux`].
///
/// Defaults: `follow_pat = true`, `dvb_si_pids = true`,
/// `emit_repeats = false`, `gate_capacity = 65_536`.
pub struct SiDemuxBuilder {
    follow_pat: bool,
    dvb_si_pids: bool,
    emit_repeats: bool,
    gate_capacity: usize,
    extra_pids: Vec<Pid>,
}

impl Default for SiDemuxBuilder {
    fn default() -> Self {
        Self {
            follow_pat: true,
            dvb_si_pids: true,
            emit_repeats: false,
            gate_capacity: 65_536,
            extra_pids: Vec::new(),
        }
    }
}

impl SiDemuxBuilder {
    /// When `true` (default), an emitted (changed) PAT auto-adds each
    /// programme's PMT PID to the watch set.
    #[must_use]
    pub fn follow_pat(mut self, on: bool) -> Self {
        self.follow_pat = on;
        self
    }

    /// When `true` (default), pre-populate the watch set with the well-known
    /// DVB/MPEG-2 SI PIDs (PAT, CAT, NIT, SDT/BAT, EIT, RST, TDT/TOT, SAT).
    #[must_use]
    pub fn dvb_si_pids(mut self, on: bool) -> Self {
        self.dvb_si_pids = on;
        self
    }

    /// Add a PID to the watch set (additive; may be called repeatedly).
    #[must_use]
    pub fn pid(mut self, pid: Pid) -> Self {
        self.extra_pids.push(pid);
        self
    }

    /// When `true`, emit every complete valid section, bypassing the version
    /// gate's suppression (the gate is still updated). Default `false`.
    #[must_use]
    pub fn emit_repeats(mut self, on: bool) -> Self {
        self.emit_repeats = on;
        self
    }

    /// Maximum number of distinct gate keys retained. At capacity the gate
    /// FIFO-evicts the oldest key. Default 65 536.
    #[must_use]
    pub fn gate_capacity(mut self, cap: usize) -> Self {
        self.gate_capacity = cap;
        self
    }

    /// Build the [`SiDemux`].
    #[must_use]
    pub fn build(self) -> SiDemux {
        let mut pids: HashMap<Pid, SectionReassembler> = HashMap::new();
        if self.dvb_si_pids {
            use crate::pid::well_known as wk;
            for pid in [
                wk::PAT,
                wk::CAT,
                wk::NIT,
                wk::SDT_BAT,
                wk::EIT,
                wk::RST,
                wk::TDT_TOT,
                wk::SAT,
            ] {
                pids.entry(pid).or_default();
            }
        }
        for p in self.extra_pids {
            pids.entry(p).or_default();
        }
        SiDemux {
            pids,
            gate: HashMap::new(),
            gate_order: VecDeque::new(),
            cfg: Config {
                follow_pat: self.follow_pat,
                emit_repeats: self.emit_repeats,
                gate_capacity: self.gate_capacity,
            },
            stats: Stats::default(),
            scratch: Vec::new(),
            completed_scratch: Vec::new(),
        }
    }
}

/// PID-filtered, version-gated SI section demultiplexer.
///
/// See the [module docs](crate::demux) for the gate and CRC policies.
pub struct SiDemux {
    pids: HashMap<Pid, SectionReassembler>,
    // TODO(perf): keys are uniform internal u64s — a non-SipHash hasher (e.g.
    // FxHash) would shave cycles at high section rates; revisit if profiling
    // shows it.
    gate: HashMap<u64, GateEntry>,
    gate_order: VecDeque<u64>,
    cfg: Config,
    stats: Stats,
    scratch: Vec<SectionEvent>,
    completed_scratch: Vec<Bytes>,
}

impl SiDemux {
    /// Start building a demux. See [`SiDemuxBuilder`] for defaults.
    #[must_use]
    pub fn builder() -> SiDemuxBuilder {
        SiDemuxBuilder::default()
    }

    /// Accumulated statistics.
    #[must_use]
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Feed one 188-byte TS packet. Infallible: malformed packets are counted
    /// in [`Stats::malformed_packets`], not raised. Returns an iterator over
    /// the changed sections this packet completed.
    pub fn feed(&mut self, packet: &[u8]) -> impl Iterator<Item = SectionEvent> + '_ {
        self.scratch.clear();
        self.stats.packets += 1;

        match TsPacket::parse(packet) {
            Err(_) => self.stats.malformed_packets += 1,
            Ok(ts) => {
                let pid = Pid::new(ts.header.pid);
                let payload = ts.payload.unwrap_or(&[]);
                // Single map lookup: feed and drain every section this packet
                // completed while holding the reassembler borrow, then process
                // them after the borrow ends (`consider` may insert new PMT
                // PIDs into the map, so it cannot run while the borrow is live).
                // A non-watched PID costs exactly one lookup and stops here;
                // `completed` does not allocate until a section actually pops.
                let mut completed = std::mem::take(&mut self.completed_scratch);
                if let Some(reasm) = self.pids.get_mut(&pid) {
                    reasm.feed(payload, ts.header.pusi);
                    while let Some(section) = reasm.pop_section() {
                        completed.push(section);
                    }
                }
                self.stats.sections_completed += completed.len() as u64;
                for section in completed.drain(..) {
                    self.consider(pid, section);
                }
                self.completed_scratch = completed;
            }
        }

        self.scratch.drain(..)
    }

    /// Gate + CRC + (maybe) push to scratch. Handles PAT-follow on emit.
    fn consider(&mut self, pid: Pid, section: Bytes) {
        // Guard: sub-3-byte sections cannot carry a header. The reassembler
        // should never emit one (it needs >= 3 bytes to know `expected`), but
        // guard defensively and count it as a CRC failure bucket — it is a
        // structurally invalid section, dropped without emission.
        if section.len() < MIN_SECTION_LEN {
            self.stats.crc_failures += 1;
            return;
        }

        let table_id = section[0];
        let long_form = (section[1] & 0x80) != 0;
        // The TOT is short-form by its SSI bit but uniquely carries a CRC.
        let has_crc = long_form || table_id == TOT_TABLE_ID;

        // CRC policy: validate CRC-bearing sections before gating.
        if has_crc {
            if section.len() < CRC_LEN {
                self.stats.crc_failures += 1;
                return;
            }
            let covered = &section[..section.len() - CRC_LEN];
            let declared = u32::from_be_bytes([
                section[section.len() - 4],
                section[section.len() - 3],
                section[section.len() - 2],
                section[section.len() - 1],
            ]);
            let computed = dvb_common::crc32_mpeg2::compute(covered);
            if computed != declared {
                self.stats.crc_failures += 1;
                return;
            }
        }

        // Build the gate key and change detector.
        let (ext, section_number, version, change_crc) =
            if long_form && section.len() >= MIN_SECTION_LEN + LONG_FORM_EXTRA + CRC_LEN {
                let ext = ((section[3] as u16) << 8) | section[4] as u16;
                let version = (section[5] >> 1) & 0x1F;
                let section_number = section[6];
                // For long-form the trailing CRC already uniquely fingerprints the
                // payload; reuse it as the change hash.
                let crc = u32::from_be_bytes([
                    section[section.len() - 4],
                    section[section.len() - 3],
                    section[section.len() - 2],
                    section[section.len() - 1],
                ]);
                (ext, section_number, version, crc)
            } else {
                // Short-form (incl. TOT and any malformed long-form that slipped
                // the size check above): no version, ext/section_number = 0,
                // change detector is a CRC over all the section bytes.
                (0u16, 0u8, 0u8, dvb_common::crc32_mpeg2::compute(&section))
            };

        let key = (pid.value() as u64)
            | ((table_id as u64) << 13)
            | ((ext as u64) << 21)
            | ((section_number as u64) << 37);

        let entry = GateEntry {
            version,
            crc: change_crc,
        };

        let changed = match self.gate.get(&key) {
            Some(prev) => *prev != entry,
            None => true,
        };

        // Update the gate (FIFO-evict at capacity for newly-seen keys).
        let is_new = !self.gate.contains_key(&key);
        if is_new && self.gate.len() >= self.cfg.gate_capacity {
            if let Some(old) = self.gate_order.pop_front() {
                self.gate.remove(&old);
                self.stats.gate_evictions += 1;
            }
        }
        if is_new {
            self.gate_order.push_back(key);
        }
        match self.gate.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut oe) => {
                *oe.get_mut() = entry;
            }
            std::collections::hash_map::Entry::Vacant(ve) => {
                ve.insert(entry);
            }
        }

        if changed || self.cfg.emit_repeats {
            let event = SectionEvent {
                pid,
                bytes: section,
            };
            // PAT-follow happens on an emitted (changed) PAT only.
            if self.cfg.follow_pat && changed && table_id == PAT_TABLE_ID {
                self.follow_pat(&event);
            }
            self.stats.emitted += 1;
            self.scratch.push(event);
        } else {
            self.stats.suppressed += 1;
        }
    }

    /// Parse the PAT and register each programme's PMT PID with a fresh
    /// reassembler. Parse failures are silently ignored — a malformed PAT that
    /// nonetheless passed CRC is implausible, and we never panic.
    fn follow_pat(&mut self, event: &SectionEvent) {
        use crate::tables::pat::PatSection;
        use dvb_common::Parse;
        if let Ok(pat) = PatSection::parse(&event.bytes) {
            for entry in &pat.entries {
                if entry.program_number != 0 {
                    self.pids.entry(Pid::new(entry.pid)).or_default();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ts::{TsHeader, TS_PACKET_SIZE};

    /// Wrap section bytes in a single PUSI TS packet on `pid`, with a
    /// pointer_field of 0 and 0xFF stuffing tail. Section must fit one packet.
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
        let start = 5;
        assert!(start + section.len() <= TS_PACKET_SIZE, "section too big");
        pkt[start..start + section.len()].copy_from_slice(section);
        pkt
    }

    /// Build a long-form section with a correct trailing CRC-32.
    fn long_section(
        table_id: u8,
        ext: u16,
        version: u8,
        section_number: u8,
        payload: &[u8],
    ) -> Vec<u8> {
        let section_length = (LONG_FORM_EXTRA + payload.len() + CRC_LEN) as u16;
        let mut v = vec![
            table_id,
            0x80 | 0x30 | ((section_length >> 8) as u8 & 0x0F),
            (section_length & 0xFF) as u8,
            (ext >> 8) as u8,
            (ext & 0xFF) as u8,
            0xC0 | ((version & 0x1F) << 1) | 0x01,
            section_number,
            section_number, // last_section_number
        ];
        v.extend_from_slice(payload);
        let crc = dvb_common::crc32_mpeg2::compute(&v);
        v.extend_from_slice(&crc.to_be_bytes());
        v
    }

    /// Build a PAT section (real CRC) mapping (program_number, pmt_pid) pairs.
    fn pat_section(tsid: u16, version: u8, entries: &[(u16, u16)]) -> Vec<u8> {
        let mut body = Vec::new();
        for &(pn, pid) in entries {
            body.extend_from_slice(&pn.to_be_bytes());
            body.push(0xE0 | ((pid >> 8) as u8 & 0x1F));
            body.push((pid & 0xFF) as u8);
        }
        long_section(0x00, tsid, version, 0, &body)
    }

    /// Build a PMT section (real CRC). One stream entry.
    fn pmt_section(program_number: u16, version: u8, pcr_pid: u16) -> Vec<u8> {
        // pcr_pid(2) + program_info_length(2)=0 + one stream(5):
        // stream type 0x02 (video), elementary_pid = pcr_pid+1, es_info_len 0.
        let body = [
            0xE0 | ((pcr_pid >> 8) as u8 & 0x1F),
            (pcr_pid & 0xFF) as u8,
            0xF0,
            0x00,
            0x02,
            0xE0 | (((pcr_pid + 1) >> 8) as u8 & 0x1F),
            ((pcr_pid + 1) & 0xFF) as u8,
            0xF0,
            0x00,
        ];
        long_section(0x02, program_number, version, 0, &body)
    }

    #[test]
    fn pat_emits_once_suppresses_repeat_reemits_on_version_change() {
        let mut demux = SiDemux::builder().build();

        let pat_v0 = pat_section(0x0001, 0, &[(1, 0x0100)]);
        let pat_v1 = pat_section(0x0001, 1, &[(1, 0x0100)]);

        let pkt_v0 = ts_packet(0x0000, &pat_v0);
        let pkt_v1 = ts_packet(0x0000, &pat_v1);

        let n0: Vec<_> = demux.feed(&pkt_v0).collect();
        assert_eq!(n0.len(), 1, "PAT v0 should emit one event");
        assert_eq!(n0[0].table_id(), 0x00);
        assert_eq!(n0[0].version(), Some(0));

        let n1: Vec<_> = demux.feed(&pkt_v0).collect();
        assert_eq!(n1.len(), 0, "repeat PAT should be suppressed");

        let n2: Vec<_> = demux.feed(&pkt_v1).collect();
        assert_eq!(n2.len(), 1, "PAT v1 should re-emit");
        assert_eq!(n2[0].version(), Some(1));

        let s = demux.stats();
        assert_eq!(s.sections_completed, 3);
        assert_eq!(s.emitted, 2);
        assert_eq!(s.suppressed, 1);
        assert_eq!(s.crc_failures, 0);
    }

    #[test]
    fn follow_pat_registers_pmt_pid_and_emits_typed_pmt() {
        use crate::tables::AnyTableSection;
        let mut demux = SiDemux::builder().build();

        // PAT maps programme 1 -> PMT on PID 0x0100.
        let pat = pat_section(0x0001, 0, &[(1, 0x0100)]);
        let pat_evts: Vec<_> = demux.feed(&ts_packet(0x0000, &pat)).collect();
        assert_eq!(pat_evts.len(), 1);

        // Before follow, a PMT packet on 0x0100 would be ignored. After the
        // PAT was emitted, 0x0100 is watched.
        let pmt = pmt_section(1, 0, 0x0100);
        let pmt_evts: Vec<_> = demux.feed(&ts_packet(0x0100, &pmt)).collect();
        assert_eq!(pmt_evts.len(), 1, "PMT on the followed PID should emit");
        assert_eq!(pmt_evts[0].pid(), Pid::new(0x0100));
        match pmt_evts[0].table_section().unwrap() {
            AnyTableSection::PmtSection(p) => assert_eq!(p.program_number, 1),
            other => panic!("expected PmtSection, got {other:?}"),
        }
    }

    #[test]
    fn corrupted_crc_sdt_dropped_and_counted() {
        let mut demux = SiDemux::builder().build();
        // SDT actual = table_id 0x42, carried on SDT_BAT pid 0x0011.
        let mut sdt = long_section(0x42, 0x0001, 0, 0, &[0xDE, 0xAD, 0xBE, 0xEF]);
        // Corrupt a payload byte AFTER the CRC was computed.
        sdt[8] ^= 0xFF;
        let evts: Vec<_> = demux.feed(&ts_packet(0x0011, &sdt)).collect();
        assert_eq!(evts.len(), 0, "corrupted SDT must not emit");
        let s = demux.stats();
        assert_eq!(s.crc_failures, 1);
        assert_eq!(s.emitted, 0);
        assert_eq!(s.sections_completed, 1);
    }

    #[test]
    fn gate_capacity_evicts_fifo_and_reemits() {
        let mut demux = SiDemux::builder().gate_capacity(2).build();

        // Three distinct EIT sections (table_id 0x4E) by table_id_extension,
        // all on the EIT pid 0x0012.
        let a = long_section(0x4E, 0x0001, 0, 0, &[0x01]);
        let b = long_section(0x4E, 0x0002, 0, 0, &[0x02]);
        let c = long_section(0x4E, 0x0003, 0, 0, &[0x03]);

        assert_eq!(demux.feed(&ts_packet(0x0012, &a)).count(), 1);
        assert_eq!(demux.feed(&ts_packet(0x0012, &b)).count(), 1);
        // Inserting c evicts a (the oldest).
        assert_eq!(demux.feed(&ts_packet(0x0012, &c)).count(), 1);
        assert_eq!(demux.stats().gate_evictions, 1);

        // a was evicted -> re-feeding it re-emits (treated as newly seen).
        assert_eq!(demux.feed(&ts_packet(0x0012, &a)).count(), 1);
    }

    #[test]
    fn garbage_packet_counted_no_panic() {
        let mut demux = SiDemux::builder().build();
        let garbage = [0x00u8; TS_PACKET_SIZE]; // bad sync byte
        let evts: Vec<_> = demux.feed(&garbage).collect();
        assert_eq!(evts.len(), 0);
        assert_eq!(demux.stats().malformed_packets, 1);
        assert_eq!(demux.stats().packets, 1);
    }

    #[test]
    fn emit_repeats_bypasses_suppression() {
        let mut demux = SiDemux::builder().emit_repeats(true).build();
        let pat = pat_section(0x0001, 0, &[(1, 0x0100)]);
        let pkt = ts_packet(0x0000, &pat);
        assert_eq!(demux.feed(&pkt).count(), 1);
        assert_eq!(demux.feed(&pkt).count(), 1, "emit_repeats re-emits");
        assert_eq!(demux.stats().suppressed, 0);
        assert_eq!(demux.stats().emitted, 2);
    }

    #[test]
    fn table_section_with_empty_registry_matches_table_section() {
        use crate::tables::registry::TableRegistry;
        use crate::tables::AnyTableSection;

        let mut demux = SiDemux::builder().build();
        let pat = pat_section(0x0001, 0, &[(1, 0x0100)]);
        let evts: Vec<_> = demux.feed(&ts_packet(0x0000, &pat)).collect();
        assert_eq!(evts.len(), 1);

        let reg = TableRegistry::new();
        let with_reg = evts[0].table_section_with(&reg).unwrap();
        let without = evts[0].table_section().unwrap();
        assert!(matches!(with_reg, AnyTableSection::PatSection(_)));
        assert!(matches!(without, AnyTableSection::PatSection(_)));
    }

    #[test]
    fn table_section_with_custom_registry_yields_other() {
        use crate::tables::registry::TableRegistry;
        use crate::tables::AnyTableSection;
        use crate::traits::TableDef;
        use dvb_common::Parse;

        const PRIVATE_TID: u8 = 0x90;

        #[derive(Debug)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        struct PrivateTable {
            table_id: u8,
        }

        impl<'a> Parse<'a> for PrivateTable {
            type Error = crate::Error;
            fn parse(bytes: &'a [u8]) -> crate::Result<Self> {
                if bytes.is_empty() {
                    return Err(crate::Error::BufferTooShort {
                        need: 1,
                        have: 0,
                        what: "PrivateTable",
                    });
                }
                Ok(Self { table_id: bytes[0] })
            }
        }

        impl<'a> TableDef<'a> for PrivateTable {
            const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(PRIVATE_TID, PRIVATE_TID)];
            const NAME: &'static str = "PRIVATE_TABLE";
        }

        let mut reg = TableRegistry::new();
        reg.register::<PrivateTable>();

        let mut demux = SiDemux::builder()
            .dvb_si_pids(false)
            .pid(Pid::new(0x0200))
            .build();

        let section = long_section(PRIVATE_TID, 0x0001, 0, 0, &[0x42]);
        let evts: Vec<_> = demux.feed(&ts_packet(0x0200, &section)).collect();
        assert_eq!(evts.len(), 1);

        let result = evts[0].table_section_with(&reg).unwrap();
        match result {
            AnyTableSection::Other {
                table_id,
                ref value,
            } => {
                assert_eq!(table_id, PRIVATE_TID);
                let pt = value.downcast_ref::<PrivateTable>().unwrap();
                assert_eq!(pt.table_id, PRIVATE_TID);
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }
}
