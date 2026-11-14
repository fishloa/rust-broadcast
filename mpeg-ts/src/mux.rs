//! Section → TS packetiser (the byte-exact inverse of
//! [`SectionReassembler::feed`](crate::ts::SectionReassembler::feed)).
//!
//! Per the PSI carriage rules of ITU-T H.222.0 §2.4.4 (= ISO/IEC 13818-1:2007 §2.4.4)
//! (`private/specs-verified/mpeg/mpeg-ts/tables/h222_0/psi_section.md`): sections are packed into 188-byte packets
//! with a `pointer_field` where sections begin, concatenated contiguously, and
//! 0xFF-stuffed at the batch tail.

use alloc::vec::Vec;
use core::time::Duration;

use broadcast_common::Parse;
use broadcast_common::stage::Timestamp;

use crate::pid::{Pid, well_known};
use crate::section::Section;
use crate::ts::{
    AdaptationFieldControl, ContinuityCounter, ScramblingControl, TS_PACKET_SIZE, TsHeader,
};

/// `table_id` of the PAT (ISO/IEC 13818-1 §2.4.4.3).
pub const TABLE_ID_PAT: u8 = 0x00;
/// `table_id` of the PMT (ISO/IEC 13818-1 §2.4.4.8).
pub const TABLE_ID_PMT: u8 = 0x02;
/// `table_id` of the NIT actual (ETSI EN 300 468 §5.2.1).
pub const TABLE_ID_NIT_ACTUAL: u8 = 0x40;
/// `table_id` of the SDT actual (ETSI EN 300 468 §5.2.3).
pub const TABLE_ID_SDT_ACTUAL: u8 = 0x42;
/// `table_id` of the TDT (ETSI EN 300 468 §5.2.5).
pub const TABLE_ID_TDT: u8 = 0x70;
/// `table_id` of the TOT (ETSI EN 300 468 §5.2.6).
pub const TABLE_ID_TOT: u8 = 0x73;

/// Maximum data bytes in a PUSI=1 packet (188 − 4 header − 1 pointer_field). §2.4.4.
const PUSI_PAYLOAD_CAP: usize = 183;
/// Maximum data bytes in a continuation packet (188 − 4 header). §2.4.4.
const PAYLOAD_CAP: usize = 184;
use crate::ts::STUFFING_BYTE;

/// Packetises PSI/SI sections into 188-byte TS packets.
///
/// This is the byte-exact inverse of
/// [`SectionReassembler::feed`](crate::ts::SectionReassembler::feed): packets
/// produced here, when fed back through the reassembler, yield the same
/// sections in order.
///
/// ISO/IEC 13818-1:2007 §2.4.4 (`private/specs-verified/mpeg/mpeg-ts/tables/h222_0/psi_section.md`).
pub struct SectionPacketiser {
    pid: Pid,
    continuity_counter: ContinuityCounter,
}

impl SectionPacketiser {
    /// Start a packetiser for `pid` with continuity_counter = 0.
    pub fn new(pid: Pid) -> Self {
        Self {
            pid,
            continuity_counter: ContinuityCounter::ZERO,
        }
    }

    /// Start at a specific continuity_counter — for resuming a stream.
    pub fn with_continuity(pid: Pid, cc: ContinuityCounter) -> Self {
        Self {
            pid,
            continuity_counter: cc,
        }
    }

    /// The PID this packetiser emits packets for.
    pub fn pid(&self) -> Pid {
        self.pid
    }

    /// The continuity_counter for the next emitted packet.
    pub fn continuity_counter(&self) -> ContinuityCounter {
        self.continuity_counter
    }

    /// Packetise a batch of complete sections into 188-byte TS packets,
    /// appended to `out` (cleared first).
    ///
    /// Returns the number of packets appended.
    pub fn packetise_into(
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
        // `starts` is ascending and `pos` only advances, so a forward-only
        // cursor finds the smallest section-start offset >= pos in amortised
        // O(1) per packet instead of rescanning from the front (audit
        // r01-O1: O(packets x sections)).
        let mut next_idx = 0usize;

        while pos < data.len() {
            while starts.get(next_idx).is_some_and(|&s| s < pos) {
                next_idx += 1;
            }
            let next_start = starts.get(next_idx).copied();

            let pusi: bool;
            let pointer_field: u8;
            let cap: usize;

            if let Some(ns) = next_start {
                let diff = ns.saturating_sub(pos);
                // W1 (#1074): `diff == PUSI_PAYLOAD_CAP` (183) must NOT take the
                // PUSI branch — a pointer_field of 183 would claim a section
                // starts after all 183 payload bytes, i.e. not within this
                // packet at all (H.222.0 §2.4.4 requires PUSI=1 to mean a
                // section genuinely starts here). The non-PUSI branch's cap
                // must in turn be bounded by `diff`, not just the fixed
                // 184-byte payload size, or a continuation packet at this same
                // boundary copies one byte past the next section's start —
                // the reassembler then silently drops it (it only takes what
                // completes the in-progress section), corrupting the next one.
                if diff < PUSI_PAYLOAD_CAP {
                    pusi = true;
                    pointer_field = diff as u8;
                    cap = PUSI_PAYLOAD_CAP;
                } else {
                    pusi = false;
                    pointer_field = 0;
                    cap = PAYLOAD_CAP.min(diff);
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
                scrambling: ScramblingControl::NotScrambled,
                adaptation_field_control: AdaptationFieldControl::PayloadOnly,
                continuity_counter: self.continuity_counter,
            };
            header
                .serialize_into(&mut pkt[..4])
                .expect("4-byte header buffer");

            self.continuity_counter = self.continuity_counter.next();

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

    /// Allocating convenience wrapper over [`packetise_into`](Self::packetise_into).
    pub fn packetise(&mut self, sections: &[&[u8]]) -> Vec<[u8; TS_PACKET_SIZE]> {
        let mut out = Vec::new();
        self.packetise_into(sections, &mut out);
        out
    }
}

// ── Default interval constants ────────────────────────────────────────────────

/// private/specs-verified/dvb/dvb-si/text/tr_101_211/README.md §4.4.1/§4.4.2 — NIT maximum interval.
pub const NIT_MAX_INTERVAL: SectionInterval = SectionInterval::from_const(Duration::from_secs(10));
/// private/specs-verified/dvb/dvb-si/text/tr_101_211/README.md §4.4.1/§4.4.2 — BAT maximum interval.
pub const BAT_MAX_INTERVAL: SectionInterval = SectionInterval::from_const(Duration::from_secs(10));
/// private/specs-verified/dvb/dvb-si/text/tr_101_211/README.md §4.4.1/§4.4.2 — SDT actual maximum interval.
pub const SDT_ACTUAL_MAX_INTERVAL: SectionInterval =
    SectionInterval::from_const(Duration::from_secs(2));
/// private/specs-verified/dvb/dvb-si/text/tr_101_211/README.md §4.4.1/§4.4.2 — SDT other maximum interval.
pub const SDT_OTHER_MAX_INTERVAL: SectionInterval =
    SectionInterval::from_const(Duration::from_secs(10));
/// private/specs-verified/dvb/dvb-si/text/tr_101_211/README.md §4.4.1/§4.4.2 — EIT present/following actual maximum interval.
pub const EIT_PF_ACTUAL_MAX_INTERVAL: SectionInterval =
    SectionInterval::from_const(Duration::from_secs(2));
/// private/specs-verified/dvb/dvb-si/text/tr_101_211/README.md §4.4.1 — EIT present/following other maximum interval
/// (satellite and cable; §4.4.2 gives 20 s for terrestrial).
pub const EIT_PF_OTHER_MAX_INTERVAL: SectionInterval =
    SectionInterval::from_const(Duration::from_secs(10));
/// private/specs-verified/dvb/dvb-si/text/tr_101_211/README.md §4.4.1 — EIT schedule (first 8 days) maximum interval.
pub const EIT_SCHED_MAX_INTERVAL: SectionInterval =
    SectionInterval::from_const(Duration::from_secs(10));
/// private/specs-verified/dvb/dvb-si/text/tr_101_211/README.md §4.4.1 — EIT schedule (further than 8 days ahead) maximum interval.
pub const EIT_SCHED_EXT_MAX_INTERVAL: SectionInterval =
    SectionInterval::from_const(Duration::from_secs(30));
/// private/specs-verified/dvb/dvb-si/text/tr_101_211/README.md §4.4.1/§4.4.2 — TDT maximum interval.
pub const TDT_MAX_INTERVAL: SectionInterval = SectionInterval::from_const(Duration::from_secs(30));
/// private/specs-verified/dvb/dvb-si/text/tr_101_211/README.md §4.4.1/§4.4.2 — TOT maximum interval.
pub const TOT_MAX_INTERVAL: SectionInterval = SectionInterval::from_const(Duration::from_secs(30));

/// The inter-section repetition interval of one table (EN 300 468 §5.1.4.1):
/// a [`Duration`] that is never below [`MIN_SECTION_INTERVAL`].
///
/// The 25 ms floor used to be a `debug_assert!` inside [`SiMux::upsert`], so a
/// release build accepted `Duration::ZERO` (one section every poll) silently.
/// The bound is a property of the interval itself, so it is checked here, once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct SectionInterval(Duration);

/// A repetition interval below the EN 300 468 §5.1.4.1 floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SectionIntervalTooShort {
    /// The rejected interval.
    pub interval: Duration,
    /// The floor it must reach ([`MIN_SECTION_INTERVAL`]).
    pub min: Duration,
}

impl core::fmt::Display for SectionIntervalTooShort {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "section interval {:?} is below the {:?} minimum (EN 300 468 §5.1.4.1)",
            self.interval, self.min
        )
    }
}

