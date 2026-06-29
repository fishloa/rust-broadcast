//! PAT regeneration operation.
//!
//! Rebuilds the Program Association Table (PAT) so the program table is
//! consistent with the Program Map Tables (PMTs) actually present in the
//! stream — independent of whether the original PAT is present or valid.
//!
//! # How it works (PMT scan is the authoritative source)
//!
//! A PAT maps `program_number → PMT PID`, but a corrupt or stripped PAT can't
//! be trusted to supply that mapping.  Instead, every PMT section (table_id
//! `0x02`, ISO/IEC 13818-1 §2.4.4.8) carries its own `program_number` in its
//! body and arrives on a known PID.  This op therefore *discovers* the mapping
//! by scanning sections:
//!
//! - Candidate PIDs (any non-PAT, non-null PID whose first section begins with
//!   the PMT `table_id`) are reassembled with [`mpeg_ts::ts::SectionReassembler`].
//! - Each completed section is parsed with [`dvb_si::tables::pmt::PmtSection`];
//!   on success the pair `(pmt.program_number → arriving PID)` is recorded.
//! - The regenerated PAT is built from THAT mapping (via the dvb-si
//!   [`PatSection`] builder + [`mpeg_ts::mux::SectionPacketizer`]), so even a
//!   destroyed PAT can be rebuilt as long as the PMTs are present.
//!
//! The original PAT is still observed as a *hint* for the `transport_stream_id`
//! (which is not derivable from the PMTs), but the PMT scan is authoritative for
//! the program mapping.
//!
//! # Emit position (in-position, not at flush)
//!
//! The regenerated PAT replaces the original PAT **in position**:
//!
//! - Each incoming PAT packet (PID `0x0000`) slot is replaced by a regenerated
//!   PAT packet built from the PMT-derived mapping (so a corrupt PAT slot still
//!   becomes correct), *once the mapping is known*.
//! - If the stream has **no** PAT packets at all (fully stripped), the
//!   regenerated PAT is emitted once early — as soon as the mapping is first
//!   resolved.
//!
//! ## Single-pass limitation
//!
//! This op is single-pass and streaming.  PAT slots that arrive **before** the
//! first PMT section has been seen cannot be replaced (the mapping is not yet
//! known) and are dropped; the regenerated PAT then appears at the first PAT
//! slot following the PMT, or — if the PAT was stripped — immediately after the
//! mapping resolves.  In practice broadcast muxes cycle the PAT ahead of and
//! alongside the PMTs, so at least one PAT slot follows the PMT.
//!
//! # Spec
//!
//! ISO/IEC 13818-1 (= ITU-T H.222.0) §2.4.4.3 (PAT) / §2.4.4.8 (PMT).

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use broadcast_common::traits::{Parse, Serialize};
use dvb_si::tables::pat::{PatEntry, PatSection};
use dvb_si::tables::pmt::{self, PmtSection};
use mpeg_ts::mux::SectionPacketizer;
use mpeg_ts::ts::{extract_ts_payload, SectionReassembler, TsHeader, TS_PACKET_SIZE};

use crate::ops::{Op, StreamModel};

/// PAT well-known PID (ISO/IEC 13818-1 §2.4.4.3).
const PAT_PID: u16 = 0x0000;
/// Null-packet PID (ISO/IEC 13818-1 §2.4.1).
const NULL_PID: u16 = 0x1FFF;

// ── The operation ──────────────────────────────────────────────────────────

/// PAT regeneration operation.
///
/// Discovers the program → PMT-PID mapping by scanning PMT sections, then
/// emits a freshly-built PAT in the position of the original PAT packets.
pub(crate) struct PsiRegenOp {
    /// Authoritative mapping: program_number → PMT PID, derived from parsed PMT
    /// sections (each PMT carries its own program_number in its body).
    pmt_programs: BTreeMap<u16, u16>,
    /// Per-PID section reassemblers for candidate PMT PIDs.
    pmt_reasm: BTreeMap<u16, SectionReassembler>,
    /// transport_stream_id hint, copied from the original PAT if one is seen.
    /// Not derivable from PMTs, so absent a PAT we fall back to 0.
    transport_stream_id: Option<u16>,
    /// Whether any PAT slot (PID 0x0000) has been seen.  If so we replace slots
    /// in-position; if not (fully stripped) we early-emit on PMT-cycle wrap.
    seen_pat_slot: bool,
    /// Whether a regenerated PAT has already been emitted (so the stripped-PAT
    /// early-emit path fires at most once).
    emitted_pat: bool,
}

impl PsiRegenOp {
    pub(crate) fn new() -> Self {
        Self {
            pmt_programs: BTreeMap::new(),
            pmt_reasm: BTreeMap::new(),
            transport_stream_id: None,
            seen_pat_slot: false,
            emitted_pat: false,
        }
    }

    /// Extract payload and PUSI from a raw 188-byte packet, or `None` if it has no payload.
    fn ts_payload_and_pusi(packet: &[u8]) -> Option<(&[u8], bool)> {
        let header = TsHeader::parse(&packet[..4]).ok()?;
        let payload = extract_ts_payload(packet)?;
        Some((payload, header.pusi))
    }

    /// Extract the PID from a TS packet header (bytes 1-2).
    fn pid_from_packet(packet: &[u8]) -> u16 {
        (((packet[1] & 0x1F) as u16) << 8) | packet[2] as u16
    }

    /// The `table_id` a PUSI section payload begins with, if any.
    ///
    /// On a PUSI packet, `payload[0]` is the pointer_field; the section's
    /// `table_id` is the byte immediately after it.
    fn pusi_table_id(payload: &[u8]) -> Option<u8> {
        let pointer = *payload.first()? as usize;
        payload.get(1 + pointer).copied()
    }

