//! PID filter / service extract operation.
//!
//! Filters a TS to a configured set of PIDs.  Two modes:
//!
//! - **Keep-set** ([`PidFilter::keep`]) — pass only packets whose PID is in
//!   the caller-supplied set (PAT PID 0x0000 is always added automatically).
//! - **Service extract** ([`PidFilter::service`]) — observe the PAT to find
//!   the PMT PID for the requested program_number, then observe that PMT to
//!   collect the PCR PID and all ES PIDs; keep exactly
//!   `{0x0000, pmt_pid, pcr_pid, …es_pids}` and drop everything else.
//!
//! The op is **stateful**: in service-extract mode the keep-set is initially
//! unknown.  While waiting for the PAT and PMT the op passes all PSI PIDs
//! through unchanged (conservative: avoids dropping PAT/PMT packets that carry
//! the metadata it needs), and buffers no non-PSI packets.
//!
//! # Spec
//!
//! ISO/IEC 13818-1 (= ITU-T H.222.0) §2.4.4.3 (PAT) / §2.4.4.8 (PMT).

use alloc::collections::BTreeSet;

use broadcast_common::traits::Parse;
use dvb_si::tables::pat::PatSection;
use dvb_si::tables::pmt::PmtSection;
use mpeg_ts::ts::{SectionReassembler, TS_PACKET_SIZE, TsHeader, extract_ts_payload};

use crate::ops::{Op, StreamModel};

/// PAT well-known PID (ISO/IEC 13818-1 §2.4.4.3).
const PAT_PID: u16 = 0x0000;
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
    /// `{PAT, pmt_pid, pcr_pid, …es_pids}` and drop all other PIDs.
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
        pmt_pid: u16,
        /// Reassembles PMT sections on `pmt_pid`.
        pmt_reasm: SectionReassembler,
    },
    /// PMT seen; keep-set fully resolved.
    Resolved { keep: BTreeSet<u16> },
}

/// Extract `(payload, pusi)` from a raw 188-byte packet, or `None` if it has
/// no payload. Payload extraction defers to [`mpeg_ts::ts::extract_ts_payload`]
/// (handles the adaptation-field offset); PUSI comes from the parsed header.
fn ts_payload_and_pusi(packet: &[u8]) -> Option<(&[u8], bool)> {
    let header = TsHeader::parse(&packet[..4]).ok()?;
    let payload = extract_ts_payload(packet)?;
    Some((payload, header.pusi))
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
                ServiceState::Resolved { keep } => keep.contains(&pid),
            },
        }
    }

    /// Observe a packet and potentially advance the service-extract state machine.
    fn observe(&mut self, packet: &[u8]) {
        let state = match &mut self.state {
            FilterState::KeepSet(_) => return,
            FilterState::Service(s) => s,
        };

        match state {
            ServiceState::WaitingPat {
                program_number,
                pat_reasm,
            } => {
                // Listen on PID 0x0000 for the PAT.
                let pid = (((packet[1] & 0x1F) as u16) << 8) | packet[2] as u16;
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
                            pmt_pid,
                            pmt_reasm: SectionReassembler::default(),
                        };
                        return;
                    }
                }
            }

            ServiceState::WaitingPmt { pmt_pid, pmt_reasm } => {
                let pid = (((packet[1] & 0x1F) as u16) << 8) | packet[2] as u16;
                if pid != *pmt_pid {
                    return;
                }
                let Some((payload, pusi)) = ts_payload_and_pusi(packet) else {
                    return;
                };
                pmt_reasm.feed(payload, pusi);

                let pmt_pid = *pmt_pid;
                while let Some(section) = pmt_reasm.pop_section() {
                    let Ok(pmt) = PmtSection::parse(&section) else {
                        continue;
                    };
                    // Resolve the keep-set.
                    let mut keep = BTreeSet::new();
                    keep.insert(PAT_PID);
                    keep.insert(pmt_pid);
                    keep.insert(pmt.pcr_pid);
                    for stream in &pmt.streams {
                        keep.insert(stream.elementary_pid);
                    }
                    *state = ServiceState::Resolved { keep };
                    return;
                }
            }

            ServiceState::Resolved { .. } => {
                // Nothing more to observe.
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
        let pid = (((packet[1] & 0x1F) as u16) << 8) | packet[2] as u16;

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