impl core::error::Error for SectionIntervalTooShort {}

impl SectionInterval {
    /// Validate `interval` against the EN 300 468 §5.1.4.1 25 ms floor.
    ///
    /// # Errors
    /// [`SectionIntervalTooShort`] when `interval < MIN_SECTION_INTERVAL`.
    pub const fn try_new(
        interval: Duration,
    ) -> core::result::Result<Self, SectionIntervalTooShort> {
        // A `Duration` comparison is not usable in a `const fn`; compare nanos.
        if interval.as_nanos() < MIN_SECTION_INTERVAL_NANOS {
            Err(SectionIntervalTooShort {
                interval,
                min: MIN_SECTION_INTERVAL,
            })
        } else {
            Ok(Self(interval))
        }
    }

    /// Validate at compile time, for the spec's own named intervals
    /// ([`PAT_MAX_INTERVAL`] and friends): a mistyped interval is then a build
    /// error rather than a run-time `try_new`.
    ///
    /// # Panics
    /// In a `const` context, when `interval < MIN_SECTION_INTERVAL`.
    #[must_use]
    pub const fn from_const(interval: Duration) -> Self {
        assert!(
            interval.as_nanos() >= MIN_SECTION_INTERVAL_NANOS,
            "named section interval is below the 25 ms minimum (EN 300 468 §5.1.4.1)"
        );
        Self(interval)
    }

    /// The interval.
    #[must_use]
    pub const fn duration(self) -> Duration {
        self.0
    }
}

impl TryFrom<Duration> for SectionInterval {
    type Error = SectionIntervalTooShort;
    fn try_from(interval: Duration) -> core::result::Result<Self, Self::Error> {
        Self::try_new(interval)
    }
}

impl From<SectionInterval> for Duration {
    fn from(interval: SectionInterval) -> Self {
        interval.0
    }
}

/// private/specs-verified/dvb/dvb-si/tables/en_300_468/514-repetition-rates-and-random-access-hand-transcribed.md
/// §5.1.4.1 — minimum inter-section interval floor (≤100 Mbit/s TSs).
pub const MIN_SECTION_INTERVAL: Duration = Duration::from_millis(25);
/// [`MIN_SECTION_INTERVAL`] in nanoseconds, for the `const` comparisons.
const MIN_SECTION_INTERVAL_NANOS: u128 = 25_000_000;

/// private/specs-verified/dvb/dvb-si/ §4.1.7 — PAT maximum interval (shall ≤ 100 ms).
pub const PAT_MAX_INTERVAL: SectionInterval =
    SectionInterval::from_const(Duration::from_millis(100));
/// private/specs-verified/dvb/dvb-si/ §4.1.7 — PMT maximum interval (shall ≤ 100 ms).
pub const PMT_MAX_INTERVAL: SectionInterval =
    SectionInterval::from_const(Duration::from_millis(100));

// ── SiMux ────────────────────────────────────────────────────────────────────

/// The key one scheduled section is stored under: the PID it is carried on
/// and its identity inside that PID — `table_id` plus `section_number`
/// (ISO/IEC 13818-1 §2.4.4.4 / EN 300 468 §5.2). Several distinct tables
/// share a PID by spec (TDT/TOT both on `0x0014`, SDT/BAT both on `0x0011`,
/// EIT present/following vs. schedule sub-tables on `0x0012`), and a long
/// table spans several sections told apart by `section_number`, so neither
/// PID alone nor PID plus `table_id` names a section.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SectionKey {
    /// The PID the section is carried on.
    pub pid: Pid,
    /// `table_id` (ISO/IEC 13818-1 §2.4.4.1).
    pub table_id: u8,
    /// `section_number`: the section's position in its table.
    pub section_number: u8,
}

/// A section [`SiMux::upsert`] could not schedule, kept so the caller can
/// see what was refused rather than having it dropped without a word.
///
/// The section itself is not retained: the bytes are the caller's, and a
/// rejection names only where they were registered and why they cannot be
/// emitted.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RejectedSection {
    /// The PID the section was registered on.
    pub pid: Pid,
    /// Why it was rejected.
    pub reason: SectionRejection,
}

/// Why [`SiMux::upsert`] could not put a section into the emission
/// schedule.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum SectionRejection {
    /// The input held no complete section (an empty slice, or a header that
    /// declares more bytes than were given), so there is nothing to parse.
    /// An empty input used to register a whole `(pid, table_id 0)` entry and
    /// emit the PAT interval's worth of nothing.
    NotASection,
    /// Two or more sections were given for one PID. A `SectionKey` names one
    /// section, so a batch is ambiguous; register each on its own call.
    MoreThanOneSection,
    /// Several sections on one PID carried the same `(table_id,
    /// section_number)`: the second would silently replace the first.
    DuplicateSection,
    /// The section bytes did not parse as a section header.
    Malformed,
    /// A convenience registration received a section of another table: a
    /// `table_id` other than the one that registration is for.
    WrongTableId {
        /// The `table_id` the registration is for.
        expected: u8,
        /// The `table_id` the section carries, if its header fit.
        found: Option<u8>,
    },
}

/// Section-repetition scheduler that builds TS packets on a caller-supplied clock.
///
/// Each entry is one complete [section](crate::section::Section) — its
/// `table_id` and `section_number` identify it, its PID + interval say where
/// and how often it goes out. Call [`poll_into`](Self::poll_into) with
/// monotonically-increasing `now` values to get 188-byte TS packets for every
/// entry whose interval has elapsed since its last emission.
///
/// Entries are keyed by [`SectionKey`]: `(pid, table_id, section_number)`.
/// A section is given as a single `Section`: the length and CRC are derived
/// from the section's own fields when it is packetised, so a caller cannot
/// hand the scheduler bytes that are not a section, sections of mixed tables,
/// or a trailing truncated section that used to be dropped without a word.
/// Input that cannot be scheduled is kept and reported by
/// [`rejected`](Self::rejected) ([`RejectedSection`]).
///
/// All entries on the same PID share one [`SectionPacketiser`] (and so one
/// continuity counter), created lazily the first time that PID emits — this
/// is what keeps the continuity counter continuous per PID across table
/// kinds, not just within one kind.
///
/// # 25 ms floor
///
/// [`MIN_SECTION_INTERVAL`] is the minimum valid interval (EN 300 468 §5.1.4.1),
/// enforced by [`SectionInterval::try_new`]: an interval below it cannot reach
/// [`upsert`](Self::upsert) at all.
pub struct SiMux {
    entries: Vec<Entry>,
    rejected: Vec<RejectedSection>,
    /// One packetiser per distinct PID, shared across every table_id on it.
    packetisers: Vec<SectionPacketiser>,
}

struct Entry {
    key: SectionKey,
    section: Section<'static>,
    interval: SectionInterval,
    last_emit: Option<Timestamp>,
}

