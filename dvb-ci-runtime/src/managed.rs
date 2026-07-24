//! Managed CAS-layer state (#763) — the [`Driver`](crate::Driver)'s owned view
//! of the slot's active descrambled-service set.
//!
//! This is Layer 1 of the #763 CAS orchestration design
//! (`docs/superpowers/specs/2026-07-24-dvb-ci-cas-layer-design.md`): parsed
//! `dvb-si` structures in (never raw bytes), the existing
//! `dvb_ci::builder::build_ca_pmt` PMT→`ca_pmt` projection
//! (ETSI EN 50221 §8.4.3.4, Table 25) does the wire work, and this module just
//! tracks what was sent. [`Driver::add_service`](crate::driver::Driver::add_service)
//! builds + sends the `ca_pmt` and records the service here.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use dvb_ci::objects::ca_pmt::CaPmtCmdId;
use dvb_ci::objects::ca_pmt_reply::CaEnable;
use dvb_si::descriptors::DescriptorLoop;
use dvb_si::descriptors::ca::TAG as CA_DESCRIPTOR_TAG;
use dvb_si::tables::cat::CatCaEntry;
use dvb_si::tables::pmt::PmtSection;

/// Default entitlement re-query cadence (#763 Task 5's `Resource::tick`-driven
/// refresh; `Duration::ZERO` disables it). Set on [`ManagedCa::new`] so the
/// field is in place before the re-query timer is wired up.
pub const REQUERY_DEFAULT: Duration = Duration::from_secs(10);

/// Errors from the managed CAS-layer API (`Driver::add_service` and friends,
/// #763).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CaError {
    /// The PMT carries no `CA_descriptor` (ETSI EN 300 468 §6.2.16, tag
    /// `0x09`) at programme or elementary-stream level — there is nothing for
    /// the CAM to descramble, so no `ca_pmt` is built or sent.
    #[error("PMT for program_number {program_number} has no CA_descriptor at program or ES level")]
    NoCaDescriptor {
        /// The programme whose PMT carried no CA info.
        program_number: u16,
    },
    /// Sending the built `ca_pmt` to the device failed.
    #[error("ca_pmt send failed: {0}")]
    Io(#[from] std::io::Error),
    /// The CAT's descriptor loop (ISO/IEC 13818-1 §2.4.4.5) carried a
    /// truncated `CA_descriptor` (EN 300 468 §6.2.16) — [`Driver::set_cat`](crate::driver::Driver::set_cat)
    /// could not extract the CAID/EMM-PID map.
    #[error("CAT CA_descriptor parse failed: {0}")]
    Cat(#[from] dvb_si::error::Error),
}

/// One actively-managed service (owned, no borrowed lifetime — copied out of
/// the caller's `PmtSection` at [`Driver::add_service`](crate::driver::Driver::add_service)
/// time).
///
/// `cmd` and `last_ca_enable` are recorded starting now but are only *read* by
/// the Task 5 re-query/edge-event logic and Task 6's `remove_service`; this
/// baseline (Task 3) writes them so that state is ready when those land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedService {
    /// Elementary-stream PIDs carried by this programme (every stream, not
    /// only the CA-bearing ones — a caller routing PIDs into `ci0` needs the
    /// full component set).
    pub es_pids: Vec<u16>,
    /// `CA_PID`s (ECM PIDs) advertised by this programme's `CA_descriptor`s,
    /// programme- and ES-level combined.
    pub ca_pids: Vec<u16>,
    /// The `ca_pmt_cmd_id` last sent for this service (EN 50221 §8.4.3.4
    /// Table 25).
    pub cmd: CaPmtCmdId,
    /// The last observed programme-level `CA_enable` (EN 50221 §8.4.3.5 Table
    /// 26), for the Task 5 edge-triggered `Notification::Entitlement`. `None`
    /// until a `ca_pmt_reply` has been seen for this programme.
    pub last_ca_enable: Option<CaEnable>,
}

