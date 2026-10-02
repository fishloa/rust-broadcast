//! PID filter / service extract operation.
//!
//! Filters a TS to a configured set of PIDs.  Two modes:
//!
//! - **Keep-set** ([`PidFilter::keep`]) — pass only packets whose PID is in
//!   the caller-supplied set (PAT PID 0x0000 is always added automatically).
//! - **Service extract** ([`PidFilter::service`]) — observe the PAT to find
//!   the PMT PID for the requested program_number, then observe that PMT to
//!   collect the PCR PID, all ES PIDs and all Conditional Access PIDs (the
//!   ECM PIDs from `CA_descriptor`s in the PMT's program-info and ES-info
//!   loops, plus the CAT PID 0x0001 and the EMM PIDs the CAT references);
//!   keep `{0x0000, 0x0001, …cat_emm_pids, pmt_pid, pcr_pid, …es_pids,
//!   …ca_pids}` and drop everything else.  Dropping the ECM/EMM PIDs of a
//!   scrambled service would produce an undecryptable output (#1101).
//!
//! The op is **stateful**: in service-extract mode the keep-set is initially
//! unknown.  While waiting for the PAT and PMT the op passes all PSI PIDs
//! through unchanged (conservative: avoids dropping PAT/PMT packets that carry
//! the metadata it needs), and buffers no non-PSI packets.  After resolution
//! the PAT and PMT keep being re-observed on **version change**: a new PAT
//! version re-resolves the PMT PID, and a new PMT version re-resolves the ES,
//! PCR and CA PIDs, so ES PIDs added or moved mid-stream are not silently
//! dropped for the rest of the stream (#1101).
//!
//! # Spec
//!
//! ISO/IEC 13818-1 (= ITU-T H.222.0) §2.4.4.3 (PAT) / §2.4.4.6 (CAT) /
//! §2.4.4.8 (PMT) / §2.6.16 (CA_descriptor).

use alloc::collections::BTreeSet;

use broadcast_common::traits::Parse;
use dvb_si::descriptors::ca::CaDescriptor;
use dvb_si::descriptors::{AnyDescriptor, parse_loop};
use dvb_si::tables::cat::CatSection;
use dvb_si::tables::pat::PatSection;
use dvb_si::tables::pmt::PmtSection;
use mpeg_ts::ts::{SectionReassembler, TS_PACKET_SIZE, TsHeader, extract_ts_payload};

use crate::ops::{Op, StreamModel};

/// PAT well-known PID (ISO/IEC 13818-1 §2.4.4.3).
const PAT_PID: u16 = 0x0000;
/// CAT well-known PID (ISO/IEC 13818-1 §2.4.4.6).
const CAT_PID: u16 = 0x0001;
/// Null-packet PID (ISO/IEC 13818-1 §2.4.1).
const NULL_PID: u16 = 0x1FFF;

/// Configuration for [`TsFixBuilder::filter_pids`](crate::TsFixBuilder::filter_pids).
///
/// `#[non_exhaustive]` — future modes may be added without a breaking change.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum PidFilter {
    /// Keep only packets whose PID is in `pids` (PAT PID 0x0000 is always added).
    ///
    /// Constructed via [`PidFilter::keep`].
    Keep {
        /// The set of PIDs to retain.
        pids: BTreeSet<u16>,
    },

    /// Extract one programme: resolve its PMT PID via the PAT, then keep
    /// `{PAT, CAT, pmt_pid, pcr_pid, …es_pids, …ca_pids}` and drop all other
    /// PIDs.
    ///
    /// Constructed via [`PidFilter::service`].
    Service {
        /// `program_number` to extract (as signalled in the PAT).
        program_number: u16,
    },
}

impl PidFilter {
    /// Build a keep-set filter.
    ///
    /// PAT PID 0x0000 is always implicitly included regardless of the supplied
    /// set — the PAT must be preserved for any downstream demuxer to work.
    ///
    /// # Example
    /// ```
    /// use ts_fix::PidFilter;
    /// let cfg = PidFilter::keep([0x0101, 0x0102]);
    /// ```
    pub fn keep(pids: impl IntoIterator<Item = u16>) -> Self {
        let mut set: BTreeSet<u16> = pids.into_iter().collect();
        set.insert(PAT_PID);
        Self::Keep { pids: set }
    }

    /// Build a service-extract filter.
    ///
    /// The engine will observe the live PAT/PMT to discover the program's PIDs
    /// and then drop everything else.
    ///
    /// # Example
    /// ```
    /// use ts_fix::PidFilter;
    /// let cfg = PidFilter::service(1);
    /// ```
    pub fn service(program_number: u16) -> Self {
        Self::Service { program_number }
    }
}