    /// Observe the original PAT only to capture the transport_stream_id hint.
    fn observe_pat_tsid(&mut self, payload: &[u8], pusi: bool) {
        if self.transport_stream_id.is_some() {
            return;
        }
        let section = if pusi {
            let pointer = match payload.first() {
                Some(&p) => p as usize,
                None => return,
            };
            match payload.get(1 + pointer..) {
                Some(s) => s,
                None => return,
            }
        } else {
            payload
        };
        if let Ok(pat) = PatSection::parse(section) {
            self.transport_stream_id = Some(pat.transport_stream_id);
        }
    }

    /// Feed a candidate PID's payload into its reassembler and record any PMT.
    ///
    /// Returns `true` if a PMT was parsed for a `program_number` already in the
    /// map — i.e. the PMT cycle has wrapped, signalling that the mapping is
    /// complete (every program's PMT has been seen at least once).
    fn scan_pmt(&mut self, pid: u16, payload: &[u8], pusi: bool) -> bool {
        // Only start tracking a PID once we see a section beginning with the PMT
        // table_id (bounds memory: we don't reassemble every PID in the mux).
        let reasm = match self.pmt_reasm.entry(pid) {
            alloc::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
            alloc::collections::btree_map::Entry::Vacant(slot) => {
                if !pusi {
                    return false;
                }
                match Self::pusi_table_id(payload) {
                    Some(tid) if tid == pmt::TABLE_ID => {}
                    _ => return false,
                }
                slot.insert(SectionReassembler::default())
            }
        };
        reasm.feed(payload, pusi);

        let mut cycle_wrapped = false;
        while let Some(section) = reasm.pop_section() {
            if let Ok(p) = PmtSection::parse(&section) {
                // (program_number from the PMT body → the PID it arrived on).
                let prev = self.pmt_programs.insert(p.program_number, pid);
                if prev.is_some() {
                    cycle_wrapped = true;
                }
            }
        }
        cycle_wrapped
    }

    /// Build a regenerated PAT section from the PMT-derived mapping.
    ///
    /// Returns `None` if no programs have been discovered yet.
    fn rebuild_pat(&self) -> Option<Vec<u8>> {
        if self.pmt_programs.is_empty() {
            return None;
        }

        let entries: Vec<PatEntry> = self
            .pmt_programs
            .iter()
            .map(|(&program_number, &pmt_pid)| PatEntry {
                program_number,
                pid: pmt_pid,
            })
            .collect();

        let pat = PatSection {
            // transport_stream_id is not derivable from PMTs; use the hint from
            // the original PAT if one was seen, else 0.
            transport_stream_id: self.transport_stream_id.unwrap_or(0),
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            entries,
        };

        let mut buf = vec![0u8; pat.serialized_len()];
        pat.serialize_into(&mut buf).ok()?;
        Some(buf)
    }

    /// Emit a regenerated PAT (packetized onto PID 0x0000), if a mapping exists.
    fn emit_regen_pat(&mut self, out: &mut dyn FnMut(&[u8])) {
        if let Some(section) = self.rebuild_pat() {
            let mut packetizer = SectionPacketizer::new(PAT_PID);
            for pkt in packetizer.packetize(&[&section]) {
                out(&pkt);
            }
            self.emitted_pat = true;
        }
    }
}

impl Op for PsiRegenOp {
    fn process(&mut self, packet: &[u8], _model: &mut StreamModel, out: &mut dyn FnMut(&[u8])) {
        if packet.len() != TS_PACKET_SIZE {
            // Should not happen (engine validated), but be safe.
            out(packet);
            return;
        }

        let pid = Self::pid_from_packet(packet);

        // PAT slot: the stream carries a PAT cycle, so we replace each slot
        // in-position with a regenerated PAT (built from the PMT-derived
        // mapping) and drop the original.  A slot that arrives before any PMT
        // has been parsed has no mapping to emit yet and is simply dropped; the
        // next PAT slot (after the PMTs in the cycle) carries the full mapping.
        if pid == PAT_PID {
            self.seen_pat_slot = true;
            if let Some((payload, pusi)) = Self::ts_payload_and_pusi(packet) {
                // Use the original PAT only as a transport_stream_id hint.
                self.observe_pat_tsid(payload, pusi);
            }
            self.emit_regen_pat(out);
            return;
        }

        // Null packets pass through untouched (not a PMT candidate).
        if pid == NULL_PID {
            out(packet);
            return;
        }

        // Scan this PID for PMT sections (authoritative mapping source).
        if let Some((payload, pusi)) = Self::ts_payload_and_pusi(packet) {
            let cycle_wrapped = self.scan_pmt(pid, payload, pusi);
            // Stripped-PAT early-emit: when the stream carries NO PAT slot to
            // replace, emit the regenerated PAT as soon as the PMT cycle wraps
            // (every program's PMT has been seen → mapping complete), before
            // forwarding this packet so the PAT precedes the following data.
            if !self.seen_pat_slot && !self.emitted_pat && cycle_wrapped {
                self.emit_regen_pat(out);
            }
        }

        // Pass through the original non-PAT packet.
        out(packet);
    }

    fn flush(&mut self, _model: &mut StreamModel, out: &mut dyn FnMut(&[u8])) {
        // Fallback: if a mapping was discovered but no PAT was ever emitted
        // (e.g. the stream was stripped and the PMT cycle never wrapped within
        // the capture), emit one now so the regenerated PAT is never lost.
        if !self.emitted_pat {
            self.emit_regen_pat(out);
        }
    }
}