/// Find the packetiser for `pid`, creating one (fresh continuity counter) if
/// this is the first entry ever registered on that PID. A free function, not
/// a method, so callers can hold `&mut entries` and this borrow concurrently.
fn packetiser_for(packetisers: &mut Vec<SectionPacketiser>, pid: Pid) -> &mut SectionPacketiser {
    if let Some(idx) = packetisers.iter().position(|p| p.pid() == pid) {
        &mut packetisers[idx]
    } else {
        packetisers.push(SectionPacketiser::new(pid));
        packetisers.last_mut().expect("just pushed")
    }
}

impl SiMux {
    /// Create an empty scheduler (no entries, no pending emissions).
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            rejected: Vec::new(),
            packetisers: Vec::new(),
        }
    }

    /// Register or replace one section carried on `pid` at `interval` (a
    /// validated [`SectionInterval`]: EN 300 468 §5.1.4.1's 25 ms floor).
    ///
    /// The section is stored under its own [`SectionKey`], so re-calling for
    /// the same `(pid, table_id, section_number)` updates that one section
    /// and its interval while preserving the PID's shared continuity counter;
    /// any other section becomes a second, independent entry.
    ///
    /// # Errors
    ///
    /// Returns the [`SectionRejection`] for a section that cannot be
    /// scheduled: [`NotASection`](SectionRejection::NotASection) (the input
    /// held no complete section, so the key would not be its own),
    /// [`MoreThanOneSection`](SectionRejection::MoreThanOneSection) (a
    /// [`SectionKey`] names one section, so a batch is ambiguous),
    /// [`DuplicateSection`](SectionRejection::DuplicateSection) or
    /// [`Malformed`](SectionRejection::Malformed) or
    /// [`WrongTableId`](SectionRejection::WrongTableId). Every rejection is
    /// kept in [`rejected`](Self::rejected).
    pub fn upsert(
        &mut self,
        pid: Pid,
        section: Vec<u8>,
        interval: SectionInterval,
    ) -> core::result::Result<(), SectionRejection> {
        let rejection = self.register(pid, section, interval);
        match rejection {
            Some(reason) => {
                self.rejected.push(RejectedSection { pid, reason });
                Err(reason)
            }
            None => Ok(()),
        }
    }

    /// Register one complete section, replacing the entry already stored
    /// under its key.
    fn register(
        &mut self,
        pid: Pid,
        section: Vec<u8>,
        interval: SectionInterval,
    ) -> Option<SectionRejection> {
        let batch = split_sections(&section);
        let owned = match batch.len() {
            0 => return Some(SectionRejection::NotASection),
            1 => match Section::parse(batch[0]) {
                Ok(parsed) => parsed,
                Err(_) => return Some(SectionRejection::Malformed),
            },
            _ => {
                if !sections_are_distinct(&batch) {
                    return Some(SectionRejection::DuplicateSection);
                }
                return Some(SectionRejection::MoreThanOneSection);
            }
        };
        let key = SectionKey {
            pid,
            table_id: owned.table_id,
            section_number: owned.section_number,
        };
        let stored = Section::into_owned(owned);
        if let Some(entry) = self.entries.iter_mut().find(|e| e.key == key) {
            entry.section = stored;
            entry.interval = interval;
        } else {
            self.entries.push(Entry {
                key,
                section: stored,
                interval,
                last_emit: None,
            });
        }
        None
    }

    /// The sections that could not be scheduled, with why, in the order they
    /// were refused. Never silently dropped: read it after
    /// [`upsert`](Self::upsert) to surface a bad input instead of trusting the
    /// scheduler to have taken everything.
    #[must_use]
    pub fn rejected(&self) -> &[RejectedSection] {
        &self.rejected
    }

    /// PAT at [`PAT_MAX_INTERVAL`] on PID 0x0000.
    ///
    /// # Errors
    ///
    /// As [`upsert`](Self::upsert); only a PAT section is accepted.
    pub fn upsert_pat(&mut self, section: Vec<u8>) -> core::result::Result<(), SectionRejection> {
        upsert_checked(
            self,
            well_known::PAT,
            section,
            PAT_MAX_INTERVAL,
            TABLE_ID_PAT,
        )
    }

    /// PMT at [`PMT_MAX_INTERVAL`] on the caller-supplied `pid`.
    ///
    /// Interval cites private/specs-verified/dvb/dvb-si/ §4.1.7.
    ///
    /// # Errors
    ///
    /// As [`upsert`](Self::upsert); the PAT interval used to apply to whatever
    /// was stored under `table_id 0`, so only a PMT section is accepted.
    pub fn upsert_pmt(
        &mut self,
        pid: Pid,
        section: Vec<u8>,
    ) -> core::result::Result<(), SectionRejection> {
        upsert_checked(self, pid, section, PMT_MAX_INTERVAL, TABLE_ID_PMT)
    }

    /// SDT actual at [`SDT_ACTUAL_MAX_INTERVAL`] on PID 0x0011.
    ///
    /// Interval cites private/specs-verified/dvb/dvb-si/text/tr_101_211/README.md §4.4.1/§4.4.2.
    ///
    /// # Errors
    ///
    /// As [`upsert`](Self::upsert); only an SDT actual section is accepted.
    pub fn upsert_sdt_actual(
        &mut self,
        section: Vec<u8>,
    ) -> core::result::Result<(), SectionRejection> {
        upsert_checked(
            self,
            well_known::SDT_BAT,
            section,
            SDT_ACTUAL_MAX_INTERVAL,
            TABLE_ID_SDT_ACTUAL,
        )
    }

    /// NIT at [`NIT_MAX_INTERVAL`] on PID 0x0010.
    ///
    /// # Errors
    ///
    /// As [`upsert`](Self::upsert); only a NIT section is accepted.
    pub fn upsert_nit(&mut self, section: Vec<u8>) -> core::result::Result<(), SectionRejection> {
        upsert_checked(
            self,
            well_known::NIT,
            section,
            NIT_MAX_INTERVAL,
            TABLE_ID_NIT_ACTUAL,
        )
    }

    /// TDT at [`TDT_MAX_INTERVAL`] on PID 0x0014.
    ///
    /// Interval cites private/specs-verified/dvb/dvb-si/text/tr_101_211/README.md §4.4.1/§4.4.2.
    ///
    /// # Errors
    ///
    /// As [`upsert`](Self::upsert); only a TDT section is accepted.
    pub fn upsert_tdt(&mut self, section: Vec<u8>) -> core::result::Result<(), SectionRejection> {
        upsert_checked(
            self,
            well_known::TDT_TOT,
            section,
            TDT_MAX_INTERVAL,
            TABLE_ID_TDT,
        )
    }

    /// TOT at [`TOT_MAX_INTERVAL`] on PID 0x0014.
    ///
    /// Interval cites private/specs-verified/dvb/dvb-si/text/tr_101_211/README.md §4.4.1/§4.4.2.
    ///
    /// # Errors
    ///
    /// As [`upsert`](Self::upsert); only a TOT section is accepted.
    pub fn upsert_tot(&mut self, section: Vec<u8>) -> core::result::Result<(), SectionRejection> {
        upsert_checked(
            self,
            well_known::TDT_TOT,
            section,
            TOT_MAX_INTERVAL,
            TABLE_ID_TOT,
        )
    }

    /// Emit every entry due at `now` (i.e. `now - last_emit >= interval`, and
    /// first call always due), packetising via [`SectionPacketiser`], appended
    /// to `out` (cleared first).
    ///
    /// Updates each emitted entry's `last_emit = now`. Deterministic given the
    /// fed `now` sequence. Returns the packet count appended.
    pub fn poll_into(&mut self, now: Timestamp, out: &mut Vec<[u8; TS_PACKET_SIZE]>) -> usize {
        out.clear();
        let before = out.len();

        let packetisers = &mut self.packetisers;
        for entry in &mut self.entries {
            let due = match entry.last_emit {
                None => true,
                Some(last) => now.saturating_sub(last) >= entry.interval.duration(),
            };
            if due {
                let mut tmp = Vec::new();
                if let Ok(bytes) = entry.section.serialize() {
                    let refs: [&[u8]; 1] = [&bytes];
                    let packetiser = packetiser_for(packetisers, entry.key.pid);
                    packetiser.packetise_into(&refs, &mut tmp);
                }
                out.append(&mut tmp);
                entry.last_emit = Some(now);
            }
        }

        out.len() - before
    }

    /// Allocating convenience wrapper over [`poll_into`](Self::poll_into).
    pub fn poll(&mut self, now: Timestamp) -> Vec<[u8; TS_PACKET_SIZE]> {
        let mut out = Vec::new();
        self.poll_into(now, &mut out);
        out
    }
}