// ── Internal state machine ───────────────────────────────────────────────────

/// State for service-extract mode.
///
/// PSI sections (PAT, PMT) are reassembled with the canonical
/// [`mpeg_ts::ts::SectionReassembler`] rather than a bespoke buffer — it
/// handles pointer_field, multi-packet sections, and multiple sections per
/// payload correctly and is the better-tested code path.
enum ServiceState {
    /// Waiting to see the PAT; we know which program_number we want.
    WaitingPat {
        program_number: u16,
        /// Reassembles PAT sections on PID 0x0000.
        pat_reasm: SectionReassembler,
    },
    /// PAT seen; waiting for the PMT on `pmt_pid`.
    WaitingPmt {
        program_number: u16,
        pmt_pid: u16,
        /// Reassembles PMT sections on `pmt_pid`.
        pmt_reasm: SectionReassembler,
    },
    /// PMT seen; keep-set fully resolved.  The PAT and PMT (and, when the
    /// keep-set references CA PIDs, the CAT) keep being observed so PAT/PMT
    /// version changes re-resolve the keep-set.
    Resolved {
        program_number: u16,
        pmt_pid: u16,
        /// Last observed PAT `version_number` (ISO/IEC 13818-1 §2.4.4.3).
        pat_version: u8,
        /// Last observed PMT `version_number` (ISO/IEC 13818-1 §2.4.4.8).
        pmt_version: u8,
        /// Last observed CAT `version_number`, or `None` until a CAT section
        /// has been parsed (ISO/IEC 13818-1 §2.4.4.6).
        cat_version: Option<u8>,
        /// Reassembles PAT sections on PID 0x0000.
        pat_reasm: SectionReassembler,
        /// Reassembles PMT sections on `pmt_pid`.
        pmt_reasm: SectionReassembler,
        /// Reassembles CAT sections on PID 0x0001.
        cat_reasm: SectionReassembler,
        keep: BTreeSet<u16>,
    },
}

/// Extract `(payload, pusi)` from a raw 188-byte packet, or `None` if it has
/// no payload. Payload extraction defers to [`mpeg_ts::ts::extract_ts_payload`]
/// (handles the adaptation-field offset); PUSI comes from the parsed header.
fn ts_payload_and_pusi(packet: &[u8]) -> Option<(&[u8], bool)> {
    let header = TsHeader::parse(packet).ok()?;
    let payload = extract_ts_payload(packet)?;
    Some((payload, header.pusi))
}

/// PID of a TS packet header (13-bit field, ISO/IEC 13818-1 §2.4.3.3).
///
/// Parsed through [`TsHeader::parse`] so a short packet is `None`, not an
/// out-of-bounds index, and the PID mask lives in one place.
fn pid_of(packet: &[u8]) -> Option<u16> {
    TsHeader::parse(packet).ok().map(|h| h.pid)
}

/// Collect every `ca_pid` from the CA_descriptors (tag 0x09, ISO/IEC 13818-1
/// §2.6.16) in a descriptor loop.  A malformed loop is skipped rather than
/// failing the whole PMT — the ES/PCR PIDs are still valid.
fn ca_pids_in_loop(raw: &[u8]) -> BTreeSet<u16> {
    let mut pids = BTreeSet::new();
    for desc in parse_loop(raw) {
        if let Ok(AnyDescriptor::Ca(CaDescriptor { ca_pid, .. })) = desc {
            pids.insert(ca_pid);
        }
    }
    pids
}

/// Build the service-extract keep-set from a resolved PMT (plus the EMM PIDs
/// discovered from the CAT, if one has been parsed).
fn resolve_keep_set(
    pmt: &PmtSection<'_>,
    pmt_pid: u16,
    cat_emm_pids: &BTreeSet<u16>,
) -> BTreeSet<u16> {
    let mut keep = BTreeSet::new();
    keep.insert(PAT_PID);
    keep.insert(pmt_pid);
    keep.insert(pmt.pcr_pid);
    for stream in &pmt.streams {
        keep.insert(stream.elementary_pid);
    }
    // ECM PIDs from the PMT program-info loop (§2.4.4.8 / §2.6.16).
    let mut has_ca = false;
    for pid in ca_pids_in_loop(pmt.program_info.raw()) {
        keep.insert(pid);
        has_ca = true;
    }
    // ECM PIDs from each ES's ES-info loop.
    for stream in &pmt.streams {
        for pid in ca_pids_in_loop(stream.es_info.raw()) {
            keep.insert(pid);
            has_ca = true;
        }
    }
    // If the service carries CA descriptors, the CAT (PID 0x0001) and the
    // EMM PIDs it references are required to decrypt (#1101).
    if has_ca || !cat_emm_pids.is_empty() {
        keep.insert(CAT_PID);
        keep.extend(cat_emm_pids.iter().copied());
    }
    keep
}

