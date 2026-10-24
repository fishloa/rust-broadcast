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
/// 30-stream multiplex (`ts/france-tnt-uhf32-video-head-6MB.ts`); the defaults in
/// [`crate::events`] leave roughly 1.7x headroom over the worst case and stay below the 5 s stall
/// notice. The quiet limit is input bytes without any further stream resolving after which the
/// hold gives up on the streams still unresolved (scrambled or mis-framed, or sparse data that
/// never completes an access unit): the longest gap between two resolutions on the committed
/// captures is 0.46 MB (`ts/france2.ts`) and 0.92 MB between the first two bursts of the
/// 30-stream multiplex, which then announces in two steps; `csa/csa2-plain.ts` resolves nothing
/// for 0.56 MB.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HoldLimits {
    pub(crate) max_input_bytes: usize,
    pub(crate) quiet_input_bytes: usize,
    pub(crate) max_media_us: u64,
    pub(crate) max_wall_us: u64,
}

impl HoldLimits {
    pub(crate) fn from_config(config: &crate::events::Config) -> Self {
        Self {
            max_input_bytes: config.tracks_hold_max_input_bytes,
            quiet_input_bytes: config.tracks_hold_quiet_input_bytes,
            max_media_us: config.tracks_hold_max_media_us,
            max_wall_us: config.tracks_hold_max_wall_us,
        }
    }

    pub(crate) fn disabled(&self) -> bool {
        self.max_media_us == 0 || self.max_wall_us == 0
    }

    pub(crate) fn reached(
        &self,
        input_bytes: usize,
        quiet_bytes: usize,
        media_us: u64,
        wall_us: u64,
    ) -> bool {
        input_bytes >= self.max_input_bytes
            || quiet_bytes >= self.quiet_input_bytes
            || media_us >= self.max_media_us
            || wall_us >= self.max_wall_us
    }
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

    const LIMITS: HoldLimits = HoldLimits {
        max_input_bytes: 4096,
        quiet_input_bytes: 512,
        max_media_us: 3_000,
        max_wall_us: 2_000,
    };

    #[test]
    fn each_limit_fires_alone_at_its_boundary() {
        let l = LIMITS;
        assert!(!l.reached(0, 0, 0, 0));
        assert!(!l.reached(l.max_input_bytes - 1, 0, 0, 0));
        assert!(l.reached(l.max_input_bytes, 0, 0, 0));
        assert!(!l.reached(0, l.quiet_input_bytes - 1, 0, 0));
        assert!(l.reached(0, l.quiet_input_bytes, 0, 0));
        assert!(!l.reached(0, 0, l.max_media_us - 1, 0));
        assert!(l.reached(0, 0, l.max_media_us, 0));
        assert!(!l.reached(0, 0, 0, l.max_wall_us - 1));
        assert!(l.reached(0, 0, 0, l.max_wall_us));
    }

    #[test]
    fn a_zero_wall_or_media_limit_disables_the_hold() {
        assert!(!LIMITS.disabled());
        assert!(
            HoldLimits {
                max_wall_us: 0,
                ..LIMITS
            }
            .disabled()
        );
        assert!(
            HoldLimits {
                max_media_us: 0,
                ..LIMITS
            }
            .disabled()
        );
    }
}