/// The [`Driver`](crate::Driver)'s owned CAS-layer state (#763 Layer 1) — one
/// CI slot's active service set plus the entitlement re-query cadence.
#[derive(Debug, Clone)]
pub struct ManagedCa {
    /// Active services, keyed by `program_number`.
    services: BTreeMap<u16, ManagedService>,
    /// Entitlement re-query cadence (Task 5); `Duration::ZERO` disables it.
    requery_interval: Duration,
    /// Elapsed time accumulated since the last re-query. Landed now (per the
    /// #763 plan's `ManagedCa` shape) but only read once Task 5 wires up the
    /// tick-driven re-query.
    #[allow(dead_code)]
    since: Duration,
    /// The last `set_cat`'s CAID → EMM PID map (ISO/IEC 13818-1 §2.4.4.5's
    /// `CA_descriptor`s, EN 300 468 §6.2.16). Kept even when it yields no
    /// `emm_pids` (no `ca_info` seen yet) so a later `ca_info` can recompute
    /// against it (#763 Task 4).
    cat_emm_pids: BTreeMap<u16, u16>,
    /// The last-observed `Notification::CaInfo` CAID set — the CAM's
    /// advertised systems (#763 Task 4).
    cam_caids: BTreeSet<u16>,
    /// `cat_emm_pids` ∩ `cam_caids` — the EMM PIDs to route into `ci0`.
    /// Recomputed on every [`set_cat`](Self::set_cat)/
    /// [`set_cam_caids`](Self::set_cam_caids) call.
    emm_pids: Vec<u16>,
    /// Union of active services' ES PIDs. Recomputed on every
    /// [`record`](Self::record) call.
    descramble_pids: Vec<u16>,
}

impl Default for ManagedCa {
    fn default() -> Self {
        Self {
            services: BTreeMap::new(),
            requery_interval: REQUERY_DEFAULT,
            since: Duration::ZERO,
            cat_emm_pids: BTreeMap::new(),
            cam_caids: BTreeSet::new(),
            emm_pids: Vec::new(),
            descramble_pids: Vec::new(),
        }
    }
}

impl ManagedCa {
    /// New, empty managed-CA state at the default re-query cadence
    /// ([`REQUERY_DEFAULT`]).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The currently-tracked services, keyed by `program_number`.
    #[must_use]
    pub fn services(&self) -> &BTreeMap<u16, ManagedService> {
        &self.services
    }

    /// The re-query cadence currently configured (Task 5 consumes this).
    #[must_use]
    pub fn requery_interval(&self) -> Duration {
        self.requery_interval
    }

    /// Whether no service is currently tracked — used to pick
    /// `CaPmtListManagement::Only` (first-ever service) vs `Add` (joining an
    /// already-active set) for the next `add_service`, mirroring
    /// [`Driver::descramble_programs`](crate::driver::Driver::descramble_programs)/
    /// [`Driver::add_program`](crate::driver::Driver::add_program)'s existing
    /// multi-programme list-management convention (EN 50221 §8.4.3.4 Table 25).
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.services.is_empty()
    }

    /// Record a service after its `ca_pmt` has been built and sent.
    pub(crate) fn record(&mut self, program_number: u16, service: ManagedService) {
        self.services.insert(program_number, service);
        self.recompute_descramble_pids();
    }

    /// The EMM PIDs to route into `ci0`: the last `set_cat`'s CAID → EMM-PID
    /// map, intersected with the CAM's advertised CAIDs (last `ca_info`) — a
    /// CAT entry for a CAID the CAM never advertised is never fed (#763 Task
    /// 4).
    #[must_use]
    pub fn emm_pids(&self) -> &[u16] {
        &self.emm_pids
    }

    /// The union of every actively-managed service's elementary-stream PIDs
    /// — the PIDs a caller must route into `ci0` for descrambling.
    #[must_use]
    pub fn descramble_pids(&self) -> &[u16] {
        &self.descramble_pids
    }

    /// Store the CAT's CAID → EMM-PID map (ISO/IEC 13818-1 §2.4.4.5's
    /// `CA_descriptor`s, EN 300 468 §6.2.16) and recompute [`emm_pids`](Self::emm_pids).
    /// Calling this before any `ca_info` is not an error: the map is kept so
    /// a later [`set_cam_caids`](Self::set_cam_caids) recomputes against it.
    pub(crate) fn set_cat(&mut self, entries: &[CatCaEntry]) {
        self.cat_emm_pids = entries.iter().map(|e| (e.ca_system_id, e.ca_pid)).collect();
        self.recompute_emm_pids();
    }

    /// Record the CAM's advertised CAID set (from `Notification::CaInfo`)
    /// and recompute [`emm_pids`](Self::emm_pids).
    pub(crate) fn set_cam_caids(&mut self, caids: BTreeSet<u16>) {
        self.cam_caids = caids;
        self.recompute_emm_pids();
    }

    /// `emm_pids` = `cat_emm_pids` ∩ `cam_caids`, in CAID order (the map's
    /// natural iteration order).
    fn recompute_emm_pids(&mut self) {
        self.emm_pids = self
            .cat_emm_pids
            .iter()
            .filter(|(caid, _)| self.cam_caids.contains(caid))
            .map(|(_, pid)| *pid)
            .collect();
    }

    /// `descramble_pids` = the union (dedup, sorted) of every active
    /// service's `es_pids`.
    fn recompute_descramble_pids(&mut self) {
        let mut pids: BTreeSet<u16> = BTreeSet::new();
        for service in self.services.values() {
            pids.extend(service.es_pids.iter().copied());
        }
        self.descramble_pids = pids.into_iter().collect();
    }
}