// ── The operation ────────────────────────────────────────────────────────────

/// PID filter / service-extract operation.
pub(crate) struct PidFilterOp {
    /// Current filter state.
    state: FilterState,
}

enum FilterState {
    /// Keep exactly this set of PIDs.
    KeepSet(BTreeSet<u16>),
    /// Service extract — stateful.
    Service(ServiceState),
}

impl PidFilterOp {
    pub(crate) fn new(cfg: PidFilter) -> Self {
        let state = match cfg {
            PidFilter::Keep { pids } => FilterState::KeepSet(pids),
            PidFilter::Service { program_number } => {
                FilterState::Service(ServiceState::WaitingPat {
                    program_number,
                    pat_reasm: SectionReassembler::default(),
                })
            }
        };
        Self { state }
    }

    /// Decide whether a packet on `pid` should pass the filter.
    fn should_keep(&self, pid: u16) -> bool {
        match &self.state {
            FilterState::KeepSet(set) => set.contains(&pid),
            FilterState::Service(svc_state) => match svc_state {
                ServiceState::WaitingPat { .. } => {
                    // Before PAT seen: only let PAT through.
                    pid == PAT_PID
                }
                ServiceState::WaitingPmt { pmt_pid, .. } => {
                    // PAT seen but PMT not yet: let PAT + target PMT PID through.
                    pid == PAT_PID || pid == *pmt_pid
                }
                ServiceState::Resolved { keep, .. } => keep.contains(&pid),
            },
        }
    }

