//! `PatPmtVersionCheck` — surfaces PAT and PMT version_number changes.
//!
//! ETSI EN 300 468 §5.1: the `version_number` in the PSI/SI section header
//! increments (mod 32) each time the table data changes. This check tracks
//! version changes for PAT (table_id 0x00, PID 0x0000) and PMT
//! (table_id 0x02, PID from PAT) across the stream, surfacing Info findings.
//!
//! The check reassembles sections from the TS byte stream using `mpeg-ts`'s
//! `SectionReassembler`, validates each section's CRC-32 (ISO/IEC 13818-1
//! Annex B, via [`mpeg_ts::section::Section::validate_crc`]) and parses it
//! with `dvb-si`'s typed [`PatSection`]/[`PmtSection`] — never a hand-rolled
//! walk over the section body. A corrupted section is dropped rather than
//! reporting a spurious version change.
//!
//! Only *current* (`current_next_indicator == 1`) sections are compared: a
//! `current_next_indicator == 0` section carries the *next* table generation
//! (ISO/IEC 13818-1 §2.4.4.11), so comparing it against the current one would
//! make a mux that pre-announces a version flip-flop findings.
//!
//! Version state is keyed on `(pid, table_id, table_id_extension)` — two
//! PMTs may legally share a PID (ISO/IEC 13818-1 §2.4.4.8), and they are
//! distinguished only by their `table_id_extension` (the `program_number`),
//! so keying on PID alone emits a finding on every repetition.

use alloc::collections::btree_map::BTreeMap;
use alloc::vec::Vec;

use broadcast_common::Parse;
use dvb_si::tables::pat::{self, PatSection};
use dvb_si::tables::pmt::{self, PmtSection};
use mpeg_ts::section::Section;
use mpeg_ts::ts::{SectionReassembler, TS_PACKET_SIZE, TsPacket};

use crate::Diagnostic;
use crate::Report;
use crate::report::{Finding, Location, Severity};

/// Identifies one versioned table within the multiplex: the PSI PID it is
/// carried on, its `table_id` and its `table_id_extension`.
///
/// All three are required — see the module docs for why the PID alone is not
/// enough (ISO/IEC 13818-1 §2.4.4.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TableKey {
    pid: u16,
    table_id: u8,
    table_id_extension: u16,
}

/// Tracks PAT and PMT version_number changes across the stream.
///
/// Parses sections from the TS byte-stream using `mpeg-ts`'s
/// `SectionReassembler` and `dvb-si`'s typed PAT/PMT parsers. Reports a
/// finding (Info severity) each time a version_number changes for PAT or any
/// discovered PMT table.
#[derive(Debug, Clone, Copy)]
pub struct PatPmtVersionCheck;

/// The rule id and human name for a versioned table we track.
const fn table_labels(table_id: u8) -> Option<(&'static str, &'static str)> {
    match table_id {
        pat::TABLE_ID => Some(("pat-version", "PAT")),
        pmt::TABLE_ID => Some(("pmt-version", "PMT")),
        _ => None,
    }
}

/// A PAT section that passed every gate: typed-parseable, CRC-valid, current
/// (not a next-generation section), and not from a reserved PID.
///
/// Used **both** for version tracking and for PMT-PID discovery, so a
/// corrupt or stale PAT can never register a PMT PID (audit MD-W1(a)/(c)).
fn current_pat(raw: &[u8]) -> Option<PatSection> {
    let pat = PatSection::parse(raw).ok()?;
    // The typed parse does not cover the CRC — validate it explicitly
    // (ISO/IEC 13818-1 Annex B).
    Section::parse(raw).ok()?.validate_crc(raw).ok()?;
    // A next-generation section is not the current one (ISO/IEC 13818-1
    // §2.4.4.11).
    pat.current_next_indicator.then_some(pat)
}