/// `true` if `loop_` carries at least one `CA_descriptor` (ISO/IEC 13818-1
/// §2.6.16 / ETSI EN 300 468 §6.2.16, tag `0x09`).
fn has_ca_descriptor(loop_: &DescriptorLoop<'_>) -> bool {
    loop_.raw_tags().any(|(tag, _)| tag == CA_DESCRIPTOR_TAG)
}

/// Byte offset within a `CA_descriptor` body (after tag + length) where the
/// `reserved(3)`/`CA_PID(13)` field starts — `CA_system_id` occupies the first
/// two bytes (ISO/IEC 13818-1 §2.6.16).
const CA_PID_BODY_OFFSET: usize = 2;
/// `CA_PID` field width in bytes.
const CA_PID_FIELD_LEN: usize = 2;
/// Mask for the `CA_PID`'s upper byte (top 3 bits are reserved, set to `1`).
const CA_PID_HIGH_MASK: u8 = 0x1F;

/// The `CA_PID` carried by one `CA_descriptor` body (tag + length already
/// stripped by [`DescriptorLoop::raw_tags`]), if the body is long enough to
/// carry the mandatory fields.
fn ca_pid_of(body: &[u8]) -> Option<u16> {
    let field = body.get(CA_PID_BODY_OFFSET..CA_PID_BODY_OFFSET + CA_PID_FIELD_LEN)?;
    Some((u16::from(field[0] & CA_PID_HIGH_MASK) << 8) | u16::from(field[1]))
}

/// Collect the `CA_PID`s from every `CA_descriptor` in `loop_`.
fn ca_pids_in(loop_: &DescriptorLoop<'_>) -> Vec<u16> {
    loop_
        .raw_tags()
        .filter(|(tag, _)| *tag == CA_DESCRIPTOR_TAG)
        .filter_map(|(_, body)| ca_pid_of(body))
        .collect()
}

/// Whether `pmt` carries a `CA_descriptor` at programme or any ES level (EN
/// 300 468 §6.2.16) — used to reject a CA-free PMT before building a useless
/// `ca_pmt`.
pub(crate) fn pmt_has_ca(pmt: &PmtSection<'_>) -> bool {
    has_ca_descriptor(&pmt.program_info)
        || pmt.streams.iter().any(|s| has_ca_descriptor(&s.es_info))
}

/// The owned [`ManagedService`] state to record for `pmt`, sent with `cmd`.
pub(crate) fn service_of(pmt: &PmtSection<'_>, cmd: CaPmtCmdId) -> ManagedService {
    let mut ca_pids = ca_pids_in(&pmt.program_info);
    for s in &pmt.streams {
        ca_pids.extend(ca_pids_in(&s.es_info));
    }
    ManagedService {
        es_pids: pmt.streams.iter().map(|s| s.elementary_pid).collect(),
        ca_pids,
        cmd,
        last_ca_enable: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_managed_ca_is_empty_at_default_cadence() {
        let m = ManagedCa::new();
        assert!(m.is_empty());
        assert!(m.services().is_empty());
        assert_eq!(m.requery_interval(), REQUERY_DEFAULT);
    }

    #[test]
    fn record_tracks_the_service() {
        let mut m = ManagedCa::new();
        let svc = ManagedService {
            es_pids: vec![0x100, 0x101],
            ca_pids: vec![0x0064],
            cmd: CaPmtCmdId::OkDescrambling,
            last_ca_enable: None,
        };
        m.record(7, svc.clone());
        assert!(!m.is_empty());
        assert_eq!(m.services().get(&7), Some(&svc));
    }

    #[test]
    fn ca_error_no_ca_descriptor_displays_program_number() {
        let e = CaError::NoCaDescriptor { program_number: 42 };
        assert!(e.to_string().contains("42"));
    }
}
