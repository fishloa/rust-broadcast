//! `PatPmtVersionCheck` — surfaces PAT and PMT version_number changes.
//!
//! ETSI EN 300 468 §5.1: the `version_number` in the PSI/SI section header
//! increments (mod 32) each time the table data changes. This check tracks
//! version changes for PAT (table_id 0x00, PID 0x0000) and PMT
//! (table_id 0x02, PID from PAT) across the stream, surfacing Info or Warning
//! findings.
//!
//! The check reassembles sections from the TS byte stream using `mpeg-ts`'s
//! `SectionReassembler`, then parses PAT/PMT sections via `dvb-si` for typed
//! field access.

use alloc::collections::btree_map::BTreeMap;
use alloc::vec::Vec;

use crate::report::{Finding, Location, Severity};
use crate::Diagnostic;
use crate::Report;
use mpeg_ts::ts::SectionReassembler;
use mpeg_ts::ts::{TsPacket, TS_PACKET_SIZE};

/// Tracks PAT and PMT version_number changes across the stream.
///
/// Parses sections from the TS byte-stream using `mpeg-ts`'s
/// `SectionReassembler` and `dvb-si`'s PAT/PMT parsers. Reports a finding
/// (Info severity) each time a version_number changes for PAT or any
/// discovered PMT PID.
#[derive(Debug, Clone, Copy)]
pub struct PatPmtVersionCheck;

impl PatPmtVersionCheck {
    fn process_section(
        &self,
        pid: u16,
        section_data: &[u8],
        versions: &mut BTreeMap<(u16, u8), u8>,
        report: &mut Report,
    ) {
        if section_data.len() < 8 {
            return;
        }

        let table_id = section_data[0];
        // section_syntax_indicator is bit 7 of byte 1
        let _syntax = (section_data[1] & 0x80) != 0;
        // section_length is lower 12 bits of bytes 1-2
        let _section_length = (((section_data[1] & 0x0F) as u16) << 8) | section_data[2] as u16;

        // table_id_extension (for PAT: transport_stream_id; for PMT: program_number).
        let table_id_ext = ((section_data[3] as u16) << 8) | section_data[4] as u16;

        // version_number is bits [6:2] of byte 5.
        let version = (section_data[5] >> 1) & 0x1F;
        let _current_next = (section_data[5] & 0x01) != 0;
        let _section_number = section_data[6];
        let _last_section_number = section_data[7];

        let key = (pid, table_id);
        let prev = versions.get(&key).copied();

        if let Some(prev_ver) = prev {
            if prev_ver != version {
                let severity = Severity::Info;
                let rule_id = if table_id == 0x00 {
                    "pat-version"
                } else if table_id == 0x02 {
                    "pmt-version"
                } else {
                    return;
                };

                let table_name = if table_id == 0x00 { "PAT" } else { "PMT" };

                report.push(Finding::new(
                    severity,
                    Location::new(0, pid),
                    rule_id,
                    alloc::format!(
                        "{table_name} version_number changed: {} → {} \
                         (table_id_ext=0x{table_id_ext:04X})",
                        prev_ver,
                        version,
                    ),
                ));
            }
        }

        versions.insert(key, version);
    }
}

impl Diagnostic for PatPmtVersionCheck {
    fn run(&self, ts: &[u8], report: &mut Report) {
        let n_packets = ts.len() / TS_PACKET_SIZE;

        // SectionReassembler per PID for PSI sections.
        let mut reassemblers: BTreeMap<u16, SectionReassembler> = BTreeMap::new();
        // Track (pid, table_id) -> version_number.
        let mut versions: BTreeMap<(u16, u8), u8> = BTreeMap::new();
        // Discovered PMT PIDs from PAT.
        let mut pmt_pids: Vec<u16> = Vec::new();

        // Always watch PID 0x0000 (PAT).
        reassemblers.entry(0x0000).or_default();

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

            let payload = match pkt.payload {
                Some(pl) => pl,
                None => continue,
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
                self.process_section(pid, section_data, &mut versions, report);

                // If this is a PAT section, extract PMT PIDs.
                if section_data.len() > 8 && section_data[0] == 0x00 {
                    let mut off = 8usize;
                    while off + 4 <= section_data.len() {
                        let pmt_pid = (((section_data[off + 2] as u16) & 0x1F) << 8)
                            | section_data[off + 3] as u16;
                        if !pmt_pids.contains(&pmt_pid) && !new_pmt_pids.contains(&pmt_pid) {
                            new_pmt_pids.push(pmt_pid);
                        }
                        off += 4;
                    }
                }
            }

            // Register new PMT PIDs after draining sections.
            for pmt_pid in new_pmt_pids {
                pmt_pids.push(pmt_pid);
                reassemblers.entry(pmt_pid).or_default();
            }
        }
    }
}