/// The PMT PIDs a current PAT declares, ignoring the NIT entry
/// (`program_number == 0`, ISO/IEC 13818-1 §2.4.4.3) and any reserved PID.
///
/// A PAT entry naming PID 0 would otherwise collide with the PAT PID itself;
/// PID 0x1FFF (`NULL`) is reserved and can never carry a PMT
/// (§2.4.3.3).
fn declared_pmt_pids(pat: &PatSection) -> impl Iterator<Item = u16> + '_ {
    pat.entries.iter().filter_map(|entry| {
        let pid = entry.pid;
        (entry.program_number != pat::PROGRAM_NUMBER_NIT && pid != pat::PID && pid != NULL_PID)
            .then_some(pid)
    })
}

/// The reserved null-packet PID (`0x1FFF`), which never carries a section
/// (ISO/IEC 13818-1 §2.4.3.3).
const NULL_PID: u16 = 0x1FFF;

impl PatPmtVersionCheck {
    fn process_section(
        &self,
        packet_index: usize,
        pid: u16,
        raw: &[u8],
        versions: &mut BTreeMap<TableKey, u8>,
        report: &mut Report,
    ) {
        // Typed parse first: it yields the table_id_extension, version_number
        // and current_next_indicator without any hand-rolled bit masking. For
        // a PAT the same gate discovery uses (CRC + current_next) is applied
        // here, so the two can never disagree about which sections are real.
        if pid == pat::PID && current_pat(raw).is_none() {
            return;
        }
        let (table_id, table_id_extension, version) = if pid == pat::PID {
            let Ok(pat) = PatSection::parse(raw) else {
                return;
            };
            (pat::TABLE_ID, pat.transport_stream_id, pat.version_number)
        } else {
            let Ok(pmt) = PmtSection::parse(raw) else {
                return;
            };
            if !pmt.current_next_indicator {
                return;
            }
            (pmt::TABLE_ID, pmt.program_number, pmt.version_number)
        };

        let Some((rule_id, table_name)) = table_labels(table_id) else {
            return;
        };

        let key = TableKey {
            pid,
            table_id,
            table_id_extension,
        };

        if let Some(&prev_ver) = versions.get(&key)
            && prev_ver != version
        {
            report.push(Finding::new(
                Severity::Info,
                Location::new(packet_index, u32::from(pid)),
                rule_id,
                alloc::format!(
                    "{table_name} version_number changed: {prev_ver} → {version} \
                     (table_id_ext=0x{table_id_extension:04X})",
                ),
            ));
        }

        versions.insert(key, version);
    }
}