    /// Observe a packet and potentially advance the service-extract state machine.
    fn observe(&mut self, packet: &[u8]) {
        let state = match &mut self.state {
            FilterState::KeepSet(_) => return,
            FilterState::Service(s) => s,
        };
        let Some(pid) = pid_of(packet) else {
            return;
        };

        match state {
            ServiceState::WaitingPat {
                program_number,
                pat_reasm,
            } => {
                // Listen on PID 0x0000 for the PAT.
                if pid != PAT_PID {
                    return;
                }
                let Some((payload, pusi)) = ts_payload_and_pusi(packet) else {
                    return;
                };
                pat_reasm.feed(payload, pusi);

                let pn = *program_number;
                while let Some(section) = pat_reasm.pop_section() {
                    let Ok(pat) = PatSection::parse(&section) else {
                        continue;
                    };
                    // Find the PMT PID for our program_number.
                    if let Some(entry) = pat.entries.iter().find(|e| e.program_number == pn) {
                        let pmt_pid = entry.pid;
                        *state = ServiceState::WaitingPmt {
                            program_number: pn,
                            pmt_pid,
                            pmt_reasm: SectionReassembler::default(),
                        };
                        return;
                    }
                }
            }

            ServiceState::WaitingPmt {
                program_number,
                pmt_pid,
                pmt_reasm,
            } => {
                if pid != *pmt_pid {
                    return;
                }
                let Some((payload, pusi)) = ts_payload_and_pusi(packet) else {
                    return;
                };
                pmt_reasm.feed(payload, pusi);

                let (pn, pmt_pid) = (*program_number, *pmt_pid);
                while let Some(section) = pmt_reasm.pop_section() {
                    let Ok(pmt) = PmtSection::parse(&section) else {
                        continue;
                    };
                    let keep = resolve_keep_set(&pmt, pmt_pid, &BTreeSet::new());
                    *state = ServiceState::Resolved {
                        program_number: pn,
                        pmt_pid,
                        pat_version: 0,
                        pmt_version: pmt.version_number,
                        cat_version: None,
                        pat_reasm: SectionReassembler::default(),
                        pmt_reasm: SectionReassembler::default(),
                        cat_reasm: SectionReassembler::default(),
                        keep,
                    };
                    return;
                }
            }

            ServiceState::Resolved {
                program_number,
                pmt_pid,
                pat_version,
                pmt_version,
                cat_version,
                pat_reasm,
                pmt_reasm,
                cat_reasm,
                keep,
            } => {
                if pid == *pmt_pid {
                    let Some((payload, pusi)) = ts_payload_and_pusi(packet) else {
                        return;
                    };
                    pmt_reasm.feed(payload, pusi);
                    while let Some(section) = pmt_reasm.pop_section() {
                        let Ok(pmt) = PmtSection::parse(&section) else {
                            continue;
                        };
                        // The PMT that initially resolved the state is the
                        // reference version; only a *different* version is a
                        // change (§2.4.4.8).
                        if pmt.version_number == *pmt_version {
                            continue;
                        }
                        // PMT version change: re-resolve the keep-set,
                        // preserving any EMM PIDs already learned from the CAT.
                        *pmt_version = pmt.version_number;
                        let mut emm_pids = BTreeSet::new();
                        for p in keep.iter() {
                            if *p != PAT_PID && *p != CAT_PID && *p != *pmt_pid {
                                emm_pids.insert(*p);
                            }
                        }
                        *keep = resolve_keep_set(&pmt, *pmt_pid, &emm_pids);
                    }
                } else if pid == PAT_PID {
                    let Some((payload, pusi)) = ts_payload_and_pusi(packet) else {
                        return;
                    };
                    pat_reasm.feed(payload, pusi);
                    while let Some(section) = pat_reasm.pop_section() {
                        let Ok(pat) = PatSection::parse(&section) else {
                            continue;
                        };
                        // The PAT version observed when the state resolved is
                        // the reference; only a different version is a change
                        // (§2.4.4.3).
                        if pat.version_number == *pat_version {
                            continue;
                        }
                        *pat_version = pat.version_number;
                        if let Some(entry) = pat
                            .entries
                            .iter()
                            .find(|e| e.program_number == *program_number)
                            && entry.pid != *pmt_pid
                        {
                            // The PMT moved to a new PID: track the new
                            // one and drop the old PMT PID from the keep-set.
                            keep.remove(pmt_pid);
                            *pmt_pid = entry.pid;
                            *pmt_version = 0;
                            keep.insert(entry.pid);
                        }
                    }
                } else if pid == CAT_PID && keep.contains(&CAT_PID) {
                    let Some((payload, pusi)) = ts_payload_and_pusi(packet) else {
                        return;
                    };
                    cat_reasm.feed(payload, pusi);
                    while let Some(section) = cat_reasm.pop_section() {
                        let Ok(cat) = CatSection::parse(&section) else {
                            continue;
                        };
                        if cat_version.is_some_and(|v| v == cat.version_number) {
                            continue;
                        }
                        *cat_version = Some(cat.version_number);
                        // Add the CAT's EMM PIDs to the keep-set (ISO/IEC
                        // 13818-1 §2.4.4.6 — the CA_descriptor's ca_pid here
                        // is an EMM PID).
                        keep.insert(CAT_PID);
                        if let Ok(entries) = cat.ca_descriptors() {
                            keep.extend(entries.iter().map(|e| e.ca_pid));
                        }
                    }
                }
            }
        }
    }
}

impl Op for PidFilterOp {
    fn process(&mut self, packet: &[u8], _model: &mut StreamModel, out: &mut dyn FnMut(&[u8])) {
        if packet.len() != TS_PACKET_SIZE {
            // Should not happen (engine validated), but be safe.
            out(packet);
            return;
        }

        // Extract PID before potential state mutation.
        let Some(pid) = pid_of(packet) else {
            out(packet);
            return;
        };

        // Always skip null packets.
        if pid == NULL_PID {
            return;
        }

        // Advance the service-extract state machine by observing this packet.
        self.observe(packet);

        if self.should_keep(pid) {
            out(packet);
        }
    }

    fn flush(&mut self, _model: &mut StreamModel, _out: &mut dyn FnMut(&[u8])) {
        // Nothing buffered.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_helpers_never_index_past_a_short_packet() {
        for len in 0..4 {
            let short = [0x47u8, 0x01, 0x02, 0x10];
            assert_eq!(pid_of(&short[..len]), None, "len {len}");
            assert_eq!(ts_payload_and_pusi(&short[..len]), None, "len {len}");
        }
        // PID is the 13-bit field: byte1 low 5 bits (the top 3 flag bits are
        // ignored) and byte 2.
        assert_eq!(pid_of(&[0x47, 0xFF, 0xFF, 0x10]), Some(0x1FFF));
        assert_eq!(pid_of(&[0x47, 0x41, 0x23, 0x10]), Some(0x0123));
    }
}