fn sections_are_distinct(sections: &[&[u8]]) -> bool {
    let mut keys: Vec<(u8, u8)> = Vec::with_capacity(sections.len());
    for bytes in sections {
        let Ok(section) = Section::parse(bytes) else {
            return true;
        };
        let key = (section.table_id, section.section_number);
        if keys.contains(&key) {
            return false;
        }
        keys.push(key);
    }
    true
}

/// The scheduler accepts only this `table_id` for a convenience registration
/// and otherwise keeps the section as rejected.
fn upsert_checked(
    mux: &mut SiMux,
    pid: Pid,
    section: Vec<u8>,
    interval: SectionInterval,
    table_id: u8,
) -> core::result::Result<(), SectionRejection> {
    if split_sections(&section)
        .first()
        .is_some_and(|s| s.first().copied() != Some(table_id))
    {
        let reason = SectionRejection::WrongTableId {
            expected: table_id,
            found: section.first().copied(),
        };
        mux.rejected.push(RejectedSection { pid, reason });
        return Err(reason);
    }
    mux.upsert(pid, section, interval)
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
    while let Some(end) = crate::ts::section_total_len(&data[pos..]).map(|len| pos + len) {
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
    use alloc::vec;
    use alloc::vec::Vec;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn concat_sections(sections: &[Vec<u8>]) -> Vec<u8> {
        let total: usize = sections.iter().map(|s| s.len()).sum();
        let mut out = Vec::with_capacity(total);
        for s in sections {
            out.extend_from_slice(s);
        }
        out
    }

    /// Round-trip `sections` through packetise → reassembler, asserting
    /// byte-identical output in order and no leftovers.
    fn assert_round_trip(sections: &[Vec<u8>]) {
        let mut packetiser = SectionPacketiser::new(Pid::from_const(0x0100));
        let refs: Vec<&[u8]> = sections.iter().map(|s| s.as_slice()).collect();
        let packets = packetiser.packetise(&refs);

        let mut reasm = SectionReassembler::default();
        for pkt_raw in &packets {
            let pkt = TsPacket::parse(pkt_raw).expect("parse generated packet");
            let payload = pkt.payload.expect("payload present");
            let pusi = pkt.header.pusi;
            reasm.feed(payload, pusi);
        }

        let got: Vec<_> = core::iter::from_fn(|| reasm.pop_section()).collect();
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

    /// 400 sections whose sizes sweep 0..=399 body bytes (so section starts
    /// land on every offset around the 183/184-byte packet boundaries)
    /// round-trip byte-identically through packetise -> reassemble (audit
    /// r01-O1: the section-start lookup is now a forward cursor; this is the
    /// independent oracle that the rewrite chose the same boundaries).
    #[test]
    fn many_sections_with_boundary_straddling_sizes_round_trip() {
        let sections: Vec<Vec<u8>> = (0..400usize)
            .map(|n| {
                let body: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
                test_fixtures::psi::short_section((n % 200) as u8, &body)
            })
            .collect();
        assert_round_trip(&sections);
    }

    // ── split_sections ──────────────────────────────────────────────────────

    #[test]
    fn split_single_section() {
        let s = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xAA; 5]);
        let refs = split_sections(&s);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0], s.as_slice());
    }

    #[test]
    fn split_two_sections() {
        let s1 = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0x01, 0x02]);
        let s2 = test_fixtures::psi::long_section(0x46, 1, 0, 0, &[0x03, 0x04, 0x05]);
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
        let s = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xAA; 3]);
        let mut data = s.clone();
        data.push(0xFF); // trailing byte that doesn't complete a header
        let refs = split_sections(&data);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0], s.as_slice());
    }

    fn cc(v: u8) -> ContinuityCounter {
        ContinuityCounter::try_new(v).expect("4-bit counter")
    }

    // ── interval constants pinned to spec ────────────────────────────────────

    #[test]
    fn interval_constants_match_spec() {
        // TR 101 211 §4.4.1/§4.4.2
        assert_eq!(NIT_MAX_INTERVAL.duration(), Duration::from_secs(10));
        assert_eq!(BAT_MAX_INTERVAL.duration(), Duration::from_secs(10));
        assert_eq!(SDT_ACTUAL_MAX_INTERVAL.duration(), Duration::from_secs(2));
        assert_eq!(SDT_OTHER_MAX_INTERVAL.duration(), Duration::from_secs(10));
        assert_eq!(
            EIT_PF_ACTUAL_MAX_INTERVAL.duration(),
            Duration::from_secs(2)
        );
        assert_eq!(
            EIT_PF_OTHER_MAX_INTERVAL.duration(),
            Duration::from_secs(10)
        );
        assert_eq!(EIT_SCHED_MAX_INTERVAL.duration(), Duration::from_secs(10));
        assert_eq!(
            EIT_SCHED_EXT_MAX_INTERVAL.duration(),
            Duration::from_secs(30)
        );
        assert_eq!(TDT_MAX_INTERVAL.duration(), Duration::from_secs(30));
        assert_eq!(TOT_MAX_INTERVAL.duration(), Duration::from_secs(30));

        // TS 101 154 §4.1.7
        assert_eq!(PAT_MAX_INTERVAL.duration(), Duration::from_millis(100));
        assert_eq!(PMT_MAX_INTERVAL.duration(), Duration::from_millis(100));

        // EN 300 468 §5.1.4.1
        assert_eq!(MIN_SECTION_INTERVAL, Duration::from_millis(25));
    }

    // ── SiMux: basic behaviour ───────────────────────────────────────────────

    #[test]
    fn new_and_default_simux_are_empty() {
        for mut mux in [SiMux::new(), SiMux::default()] {
            let pkts = mux.poll(Timestamp::ZERO);
            assert!(pkts.is_empty());
        }
    }

    #[test]
    fn first_poll_always_emits() {
        let s = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xAA; 10]);
        let mut mux = SiMux::new();
        mux.upsert(
            Pid::from_const(0x0100),
            s.clone(),
            SectionInterval::from_const(Duration::from_millis(100)),
        )
        .expect("scheduled section");
        let pkts = mux.poll(Timestamp::ZERO);
        assert!(!pkts.is_empty(), "first poll must emit");
    }

    #[test]
    fn first_poll_emits_all_entries() {
        let s1 = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0x01; 5]);
        let s2 = test_fixtures::psi::long_section(0x46, 1, 0, 0, &[0x02; 5]);
        let mut mux = SiMux::new();
        mux.upsert(
            Pid::from_const(0x0100),
            s1.clone(),
            SectionInterval::from_const(Duration::from_millis(100)),
        )
        .expect("scheduled section");
        mux.upsert(
            Pid::from_const(0x0200),
            s2.clone(),
            SectionInterval::from_const(Duration::from_millis(200)),
        )
        .expect("scheduled section");
        let pkts = mux.poll(Timestamp::ZERO);
        // Both entries should emit — we count PID occurrences
        let pids: Vec<Pid> = pkts
            .iter()
            .map(|raw| TsPacket::parse(raw).unwrap().header.pid)
            .collect();
        assert!(pids.contains(&Pid::from_const(0x0100)));
        assert!(pids.contains(&Pid::from_const(0x0200)));
    }

    // ── deterministic schedule ───────────────────────────────────────────────

    #[test]
    fn entry_emits_only_when_due() {
        let s = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xAA; 10]);
        let mut mux = SiMux::new();
        mux.upsert(
            Pid::from_const(0x0100),
            s.clone(),
            SectionInterval::from_const(Duration::from_millis(100)),
        )
        .expect("scheduled section");

        // First poll at t=0: emits
        let pkts0 = mux.poll(Timestamp::ZERO);
        assert!(!pkts0.is_empty());

        // t=50: not yet due
        let pkts50 = mux.poll(Timestamp::from_millis(50));
        assert!(pkts50.is_empty(), "should not emit at t=50");

        // t=99: still not due
        let pkts99 = mux.poll(Timestamp::from_millis(99));
        assert!(pkts99.is_empty(), "should not emit at t=99");

        // t=100: due again
        let pkts100 = mux.poll(Timestamp::from_millis(100));
        assert!(!pkts100.is_empty(), "should emit at t=100");
    }

    #[test]
    fn two_entries_different_cadence() {
        let s1 = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0x01; 5]);
        let s2 = test_fixtures::psi::long_section(0x46, 1, 0, 0, &[0x02; 5]);
        let mut mux = SiMux::new();
        mux.upsert(
            Pid::from_const(0x0100),
            concat_sections(&[s1]),
            SectionInterval::from_const(Duration::from_millis(100)),
        )
        .expect("scheduled section");
        mux.upsert(
            Pid::from_const(0x0200),
            concat_sections(&[s2]),
            SectionInterval::from_const(Duration::from_millis(200)),
        )
        .expect("scheduled section");

        // t=0: both emit
        let pkts = mux.poll(Timestamp::ZERO);
        assert!(!pkts.is_empty());

        // t=100: only 0x0100 emits (due at 100)
        let pkts100 = mux.poll(Timestamp::from_millis(100));
        assert!(!pkts100.is_empty());
        let pids100: Vec<Pid> = pkts100
            .iter()
            .map(|raw| TsPacket::parse(raw).unwrap().header.pid)
            .collect();
        assert!(
            pids100.contains(&Pid::from_const(0x0100)),
            "PID 0x0100 should emit at t=100"
        );
        assert!(
            !pids100.contains(&Pid::from_const(0x0200)),
            "PID 0x0200 should NOT emit at t=100"
        );

        // t=150: neither emits
        let pkts150 = mux.poll(Timestamp::from_millis(150));
        assert!(pkts150.is_empty(), "neither should emit at t=150");

        // t=200: both emit again
        let pkts200 = mux.poll(Timestamp::from_millis(200));
        let pids200: Vec<Pid> = pkts200
            .iter()
            .map(|raw| TsPacket::parse(raw).unwrap().header.pid)
            .collect();
        assert!(
            pids200.contains(&Pid::from_const(0x0100)),
            "PID 0x0100 should emit at t=200"
        );
        assert!(
            pids200.contains(&Pid::from_const(0x0200)),
            "PID 0x0200 should emit at t=200"
        );
    }

    #[test]
    fn entry_emits_again_after_full_interval() {
        let s = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xAA; 20]);
        let mut mux = SiMux::new();
        mux.upsert(
            Pid::from_const(0x0100),
            s,
            SectionInterval::from_const(Duration::from_secs(1)),
        )
        .expect("scheduled section");

        // t=0
        let n0 = mux.poll_into(Timestamp::ZERO, &mut Vec::new());
        assert!(n0 > 0);

        // t=999ms: not due
        let mut buf = Vec::new();
        let n999 = mux.poll_into(Timestamp::from_millis(999), &mut buf);
        assert_eq!(n999, 0);

        // t=1s: due
        let mut buf2 = Vec::new();
        let n1000 = mux.poll_into(Timestamp::from_millis(1000), &mut buf2);
        assert!(n1000 > 0);
    }

    // ── upsert replaces existing entry (same pid + table_id only) ───────────

    #[test]
    fn upsert_updates_existing_entry() {
        // Same table_id (0x42, SDT actual) both times: this is a genuine
        // update of the same logical table, so it must replace, not add.
        let s1 = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xAA; 5]);
        let s2 = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xBB; 10]);
        let mut mux = SiMux::new();

        mux.upsert(
            Pid::from_const(0x0100),
            s1,
            SectionInterval::from_const(Duration::from_millis(100)),
        )
        .expect("scheduled section");
        mux.upsert(
            Pid::from_const(0x0100),
            s2.clone(),
            SectionInterval::from_const(Duration::from_millis(200)),
        )
        .expect("scheduled section");

        let pkts = mux.poll(Timestamp::ZERO);

        // Round-trip the output to verify the updated sections
        let mut reasm = SectionReassembler::default();
        for raw in &pkts {
            let pkt = TsPacket::parse(raw).unwrap();
            if pkt.header.pid == Pid::from_const(0x0100) {
                reasm.feed(pkt.payload.unwrap(), pkt.header.pusi);
            }
        }
        let got: Vec<_> = core::iter::from_fn(|| reasm.pop_section()).collect();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].as_ref(), s2.as_slice());
    }

    // ── issue #1000: distinct table_ids sharing a PID must NOT collide ──────

    /// Regression: `upsert_tdt` then `upsert_tot` (both PID 0x0014) used to
    /// collide because entries were keyed by PID alone — the second call
    /// silently replaced the first. Both table_ids must now come out, with a
    /// continuous continuity counter across the shared PID.
    #[test]
    fn upsert_tdt_then_tot_both_emitted_with_continuous_cc() {
        let tdt = test_fixtures::psi::long_section(0x70, 1, 0, 0, &[0xAA; 5]); // TDT
        let tot = test_fixtures::psi::long_section(0x73, 1, 0, 0, &[0xBB; 8]); // TOT
        let mut mux = SiMux::new();

        mux.upsert_tdt(tdt.clone()).expect("scheduled section");
        mux.upsert_tot(tot.clone()).expect("scheduled section");

        let pkts = mux.poll(Timestamp::ZERO);
        assert!(!pkts.is_empty());

        let mut table_ids_seen = Vec::new();
        let mut reasm = SectionReassembler::default();
        let mut prev_cc: Option<ContinuityCounter> = None;
        for raw in &pkts {
            let pkt = TsPacket::parse(raw).unwrap();
            assert_eq!(pkt.header.pid, well_known::TDT_TOT);
            if let Some(prev) = prev_cc {
                assert_eq!(
                    pkt.header.continuity_counter.forward_distance_from(prev),
                    1,
                    "continuity counter must be continuous across table kinds on one PID"
                );
            }
            prev_cc = Some(pkt.header.continuity_counter);
            reasm.feed(pkt.payload.unwrap(), pkt.header.pusi);
        }
        while let Some(section) = reasm.pop_section() {
            table_ids_seen.push(section.as_ref()[0]);
        }

        assert!(table_ids_seen.contains(&0x70), "TDT missing from output");
        assert!(table_ids_seen.contains(&0x73), "TOT missing from output");
    }

    /// Same collision class for SDT actual (0x42) vs. BAT (0x4A), both on
    /// PID 0x0011.
    #[test]
    fn upsert_sdt_and_bat_both_emitted() {
        let sdt = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xCC; 5]);
        let bat = test_fixtures::psi::long_section(0x4A, 1, 0, 0, &[0xDD; 6]);
        let mut mux = SiMux::new();

        mux.upsert_sdt_actual(sdt.clone())
            .expect("scheduled section");
        mux.upsert(
            well_known::SDT_BAT,
            bat.clone(),
            SectionInterval::from_const(Duration::from_millis(1000)),
        )
        .expect("scheduled section");

        let pkts = mux.poll(Timestamp::ZERO);
        let mut table_ids_seen = Vec::new();
        let mut reasm = SectionReassembler::default();
        for raw in &pkts {
            let pkt = TsPacket::parse(raw).unwrap();
            reasm.feed(pkt.payload.unwrap(), pkt.header.pusi);
        }
        while let Some(section) = reasm.pop_section() {
            table_ids_seen.push(section.as_ref()[0]);
        }

        assert!(table_ids_seen.contains(&0x42), "SDT actual missing");
        assert!(table_ids_seen.contains(&0x4A), "BAT missing");
    }

    // ── convenience constructors ─────────────────────────────────────────────

    #[test]
    fn upsert_sdt_actual_uses_correct_pid() {
        let s = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0x01]);
        let mut mux = SiMux::new();
        mux.upsert_sdt_actual(s).expect("scheduled section");
        let pkts = mux.poll(Timestamp::ZERO);
        let pids: Vec<Pid> = pkts
            .iter()
            .map(|raw| TsPacket::parse(raw).unwrap().header.pid)
            .collect();
        assert!(pids.iter().all(|&p| p == well_known::SDT_BAT));
    }

    #[test]
    fn upsert_pat_uses_correct_pid() {
        let s = test_fixtures::psi::long_section(0x00, 1, 0, 0, &[0x01, 0x02]);
        let mut mux = SiMux::new();
        mux.upsert_pat(s).expect("scheduled section");
        let pkts = mux.poll(Timestamp::ZERO);
        assert!(!pkts.is_empty());
        let pids: Vec<Pid> = pkts
            .iter()
            .map(|raw| TsPacket::parse(raw).unwrap().header.pid)
            .collect();
        assert!(pids.iter().all(|&p| p == well_known::PAT));
        assert!(
            mux.poll(Timestamp::from_nanos(
                (PAT_MAX_INTERVAL.duration() / 2).as_nanos() as u64
            ))
            .is_empty()
        );
        assert!(
            !mux.poll(Timestamp::from_nanos(
                PAT_MAX_INTERVAL.duration().as_nanos() as u64
            ))
            .is_empty()
        );
    }

    #[test]
    fn upsert_nit_uses_correct_pid() {
        let s = test_fixtures::psi::long_section(0x40, 1, 0, 0, &[0x01]);
        let mut mux = SiMux::new();
        mux.upsert_nit(s).expect("scheduled section");
        let pkts = mux.poll(Timestamp::ZERO);
        assert!(!pkts.is_empty());
        let pids: Vec<Pid> = pkts
            .iter()
            .map(|raw| TsPacket::parse(raw).unwrap().header.pid)
            .collect();
        assert!(pids.iter().all(|&p| p == well_known::NIT));
        assert!(
            mux.poll(Timestamp::from_nanos(
                (NIT_MAX_INTERVAL.duration() / 2).as_nanos() as u64
            ))
            .is_empty()
        );
        assert!(
            !mux.poll(Timestamp::from_nanos(
                NIT_MAX_INTERVAL.duration().as_nanos() as u64
            ))
            .is_empty()
        );
    }

    #[test]
    fn upsert_tdt_uses_correct_pid() {
        let s = test_fixtures::psi::long_section(0x70, 1, 0, 0, &[0x01]);
        let mut mux = SiMux::new();
        mux.upsert_tdt(s).expect("scheduled section");
        let pkts = mux.poll(Timestamp::ZERO);
        let pids: Vec<Pid> = pkts
            .iter()
            .map(|raw| TsPacket::parse(raw).unwrap().header.pid)
            .collect();
        assert!(pids.iter().all(|&p| p == well_known::TDT_TOT));
    }

    #[test]
    fn upsert_tot_uses_correct_pid() {
        let s = test_fixtures::psi::long_section(0x73, 1, 0, 0, &[0x01]);
        let mut mux = SiMux::new();
        mux.upsert_tot(s).expect("scheduled section");
        let pkts = mux.poll(Timestamp::ZERO);
        let pids: Vec<Pid> = pkts
            .iter()
            .map(|raw| TsPacket::parse(raw).unwrap().header.pid)
            .collect();
        assert!(pids.iter().all(|&p| p == well_known::TDT_TOT));
    }

    // ── typed input: empty, mixed-table and trailing-garbage registrations ──

    #[test]
    fn upsert_rejects_an_empty_input() {
        // `table_id_of(&[])` used to return 0, so an empty registration
        // collided with the PAT entry on PID 0 and re-armed its interval.
        let mut mux = SiMux::new();
        let pat = test_fixtures::psi::pat_section(1, 0, &[(1, 0x1000)]);
        mux.upsert_pat(pat.clone()).expect("PAT is schedulable");
        assert_eq!(
            mux.upsert(well_known::PAT, Vec::new(), PAT_MAX_INTERVAL),
            Err(SectionRejection::NotASection)
        );
        assert_eq!(mux.rejected().len(), 1);
        assert_eq!(mux.rejected()[0].pid, well_known::PAT);
        let pkts = mux.poll(Timestamp::ZERO);
        let mut reasm = SectionReassembler::default();
        for raw in &pkts {
            let pkt = TsPacket::parse(raw).unwrap();
            reasm.feed(pkt.payload.unwrap(), pkt.header.pusi);
        }
        let got: Vec<_> = core::iter::from_fn(|| reasm.pop_section()).collect();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].as_ref(), pat.as_slice());
    }

    #[test]
    fn upsert_rejects_two_sections_that_share_a_key() {
        let a = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0x01]);
        let b = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0x02]);
        let mut mux = SiMux::new();
        assert_eq!(
            mux.upsert(
                well_known::SDT_BAT,
                concat_sections(&[a, b]),
                SDT_ACTUAL_MAX_INTERVAL
            ),
            Err(SectionRejection::DuplicateSection)
        );
    }

    #[test]
    fn upsert_rejects_a_second_section_of_another_table() {
        let sdt = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0x01]);
        let bat = test_fixtures::psi::long_section(0x4A, 1, 0, 0, &[0x02]);
        let mut mux = SiMux::new();
        assert_eq!(
            mux.upsert(
                well_known::SDT_BAT,
                concat_sections(&[sdt, bat]),
                SDT_ACTUAL_MAX_INTERVAL
            ),
            Err(SectionRejection::MoreThanOneSection)
        );
    }

    #[test]
    fn upsert_keeps_a_complete_section_and_rejects_a_trailing_fragment() {
        // A truncated final section used to be dropped without a word, and
        // the caller could not tell it never went out.
        let whole = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0x01]);
        let mut with_tail = whole.clone();
        with_tail.extend_from_slice(&[0x4A, 0x00]);
        let mut mux = SiMux::new();
        mux.upsert(well_known::SDT_BAT, with_tail, SDT_ACTUAL_MAX_INTERVAL)
            .expect("the complete section is schedulable");
        let pkts = mux.poll(Timestamp::ZERO);
        let mut reasm = SectionReassembler::default();
        for raw in &pkts {
            let pkt = TsPacket::parse(raw).unwrap();
            reasm.feed(pkt.payload.unwrap(), pkt.header.pusi);
        }
        let got: Vec<_> = core::iter::from_fn(|| reasm.pop_section()).collect();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].as_ref(), whole.as_slice());
    }

    #[test]
    fn sections_of_one_table_are_keyed_by_their_section_number() {
        let first = test_fixtures::psi::long_section(0x4E, 1, 0, 0, &[0x01]);
        let second = test_fixtures::psi::long_section(0x4E, 1, 0, 1, &[0x02]);
        let mut mux = SiMux::new();
        let pid = Pid::from_const(0x0012);
        mux.upsert(pid, first.clone(), EIT_SCHED_MAX_INTERVAL)
            .expect("first section");
        mux.upsert(pid, second.clone(), EIT_SCHED_MAX_INTERVAL)
            .expect("second section");
        let pkts = mux.poll(Timestamp::ZERO);
        let mut reasm = SectionReassembler::default();
        for raw in &pkts {
            let pkt = TsPacket::parse(raw).unwrap();
            reasm.feed(pkt.payload.unwrap(), pkt.header.pusi);
        }
        let got: Vec<_> = core::iter::from_fn(|| reasm.pop_section()).collect();
        assert_eq!(got.len(), 2, "both sections of the table must go out");
    }

    #[test]
    fn upsert_pat_rejects_a_section_of_another_table() {
        let sdt = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0x01]);
        let mut mux = SiMux::new();
        assert_eq!(
            mux.upsert_pat(sdt),
            Err(SectionRejection::WrongTableId {
                expected: TABLE_ID_PAT,
                found: Some(0x42),
            })
        );
        assert!(mux.poll(Timestamp::ZERO).is_empty());
    }

    // ── round-trip through reassembler ───────────────────────────────────────

    #[test]
    fn simux_round_trip_single_entry() {
        let s1 = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xAA; 20]);
        let interval = SectionInterval::from_const(Duration::from_millis(100));
        let mut mux = SiMux::new();
        mux.upsert(Pid::from_const(0x0100), s1.clone(), interval)
            .expect("scheduled section");

        let pkts = mux.poll(Timestamp::ZERO);

        let mut reasm = SectionReassembler::default();
        for raw in &pkts {
            let pkt = TsPacket::parse(raw).unwrap();
            if pkt.header.pid == Pid::from_const(0x0100) {
                reasm.feed(pkt.payload.unwrap(), pkt.header.pusi);
            }
        }
        let got: Vec<_> = core::iter::from_fn(|| reasm.pop_section()).collect();
        assert_eq!(got.len(), 1, "round-trip must recover the section");
        assert_eq!(got[0].as_ref(), s1.as_slice());
    }

    #[test]
    fn simux_round_trip_multi_pid() {
        let s_a = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xA0; 10]);
        let s_b = test_fixtures::psi::long_section(0x46, 1, 0, 0, &[0xB0; 10]);
        let mut mux = SiMux::new();
        mux.upsert(
            Pid::from_const(0x0100),
            s_a.clone(),
            SectionInterval::from_const(Duration::from_millis(100)),
        )
        .expect("scheduled section");
        mux.upsert(
            Pid::from_const(0x0200),
            s_b.clone(),
            SectionInterval::from_const(Duration::from_millis(100)),
        )
        .expect("scheduled section");

        let pkts = mux.poll(Timestamp::ZERO);

        let mut reasm_a = SectionReassembler::default();
        let mut reasm_b = SectionReassembler::default();
        for raw in &pkts {
            let pkt = TsPacket::parse(raw).unwrap();
            match pkt.header.pid.value() {
                0x0100 => reasm_a.feed(pkt.payload.unwrap(), pkt.header.pusi),
                0x0200 => reasm_b.feed(pkt.payload.unwrap(), pkt.header.pusi),
                _ => {}
            }
        }
        let got_a: Vec<_> = core::iter::from_fn(|| reasm_a.pop_section()).collect();
        let got_b: Vec<_> = core::iter::from_fn(|| reasm_b.pop_section()).collect();
        assert_eq!(got_a.len(), 1);
        assert_eq!(got_b.len(), 1);
        assert_eq!(got_a[0].as_ref(), s_a.as_slice());
        assert_eq!(got_b[0].as_ref(), s_b.as_slice());
    }

    // ── CC continuity across polls ───────────────────────────────────────────

    #[test]
    fn continuity_counter_continuous_across_polls() {
        // Section large enough to span at least 2 packets.
        let s = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xAA; 250]);
        let mut mux = SiMux::new();
        mux.upsert(
            Pid::from_const(0x0100),
            s,
            SectionInterval::from_const(Duration::from_millis(100)),
        )
        .expect("scheduled section");

        let pkts1 = mux.poll(Timestamp::ZERO);
        assert!(pkts1.len() >= 2, "need ≥2 packets to test CC continuity");

        let last_cc_pkts1 = TsPacket::parse(&pkts1[pkts1.len() - 1])
            .unwrap()
            .header
            .continuity_counter;

        let pkts2 = mux.poll(Timestamp::from_millis(100));
        assert!(!pkts2.is_empty(), "second poll must emit");

        let first_cc_pkts2 = TsPacket::parse(&pkts2[0])
            .unwrap()
            .header
            .continuity_counter;

        assert_eq!(
            first_cc_pkts2.forward_distance_from(last_cc_pkts1),
            1,
            "CC must continue across poll cycles"
        );
    }

    // ── poll_into clears output ──────────────────────────────────────────────

    #[test]
    fn poll_into_clears_out_before_appending() {
        let s = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xAA; 5]);
        let mut mux = SiMux::new();
        mux.upsert(
            Pid::from_const(0x0100),
            s,
            SectionInterval::from_const(Duration::from_millis(100)),
        )
        .expect("scheduled section");

        let mut out = vec![[0u8; TS_PACKET_SIZE]; 42];
        let n = mux.poll_into(Timestamp::ZERO, &mut out);
        assert_eq!(n, out.len(), "out must contain only new packets");
    }

    // ── the 25 ms interval floor ─────────────────────────────────────────────

    /// The 25 ms floor is a `const` check on the spec's own named intervals, so
    /// a sub-floor one is a build error (verified by `from_const` panicking in
    /// a `const` context, which the `should_panic` surfaces here).
    #[test]
    #[should_panic(expected = "25 ms")]
    fn named_interval_const_rejects_sub_25ms_interval() {
        let _ = SectionInterval::from_const(Duration::from_millis(10));
    }

    /// EN 300 468 §5.1.4.1's 25 ms floor is enforced at construction, so a
    /// zero or sub-floor interval cannot reach the scheduler (it used to be a
    /// `debug_assert!` that release builds compiled out).
    #[test]
    fn section_interval_rejects_below_the_25ms_floor() {
        for bad in [
            Duration::ZERO,
            Duration::from_millis(1),
            Duration::from_millis(24),
            MIN_SECTION_INTERVAL - Duration::from_nanos(1),
        ] {
            let err = SectionInterval::try_new(bad).unwrap_err();
            assert_eq!(err.interval, bad);
            assert_eq!(err.min, MIN_SECTION_INTERVAL);
            assert!(SectionInterval::try_from(bad).is_err());
        }
        assert_eq!(
            SectionInterval::try_new(MIN_SECTION_INTERVAL)
                .expect("the floor itself is valid")
                .duration(),
            MIN_SECTION_INTERVAL
        );
        assert_eq!(
            Duration::from(SectionInterval::from_const(Duration::from_secs(2))),
            Duration::from_secs(2)
        );
    }

    #[test]
    fn upsert_accepts_25ms_interval() {
        let s = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xAA; 5]);
        let mut mux = SiMux::new();
        mux.upsert(
            Pid::from_const(0x0100),
            s,
            SectionInterval::from_const(MIN_SECTION_INTERVAL),
        )
        .expect("scheduled section");
        let pkts = mux.poll(Timestamp::ZERO);
        assert!(!pkts.is_empty());
    }

    // ── round-trip property (the mandatory acceptance oracle) ────────────────

    #[test]
    fn round_trip_single_short_section() {
        let s = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xAA; 10]);
        assert_round_trip(&[s]);
    }

    #[test]
    fn round_trip_one_byte_body() {
        let s = test_fixtures::psi::long_section(0x46, 1, 0, 0, &[0xBB]); // 4 bytes total
        assert_round_trip(&[s]);
    }

    #[test]
    fn round_trip_section_exactly_pusi_cap_boundary() {
        // A section whose total length is exactly PUSI_PAYLOAD_CAP (183).
        let body = vec![0xCC; PUSI_PAYLOAD_CAP - 3];
        let s = test_fixtures::psi::short_section(0x50, &body);
        assert_eq!(s.len(), PUSI_PAYLOAD_CAP);
        assert_round_trip(&[s]);
    }

    #[test]
    fn round_trip_section_just_over_pusi_cap() {
        // One byte more than fits in a PUSI packet → must span to continuation.
        let body = vec![0xDD; PUSI_PAYLOAD_CAP - 3 + 1];
        let s = test_fixtures::psi::short_section(0x52, &body);
        assert_eq!(s.len(), PUSI_PAYLOAD_CAP + 1);
        assert_round_trip(&[s]);
    }

    #[test]
    fn round_trip_section_spans_many_packets() {
        // A 2000-byte section spans ~11 packets.
        let body = vec![0xEE; 2000 - 3];
        let s = test_fixtures::psi::short_section(0x60, &body);
        assert_round_trip(&[s]);
    }

    #[test]
    fn round_trip_section_at_max_size() {
        // The maximum section (4096 total, the long-form ceiling), whose final
        // continuation packet carries the tail followed by 0xFF stuffing. Since
        // #148 the reassembler ignores that trailing stuffing instead of
        // counting it toward MAX_SECTION_SIZE, so a full-size section round-trips.
        let body = vec![0x11; 4096 - 3];
        let s = test_fixtures::psi::short_section(0x80, &body);
        assert_eq!(s.len(), 4096);
        assert_round_trip(&[s]);
    }

    #[test]
    fn round_trip_multiple_short_sections_in_one_batch() {
        let s1 = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0x01, 0x02]); // 5 bytes
        let s2 = test_fixtures::psi::long_section(0x46, 1, 0, 0, &[0x03]); // 4 bytes
        let s3 = test_fixtures::psi::long_section(0x4A, 1, 0, 0, &[0x04, 0x05, 0x06]); // 6 bytes
        assert_round_trip(&[s1, s2, s3]);
    }

    #[test]
    fn round_trip_section_ends_exactly_at_boundary() {
        // First section is exactly PUSI_PAYLOAD_CAP bytes — ends at packet
        // boundary.  Second section starts fresh in the next packet with
        // PUSI=1, pointer_field=0.
        let body1 = vec![0xA1; PUSI_PAYLOAD_CAP - 3];
        let s1 = test_fixtures::psi::short_section(0x50, &body1);
        assert_eq!(s1.len(), PUSI_PAYLOAD_CAP);

        let s2 = test_fixtures::psi::long_section(0x52, 1, 0, 0, &[0xB1, 0xB2]);
        assert_round_trip(&[s1, s2]);
    }

    #[test]
    fn continuation_packet_never_overshoots_next_section_start() {
        // First section is 366 bytes total: it fills the first (PUSI) packet's
        // 183-byte payload exactly, then the *second* packet (a continuation,
        // starting at pos=183) is exactly 183 bytes short of the next
        // section's start (183 + 183 = 366) — the mid-stream analogue of
        // `round_trip_section_ends_exactly_at_boundary` above, which only
        // exercises the boundary at pos=0.
        let body1 = vec![0xA5; 366 - 3];
        let s1 = test_fixtures::psi::short_section(0x50, &body1);
        assert_eq!(s1.len(), 366);
        let s2 = test_fixtures::psi::long_section(0x52, 1, 0, 0, &[0xB1, 0xB2, 0xB3]);

        let mut packetiser = SectionPacketiser::new(Pid::from_const(0x0100));
        let packets = packetiser.packetise(&[s1.as_slice(), s2.as_slice()]);

        for pkt_raw in &packets {
            let pkt = TsPacket::parse(pkt_raw).expect("parse generated packet");
            let payload = pkt.payload.expect("payload present");
            if pkt.header.pusi {
                let pointer = payload[0] as usize;
                // W1 (#1074): pre-fix this was 183 — a pointer_field pointing
                // past the packet's own payload, so no section byte actually
                // starts within it.
                assert!(
                    pointer < PUSI_PAYLOAD_CAP,
                    "pointer_field {pointer} leaves no room for a section to \
                     start within this packet (H.222.0 §2.4.4)"
                );
            }
        }

        assert_round_trip(&[s1, s2]);
    }

    #[test]
    fn round_trip_mix_small_large_sections() {
        // Mix of small and spanning sections that stress pointer_field and
        // concatenation.
        let s1 = test_fixtures::psi::long_section(0x10, 1, 0, 0, &[0xAA; 5]);
        let body2 = vec![0xBB; 200];
        let s2 = test_fixtures::psi::short_section(0x20, &body2);
        let s3 = test_fixtures::psi::long_section(0x30, 1, 0, 0, &[0xCC; 50]);
        let body4 = vec![0xDD; 800];
        let s4 = test_fixtures::psi::short_section(0x40, &body4);
        let s5 = test_fixtures::psi::long_section(0x50, 1, 0, 0, &[0xEE]); // 1-byte body
        assert_round_trip(&[s1, s2, s3, s4, s5]);
    }

    // ── continuity counter ───────────────────────────────────────────────────

    #[test]
    fn continuity_counter_increments_per_packet() {
        // Use a section large enough to span several packets.
        let body = vec![0xAA; 500];
        let section = test_fixtures::psi::short_section(0x42, &body);
        let mut p = SectionPacketiser::new(Pid::from_const(0x0100));

        let packets = p.packetise(&[&section]);
        assert!(packets.len() >= 3, "need multiple packets to test CC");

        let mut last_cc: Option<ContinuityCounter> = None;
        for pkt_raw in &packets {
            let pkt = TsPacket::parse(pkt_raw).unwrap();
            let cc = pkt.header.continuity_counter;
            if let Some(last) = last_cc {
                assert_eq!(
                    cc.forward_distance_from(last),
                    1,
                    "CC must increment per packet"
                );
            }
            last_cc = Some(cc);
        }
    }

    #[test]
    fn continuity_counter_wraps_and_continues_across_calls() {
        let mut p = SectionPacketiser::with_continuity(Pid::from_const(0x0100), cc(14));
        // Section large enough to span at least 3 packets.
        let body = vec![0xBB; 500];
        let s = test_fixtures::psi::short_section(0x42, &body);

        // First call: CC 14, 15, 0, …
        let pkts1 = p.packetise(&[&s]);
        assert!(pkts1.len() >= 3, "section must span ≥3 packets");
        let ccs1: Vec<u8> = pkts1
            .iter()
            .map(|b| {
                TsPacket::parse(b)
                    .unwrap()
                    .header
                    .continuity_counter
                    .value()
            })
            .collect();
        assert_eq!(ccs1[0], 14);
        assert_eq!(ccs1[1], 15);
        assert_eq!(ccs1[2], 0);

        // Second call: CC continues from where first left off.
        let previous = ContinuityCounter::try_new(ccs1.last().copied().unwrap_or(0))
            .expect("the last emitted counter is 4-bit");
        let pkts2 = p.packetise(&[&s]);
        let cc_first_pkt2 = TsPacket::parse(&pkts2[0])
            .unwrap()
            .header
            .continuity_counter;
        assert_eq!(cc_first_pkt2.forward_distance_from(previous), 1);
    }

    // ── PUSI placement ──────────────────────────────────────────────────────

    #[test]
    fn pusi_set_when_section_starts() {
        let s = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xAA; 10]);
        let mut p = SectionPacketiser::new(Pid::from_const(0x0100));
        let packets = p.packetise(&[&s]);
        assert!(!packets.is_empty());
        let pkt = TsPacket::parse(&packets[0]).unwrap();
        assert!(pkt.header.pusi, "first packet must have PUSI=1");
    }

    #[test]
    fn pusi_not_set_on_mid_section_continuation() {
        let body = vec![0xAA; 500];
        let s = test_fixtures::psi::short_section(0x42, &body);
        let mut p = SectionPacketiser::new(Pid::from_const(0x0100));
        let packets = p.packetise(&[&s]);
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
        let s1 = test_fixtures::psi::short_section(0x52, &body1);
        assert_eq!(s1.len(), 200);
        let s2 = test_fixtures::psi::long_section(0x54, 1, 0, 0, &[0xB1; 38]); // 50-byte section
        assert_eq!(s2.len(), 50);

        let mut p = SectionPacketiser::new(Pid::from_const(0x0100));
        let packets = p.packetise(&[&s1, &s2]);

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
        let s = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xAA; 5]); // 8 bytes total
        let mut p = SectionPacketiser::new(Pid::from_const(0x0100));
        let packets = p.packetise(&[&s]);

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
        let s1 = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xAA; 10]);
        let s2 = test_fixtures::psi::long_section(0x46, 1, 0, 0, &[0xBB; 5]);

        let mut p = SectionPacketiser::new(Pid::from_const(0x0100));
        let packets = p.packetise(&[&s1, &s2]);

        let mut reasm = SectionReassembler::default();
        for pkt_raw in &packets {
            let pkt = TsPacket::parse(pkt_raw).unwrap();
            reasm.feed(pkt.payload.unwrap(), pkt.header.pusi);
        }

        let got: Vec<_> = core::iter::from_fn(|| reasm.pop_section()).collect();
        assert_eq!(got.len(), 2);
        assert!(
            reasm.is_empty(),
            "stuffing tail must be discarded, not buffered"
        );
    }

    // ── misc ─────────────────────────────────────────────────────────────────

    #[test]
    fn empty_batch_produces_no_packets() {
        let mut p = SectionPacketiser::new(Pid::from_const(0x0100));
        let packets: Vec<[u8; TS_PACKET_SIZE]> = p.packetise(&[]);
        assert!(packets.is_empty());
    }

    #[test]
    fn packetise_into_clears_out_first() {
        let s = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xAA; 5]);
        let mut p = SectionPacketiser::new(Pid::from_const(0x0100));

        let mut out = vec![[0u8; TS_PACKET_SIZE]; 99]; // pre-existing junk
        let n = p.packetise_into(&[&s], &mut out);
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
        let p = SectionPacketiser::new(Pid::from_const(0x1234));
        assert_eq!(p.pid(), Pid::from_const(0x1234));
    }

    #[test]
    fn with_continuity_masks_to_4_bits() {
        let expected = ContinuityCounter::try_new(0x0E).expect("4-bit counter");
        let p = SectionPacketiser::with_continuity(Pid::from_const(0x0100), expected);
        assert_eq!(p.continuity_counter(), expected);
    }

    #[test]
    fn has_payload_always_true_no_adaptation() {
        let s = test_fixtures::psi::long_section(0x42, 1, 0, 0, &[0xAA; 50]);
        let mut p = SectionPacketiser::new(Pid::from_const(0x0100));
        let packets = p.packetise(&[&s]);
        for pkt_raw in &packets {
            let pkt = TsPacket::parse(pkt_raw).unwrap();
            assert!(pkt.header.has_payload(), "every packet must carry payload");
            assert!(
                !pkt.header.has_adaptation(),
                "no adaptation field is emitted"
            );
            assert!(!pkt.header.tei, "TEI must be false");
            assert_eq!(
                pkt.header.scrambling,
                ScramblingControl::NotScrambled,
                "scrambling must be 0"
            );
        }
    }
}