impl Diagnostic for PatPmtVersionCheck {
    fn run(&self, ts: &[u8], report: &mut Report) {
        let n_packets = ts.len() / TS_PACKET_SIZE;

        // SectionReassembler per PID for PSI sections.
        let mut reassemblers: BTreeMap<u16, SectionReassembler> = BTreeMap::new();
        // Track (pid, table_id, table_id_ext) -> version_number.
        let mut versions: BTreeMap<TableKey, u8> = BTreeMap::new();

        // Always watch the PAT PID.
        reassemblers.entry(pat::PID).or_default();

        for i in 0..n_packets {
            let offset = i * TS_PACKET_SIZE;
            let raw = &ts[offset..offset + TS_PACKET_SIZE];

            let Ok(pkt) = TsPacket::parse(raw) else {
                continue;
            };

            let pid = pkt.header.pid;

            // Only process PIDs we're watching.
            if !reassemblers.contains_key(&pid) {
                continue;
            }

            let Some(payload) = pkt.payload else {
                continue;
            };

            // Feed payload into section reassembler.
            let pusi = pkt.header.pusi;
            reassemblers.get_mut(&pid).unwrap().feed(payload, pusi);

            // Collect new PMT PIDs discovered during drain — we cannot borrow
            // `reassemblers` mutably while iterating over sections.
            let mut new_pmt_pids: Vec<u16> = Vec::new();

            // Drain completed sections.
            while let Some(section) = reassemblers.get_mut(&pid).unwrap().pop_section() {
                let section_data = &section[..];
                self.process_section(i, pid, section_data, &mut versions, report);

                // PAT discovery goes through the SAME gate as version
                // tracking — typed parse, CRC-valid, current — so a corrupt
                // or next-generation PAT cannot register a PMT PID
                // (audit MD-W1(a)/(c)), and a PAT entry naming PID 0 or the
                // reserved NULL PID is ignored rather than watched.
                if pid == pat::PID
                    && let Some(pat) = current_pat(section_data)
                {
                    for pmt_pid in declared_pmt_pids(&pat) {
                        if !new_pmt_pids.contains(&pmt_pid) {
                            new_pmt_pids.push(pmt_pid);
                        }
                    }
                }
            }

            // Register new PMT PIDs after draining sections.
            for pmt_pid in new_pmt_pids {
                reassemblers.entry(pmt_pid).or_default();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::codec_common::tests::{
        TEST_PMT_PID, build_pat_pmt_ts, build_pat_pmt_ts_versioned, build_pmt_ts_on_pid,
        build_shared_pid_pmt_ts,
    };
    use dvb_si::tables::pmt::StreamType;

    /// ISO/IEC 13818-1 §2.4.4.1: the 3-byte section header precedes the bytes
    /// counted by `section_length`.
    const SECTION_HEADER_LEN: usize = 3;
    /// ISO/IEC 13818-1 Annex B: the CRC-32 suffix closing a long-form section.
    const CRC_LEN: usize = 4;
    /// PID field width used by a PAT program entry (`reserved` 3 bits + 13-bit
    /// PID, ISO/IEC 13818-1 §2.4.4.3).
    const PID_HI_MASK: u8 = 0x1F;
    /// Offset of a TS packet's payload when it carries no adaptation field
    /// (4-byte fixed header, ISO/IEC 13818-1 §2.4.3.2/§2.4.3.4). Every
    /// fixture built by `codec_common::tests` is payload-only.
    const ONE_PACKET_PAYLOAD_OFFSET: usize = 4;

    fn check(ts: &[u8]) -> Report {
        let mut report = Report::new();
        PatPmtVersionCheck.run(ts, &mut report);
        report
    }

    /// Total byte length of the long-form section at the start of `payload`:
    /// ISO/IEC 13818-1 §2.4.4.1 defines `section_length` as the bytes
    /// *following* the 3-byte section header.
    fn long_form_section_len(payload: &[u8]) -> usize {
        let section = Section::parse(payload).expect("long-form section");
        assert!(section.section_syntax_indicator, "PAT is long-form");
        let total = SECTION_HEADER_LEN + usize::from(section.section_length);
        assert!(total >= CRC_LEN, "long-form section carries a CRC-32");
        total
    }

    /// A single stable PAT+PMT generation repeated `n` times is not a version
    /// change — this is the baseline every other test is measured against.
    #[test]
    fn stable_generation_reports_no_findings() {
        let one = build_pat_pmt_ts(&[(0x0101, StreamType::H264)]);
        let mut ts = Vec::new();
        for _ in 0..5 {
            ts.extend_from_slice(&one);
        }
        assert_eq!(check(&ts).findings().len(), 0);
    }

    /// `(e)` regression: two PMTs sharing one PID, distinguished only by
    /// `table_id_extension`, must not emit a finding on every repetition.
    /// Keying on `(pid, table_id)` alone (the pre-fix behaviour) alternates
    /// the remembered version between the two sub-tables.
    #[test]
    fn two_pmts_on_one_pid_do_not_flip_flop() {
        // The two sub-tables deliberately carry DIFFERENT version_numbers:
        // keyed on the PID alone, each repetition overwrites the remembered
        // version with the other program's, so every cycle after the first
        // looks like a version change.
        let one = build_shared_pid_pmt_ts(&[
            (1, 0x0101, StreamType::H264, 0),
            (2, 0x0102, StreamType::Hevc, 1),
        ]);
        let mut repeated = Vec::new();
        for _ in 0..5 {
            repeated.extend_from_slice(&one);
        }
        let report = check(&repeated);
        assert_eq!(
            report.findings().len(),
            0,
            "two PMTs sharing PID {TEST_PMT_PID:#06X} with stable versions must \
             not produce findings, got {:?}",
            report.findings(),
        );
    }

    /// `(d)` regression: a `current_next_indicator = 0` section carries the
    /// NEXT table generation, never the current one. A mux that alternates a
    /// next-version section with the current one must not report a version
    /// change.
    #[test]
    fn next_version_sections_are_not_compared() {
        let current = build_pat_pmt_ts(&[(0x0101, StreamType::H264)]);
        let next = build_pat_pmt_ts_versioned(&[(0x0101, StreamType::H264)], 1, false);
        let mut ts = Vec::new();
        for _ in 0..5 {
            ts.extend_from_slice(&current);
            ts.extend_from_slice(&next);
        }
        let report = check(&ts);
        assert_eq!(
            report.findings().len(),
            0,
            "current_next_indicator = 0 sections must not be compared, got {:?}",
            report.findings(),
        );
    }

    /// A genuine version change IS reported, once, carrying the packet index
    /// at which the change was seen (not packet 0 — finding `(f)`).
    #[test]
    fn version_change_is_reported_once_at_the_changing_packet() {
        let v0 = build_pat_pmt_ts(&[(0x0101, StreamType::H264)]);
        let v1 = build_pat_pmt_ts_versioned(&[(0x0101, StreamType::H264)], 1, true);
        let mut ts = Vec::new();
        ts.extend_from_slice(&v0);
        ts.extend_from_slice(&v0);
        ts.extend_from_slice(&v1);

        let report = check(&ts);
        let findings = report.findings();
        assert_eq!(
            findings
                .iter()
                .filter(|f| f.rule_id == "pat-version")
                .count(),
            1,
            "exactly one version change expected, got {findings:?}",
        );
        let pmt_finding = findings
            .iter()
            .find(|f| f.rule_id == "pmt-version")
            .expect("pmt-version finding");
        assert!(
            pmt_finding.location.packet > 0,
            "pmt-version finding must carry the packet index it was seen at, \
             got {:?}",
            pmt_finding.location,
        );
    }

    /// `(a)` regression, the misfire the audit describes: a PAT carrying one
    /// program is 16 bytes long, so its trailing CRC_32 lands exactly where a
    /// hand-rolled `while off + 4 <= section_data.len()` walk looks for a
    /// fourth program entry — registering the CRC's own bytes as a PMT PID.
    ///
    /// When a PMT genuinely sits on that aliased PID with a changing
    /// `version_number`, the old walk reports `pmt-version` findings for a
    /// table the PAT never declared. Typed PAT parsing walks only the entries
    /// inside `section_length` minus the CRC, so the aliased PID is never put
    /// under watch.
    #[test]
    fn crc_32_is_not_read_as_a_program_entry() {
        let one = build_pat_pmt_ts(&[(0x0101, StreamType::H264)]);

        // The four bytes a hand-rolled walk mistakes for a program entry: the
        // section's trailing CRC_32 (ISO/IEC 13818-1 Annex B).
        let pkt = TsPacket::parse(&one[..TS_PACKET_SIZE]).expect("TS packet");
        let payload = pkt.payload.expect("PAT packet carries a payload");
        let pointer_field = usize::from(*payload.first().expect("pointer_field"));
        let section_start = ONE_PACKET_PAYLOAD_OFFSET + 1 + pointer_field;
        let section_len = long_form_section_len(&one[section_start..]);
        let crc = &one[section_start + section_len - CRC_LEN..section_start + section_len];
        let aliased_pid = (u16::from(crc[2] & PID_HI_MASK) << 8) | u16::from(crc[3]);
        assert_ne!(
            aliased_pid, TEST_PMT_PID,
            "test setup: the CRC must alias a different PID than the real PMT",
        );

        let mut ts = Vec::new();
        for cycle in 0..3u8 {
            ts.extend_from_slice(&one);
            // A genuine PMT on the aliased PID, its version changing between
            // repetitions — the shape the old walk misreports as `pmt-version`.
            ts.extend_from_slice(&build_pmt_ts_on_pid(aliased_pid, cycle));
        }

        let report = check(&ts);
        assert_eq!(
            report.findings().len(),
            0,
            "the CRC_32's own bytes must not be registered as a PMT PID ({aliased_pid:#06X}), got {:?}",
            report.findings(),
        );
    }

    /// A CRC-invalid PAT must register **no** PMT PID — discovery runs the
    /// same typed-parse + CRC + `current_next` gate as version tracking
    /// (audit MD-W1(a)/(c)).
    ///
    /// Bites because the corrupt PAT's own program entry names a real PID:
    /// after a corrupt PAT, a version-changed PMT sent on that PID must NOT
    /// be reported, since the PID was never put under watch.
    #[test]
    fn corrupt_pat_registers_no_pmt_pid() {
        use crate::diagnostics::codec_common::tests::{TEST_PMT_PID, build_pat_pmt_ts};
        use dvb_si::tables::pmt::StreamType;

        // Two generations of the same PAT+PMT, so a *valid* stream would
        // report a version change.
        let v0 = build_pat_pmt_ts(&[(0x0101, StreamType::H264)]);
        let v1 = build_pat_pmt_ts_versioned(&[(0x0101, StreamType::H264)], 1, true);
        let mut valid = Vec::new();
        valid.extend_from_slice(&v0);
        valid.extend_from_slice(&v1);
        assert_eq!(
            check(&valid)
                .findings()
                .iter()
                .filter(|f| f.rule_id == "pmt-version")
                .count(),
            1,
            "control: the valid stream must report the change",
        );

        // Now corrupt only the PAT's payload byte (leaving its CRC stale) in
        // the first generation.
        let mut corrupt_v0 = v0.clone();
        // Corrupt a byte the typed parse accepts but the CRC covers — the
        // last byte of `section_number`, at section offset 6.
        //
        // Chosen deliberately: corrupting the *program entry's* PID would
        // change which PID an ungated walk watches, so the two cases would
        // not be comparable. Here the corrupt PAT still names the same PMT
        // PID; only its CRC is wrong.
        let pat_section_number = 4 + 1 + 6;
        corrupt_v0[pat_section_number] ^= 0xFF;
        let mut corrupt = Vec::new();
        corrupt.extend_from_slice(&corrupt_v0);
        corrupt.extend_from_slice(&v1);

        let report = check(&corrupt);
        assert!(
            report.findings().iter().all(|f| f.rule_id != "pmt-version"),
            "a corrupt PAT must not register PMT PID {TEST_PMT_PID:#06X}, so its version change must go unreported; got {:?}",
            report.findings(),
        );
    }

    /// `(a)`/`(c)` regression: the PAT section's trailing CRC-32 must never be
    /// read as a program entry, and a corrupt CRC must be dropped rather than
    /// registering a bogus PMT PID (which would then reassemble PES payloads
    /// as sections, producing garbage `pat-version` findings). A PAT with a
    /// flipped CRC byte must produce no finding.
    #[test]
    fn corrupted_pat_is_dropped_not_read_as_a_program() {
        let mut ts = build_pat_pmt_ts(&[(0x0101, StreamType::H264)]);
        // Locate the PAT section inside the first TS packet through the
        // packet's own payload accessor and its `pointer_field`
        // (ISO/IEC 13818-1 §2.4.3.4), never by assuming a fixed offset.
        let pkt = TsPacket::parse(&ts[..TS_PACKET_SIZE]).expect("TS packet");
        assert!(pkt.header.pusi, "PAT packet starts a section");
        let payload = pkt.payload.expect("PAT packet has a payload");
        let pointer_field = usize::from(*payload.first().expect("pointer_field"));
        let payload_start = TS_PACKET_SIZE - payload.len() + 1 + pointer_field;
        let section_len = long_form_section_len(&ts[payload_start..]);
        let crc_start = payload_start + section_len - CRC_LEN;

        for byte in &mut ts[crc_start..crc_start + CRC_LEN] {
            *byte ^= 0xFF;
        }
        assert!(
            Section::parse(&ts[payload_start..])
                .expect("PAT section")
                .validate_crc(&ts[payload_start..])
                .is_err(),
            "test setup must actually corrupt the CRC",
        );

        let mut repeated = Vec::new();
        for _ in 0..3 {
            repeated.extend_from_slice(&ts);
        }
        let report = check(&repeated);
        assert_eq!(
            report.findings().len(),
            0,
            "a corrupted PAT must be dropped, not parsed, got {:?}",
            report.findings(),
        );
    }
}
