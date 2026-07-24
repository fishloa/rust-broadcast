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

use std::collections::BTreeMap;
use std::time::Duration;

use dvb_ci::objects::ca_pmt::CaPmtCmdId;
use dvb_ci::objects::ca_pmt_reply::CaEnable;
use dvb_si::descriptors::DescriptorLoop;
use dvb_si::descriptors::ca::TAG as CA_DESCRIPTOR_TAG;
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
}

impl Default for ManagedCa {
    fn default() -> Self {
        Self {
            services: BTreeMap::new(),
            requery_interval: REQUERY_DEFAULT,
            since: Duration::ZERO,
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
