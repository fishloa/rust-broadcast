//! The set of elementary streams a TS-over-HTTP connection's PAT/PMT declares, and the limits
//! that bound how long the first `Event::Tracks` waits for them to resolve (DESIGN.md, Audit
//! round 5 F1).

use alloc::collections::BTreeSet;

use dvb_si::demux::SiDemux;
use dvb_si::tables::AnyTableSection;
use mpeg_ts::pid::{Pid, well_known};
use mpeg_ts::resync::TsResync;

/// Limits on the initial-track hold, counted from the first media of a connection. Measured on
/// the committed captures, every declared stream had resolved within 0.76 s of media and 0.9 MB
/// for a single programme (`ts/france2.ts`, the TF1 capture) and within 1.76 s and 1.5 MB for a
/// 30-stream multiplex (`ts/france-tnt-uhf32-video-head-6MB.ts`); the media/byte/wall limits leave roughly 1.7x
/// headroom over the worst case and stay below the 5 s stall notice.
pub(crate) const HOLD_MAX_INPUT_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const HOLD_MAX_MEDIA_US: u64 = 3_000_000;
pub(crate) const HOLD_MAX_WALL_US: u64 = 3_000_000;

/// Input bytes without any further stream resolving after which the hold gives up on the
/// streams still unresolved (scrambled or mis-framed, or sparse data that never completes an
/// access unit). The longest gap between two resolutions on the committed captures is 0.46 MB
/// (`ts/france2.ts`) and 0.92 MB between the first two bursts of the 30-stream multiplex, which
/// then announces in two steps; `csa/csa2-plain.ts` resolves nothing for 0.56 MB.
pub(crate) const HOLD_QUIET_INPUT_BYTES: usize = 512 * 1024;

pub(crate) fn limit_reached(
    input_bytes: usize,
    quiet_bytes: usize,
    media_us: u64,
    wall_us: u64,
) -> bool {
    input_bytes >= HOLD_MAX_INPUT_BYTES
        || quiet_bytes >= HOLD_QUIET_INPUT_BYTES
        || media_us >= HOLD_MAX_MEDIA_US
        || wall_us >= HOLD_MAX_WALL_US
}

const NETWORK_PROGRAM_NUMBER: u16 = 0;

pub(crate) struct DeclaredStreams {
    resync: TsResync,
    si: SiDemux,
    programs: BTreeSet<u16>,
    seen_pmts: BTreeSet<u16>,
    declared: BTreeSet<Pid>,
}

impl DeclaredStreams {
    pub(crate) fn new() -> Self {
        Self {
            resync: TsResync::new(),
            si: SiDemux::builder()
                .dvb_si_pids(false)
                .pid(well_known::PAT)
                .build(),
            programs: BTreeSet::new(),
            seen_pmts: BTreeSet::new(),
            declared: BTreeSet::new(),
        }
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        for packet in self.resync.feed(bytes) {
            for event in self.si.feed(&packet) {
                match event.table_section() {
                    Ok(AnyTableSection::PatSection(pat)) => {
                        self.programs = pat
                            .entries
                            .iter()
                            .filter(|e| e.program_number != NETWORK_PROGRAM_NUMBER)
                            .map(|e| e.program_number)
                            .collect();
                    }
                    Ok(AnyTableSection::PmtSection(pmt)) => {
                        self.seen_pmts.insert(pmt.program_number);
                        self.declared
                            .extend(pmt.streams.iter().map(|s| s.elementary_pid));
                    }
                    _ => {}
                }
            }
        }
    }

    pub(crate) fn all_resolved(&self, resolved: &BTreeSet<Pid>) -> bool {
        !self.programs.is_empty()
            && self.programs.is_subset(&self.seen_pmts)
            && self.declared.is_subset(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_limit_fires_alone_at_its_boundary() {
        assert!(!limit_reached(0, 0, 0, 0));
        assert!(!limit_reached(HOLD_MAX_INPUT_BYTES - 1, 0, 0, 0));
        assert!(limit_reached(HOLD_MAX_INPUT_BYTES, 0, 0, 0));
        assert!(!limit_reached(0, HOLD_QUIET_INPUT_BYTES - 1, 0, 0));
        assert!(limit_reached(0, HOLD_QUIET_INPUT_BYTES, 0, 0));
        assert!(!limit_reached(0, 0, HOLD_MAX_MEDIA_US - 1, 0));
        assert!(limit_reached(0, 0, HOLD_MAX_MEDIA_US, 0));
        assert!(!limit_reached(0, 0, 0, HOLD_MAX_WALL_US - 1));
        assert!(limit_reached(0, 0, 0, HOLD_MAX_WALL_US));
    }
}
