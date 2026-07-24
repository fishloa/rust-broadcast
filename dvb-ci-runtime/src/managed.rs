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
/// Task 6's `remove_service`; `last_ca_enable`/`last_descrambling_ok` are also
/// read+written by `ManagedCa::record_reply` (#763 Task 5's edge-triggered
/// `Notification::Entitlement`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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
    /// until a `ca_pmt_reply` has been seen for this programme, or when the
    /// last-seen reply's programme `CA_enable_flag` was clear.
    pub last_ca_enable: Option<CaEnable>,
    /// The last observed `descrambling_ok` (derived from `last_ca_enable`),
    /// paired with it for the Task 5 transition diff.
    pub(crate) last_descrambling_ok: bool,
    /// The exact `ca_pmt` bytes sent by [`Driver::add_service`](crate::driver::Driver::add_service)
    /// to start descrambling — `cmd_id = ok_descrambling` (EN 50221 §8.4.3.4
    /// Table 25). Kept for the add_service oracle test to assert what was
    /// actually sent; per EN 50221 §8.4.3.5, `ok_descrambling` solicits **no**
    /// `ca_pmt_reply`, so this is *not* what the Task 5 re-query timer resends
    /// — see [`requery_ca_pmt`](Self::requery_ca_pmt).
    pub(crate) built_ca_pmt: Vec<u8>,
    /// The `ca_pmt` bytes built with `cmd_id = query` (same `list_management`
    /// as the initial send) for the Task 5 re-query timer to resend on its
    /// cadence: per EN 50221 §8.4.3.5, only `query` (or `ok_mmi`) elicits a
    /// fresh `ca_pmt_reply` from a conformant CAM — `ok_descrambling` does
    /// not — so re-sending `built_ca_pmt`'s bytes is not spec-guaranteed to
    /// produce one.
    pub(crate) requery_ca_pmt: Vec<u8>,
    /// The owned raw PMT section bytes this service was built from (#763
    /// Task 6), kept so [`Driver::remove_service`](crate::driver::Driver::remove_service)
    /// can re-drive the existing [`Driver::remove_program`](crate::driver::Driver::remove_program)
    /// path (which needs the raw PMT to build the `Update`/`NotSelected`
    /// `ca_pmt`, EN 50221 §8.4.3.4 Table 25) without the caller re-supplying
    /// it.
    pub(crate) pmt_raw: Vec<u8>,
}

/// The [`Driver`](crate::Driver)'s owned CAS-layer state (#763 Layer 1) — one
/// CI slot's active service set plus the entitlement re-query cadence.
#[derive(Debug, Clone)]
pub struct ManagedCa {
    /// Active services, keyed by `program_number`.
    services: BTreeMap<u16, ManagedService>,
    /// Entitlement re-query cadence (Task 5); `Duration::ZERO` disables it.
    requery_interval: Duration,
    /// Elapsed time accumulated since the last re-query (Task 5's
    /// [`tick`](Self::tick), mirroring `resource.rs`'s `DateTime::tick`
    /// accumulate-then-fire pattern).
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

    /// Set the entitlement re-query cadence
    /// ([`Driver::set_requery_interval`](crate::driver::Driver::set_requery_interval)).
    /// `Duration::ZERO` disables re-query. Resets the accumulated `since` so a
    /// newly-set interval doesn't fire immediately off stale accumulation.
    pub(crate) fn set_requery_interval(&mut self, interval: Duration) {
        self.requery_interval = interval;
        self.since = Duration::ZERO;
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

    /// Stop tracking `program_number` (#763 Task 6's
    /// [`Driver::remove_service`](crate::driver::Driver::remove_service)),
    /// recomputing [`descramble_pids`](Self::descramble_pids) afterwards.
    /// Returns whether the programme was actually tracked (`false` is a
    /// no-op — nothing to remove).
    pub(crate) fn remove(&mut self, program_number: u16) -> bool {
        let removed = self.services.remove(&program_number).is_some();
        if removed {
            self.recompute_descramble_pids();
        }
        removed
    }

    /// Clear all module-scoped managed state (#763 Task 6's CAM hot-plug
    /// fix): the active service set, the CAT/CAM CAID-derived EMM-PID state,
    /// and the descramble-PID union, plus the re-query accumulator (`since`)
    /// so a freshly (re)inserted module doesn't inherit a departed module's
    /// partially-elapsed re-query countdown. `requery_interval` is
    /// deliberately **not** reset — it is host configuration
    /// ([`set_requery_interval`](Self::set_requery_interval)), not
    /// per-module state, and must survive a CAM insert/remove edge.
    pub(crate) fn clear(&mut self) {
        self.services.clear();
        self.cat_emm_pids.clear();
        self.cam_caids.clear();
        self.emm_pids.clear();
        self.descramble_pids.clear();
        self.since = Duration::ZERO;
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

    /// `emm_pids` = `cat_emm_pids` ∩ `cam_caids`, deduped and sorted by PID
    /// (matching [`recompute_descramble_pids`](Self::recompute_descramble_pids)'s
    /// convention) — two CAIDs the CAT maps to the *same* EMM PID must not
    /// list that PID twice.
    fn recompute_emm_pids(&mut self) {
        let pids: BTreeSet<u16> = self
            .cat_emm_pids
            .iter()
            .filter(|(caid, _)| self.cam_caids.contains(caid))
            .map(|(_, pid)| *pid)
            .collect();
        self.emm_pids = pids.into_iter().collect();
    }

    /// Advance the entitlement re-query cadence by `elapsed` — mirrors
    /// `resource.rs`'s `DateTime::tick` accumulate-then-fire pattern:
    /// accumulate `since`, and once it reaches `requery_interval`, reset it
    /// and report that a re-query is due. Returns `false` (never fires) when
    /// re-query is disabled (`requery_interval == Duration::ZERO`) or there
    /// are no active services to re-query.
    pub(crate) fn tick(&mut self, elapsed: Duration) -> bool {
        if self.requery_interval.is_zero() || self.services.is_empty() {
            return false;
        }
        self.since += elapsed;
        if self.since >= self.requery_interval {
            self.since = Duration::ZERO;
            true
        } else {
            false
        }
    }

    /// Diff an incoming `ca_pmt_reply`'s programme-level status (EN 50221
    /// §8.4.3.5 Table 26) against the last-observed status for
    /// `program_number` and report the edge-triggered transition (#763 Task
    /// 5): `Some((v, descrambling_ok))` fires only when `ca_enable` is
    /// `Some(v)` *and* `(ca_enable, descrambling_ok)` differs from what was
    /// last observed for this programme — including the first-ever reply
    /// (no prior observation) establishing the baseline and reporting it.
    /// `ca_enable == None` (programme status withdrawn) never fires: there is
    /// no per-programme status to report (the coarse withdrawal signal is
    /// #726 `HotPlug`'s job). The last-observed status is updated
    /// unconditionally, whether or not this call fires.
    ///
    /// No-op (`None`) if `program_number` names no actively-managed service
    /// (never `add_service`'d, or already removed) — there is nothing to
    /// diff against.
    pub(crate) fn record_reply(
        &mut self,
        program_number: u16,
        ca_enable: Option<CaEnable>,
        descrambling_ok: bool,
    ) -> Option<(CaEnable, bool)> {
        let service = self.services.get_mut(&program_number)?;
        let prev = (service.last_ca_enable, service.last_descrambling_ok);
        service.last_ca_enable = ca_enable;
        service.last_descrambling_ok = descrambling_ok;
        match ca_enable {
            Some(v) if prev != (ca_enable, descrambling_ok) => Some((v, descrambling_ok)),
            _ => None,
        }
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

/// The owned [`ManagedService`] state to record for `pmt`, sent with `cmd`;
/// `built_ca_pmt` is the exact `ok_descrambling` bytes sent (kept for the
/// add_service oracle test) and `requery_ca_pmt` is the `query`-variant bytes
/// the Task 5 re-query timer resends (see the field docs on
/// [`ManagedService`]).
pub(crate) fn service_of(
    pmt: &PmtSection<'_>,
    cmd: CaPmtCmdId,
    built_ca_pmt: Vec<u8>,
    requery_ca_pmt: Vec<u8>,
    pmt_raw: Vec<u8>,
) -> ManagedService {
    let mut ca_pids = ca_pids_in(&pmt.program_info);
    for s in &pmt.streams {
        ca_pids.extend(ca_pids_in(&s.es_info));
    }
    ManagedService {
        es_pids: pmt.streams.iter().map(|s| s.elementary_pid).collect(),
        ca_pids,
        cmd,
        last_ca_enable: None,
        last_descrambling_ok: false,
        built_ca_pmt,
        requery_ca_pmt,
        pmt_raw,
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
            last_descrambling_ok: false,
            built_ca_pmt: vec![0xAA, 0xBB],
            requery_ca_pmt: vec![0xCC, 0xDD],
            pmt_raw: vec![0x02, 0x00],
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

    // --- #763 Task 5 ---

    #[test]
    fn set_requery_interval_updates_and_resets_accumulator() {
        let mut m = ManagedCa::new();
        assert_eq!(m.requery_interval(), REQUERY_DEFAULT);
        m.set_requery_interval(Duration::from_secs(3));
        assert_eq!(m.requery_interval(), Duration::from_secs(3));
    }

    #[test]
    fn tick_fires_once_interval_elapses_and_resets() {
        let mut m = ManagedCa::new();
        m.set_requery_interval(Duration::from_secs(5));
        m.record(
            1,
            ManagedService {
                es_pids: vec![0x100],
                ca_pids: vec![0x64],
                cmd: CaPmtCmdId::OkDescrambling,
                last_ca_enable: None,
                last_descrambling_ok: false,
                built_ca_pmt: vec![],
                requery_ca_pmt: vec![],
                pmt_raw: vec![],
            },
        );
        assert!(
            !m.tick(Duration::from_secs(3)),
            "before the interval: no fire"
        );
        assert!(
            m.tick(Duration::from_secs(3)),
            "crossing the interval: fires"
        );
        assert!(!m.tick(Duration::from_secs(1)), "since resets after firing");
    }

    #[test]
    fn tick_disabled_at_zero_interval_never_fires() {
        let mut m = ManagedCa::new();
        m.set_requery_interval(Duration::ZERO);
        m.record(
            1,
            ManagedService {
                es_pids: vec![0x100],
                ca_pids: vec![0x64],
                cmd: CaPmtCmdId::OkDescrambling,
                last_ca_enable: None,
                last_descrambling_ok: false,
                built_ca_pmt: vec![],
                requery_ca_pmt: vec![],
                pmt_raw: vec![],
            },
        );
        assert!(!m.tick(Duration::from_secs(1000)));
    }

    #[test]
    fn tick_with_no_active_services_never_fires() {
        let mut m = ManagedCa::new();
        m.set_requery_interval(Duration::from_secs(1));
        assert!(!m.tick(Duration::from_secs(1000)));
    }

    #[test]
    fn record_reply_first_ever_some_establishes_baseline_and_reports() {
        let mut m = ManagedCa::new();
        m.record(
            1,
            ManagedService {
                es_pids: vec![0x100],
                ca_pids: vec![0x64],
                cmd: CaPmtCmdId::OkDescrambling,
                last_ca_enable: None,
                last_descrambling_ok: false,
                built_ca_pmt: vec![],
                requery_ca_pmt: vec![],
                pmt_raw: vec![],
            },
        );
        let out = m.record_reply(1, Some(CaEnable::NotPossibleNoEntitlement), false);
        assert_eq!(out, Some((CaEnable::NotPossibleNoEntitlement, false)));
    }

    #[test]
    fn record_reply_unchanged_status_does_not_re_fire() {
        let mut m = ManagedCa::new();
        m.record(
            1,
            ManagedService {
                es_pids: vec![0x100],
                ca_pids: vec![0x64],
                cmd: CaPmtCmdId::OkDescrambling,
                last_ca_enable: None,
                last_descrambling_ok: false,
                built_ca_pmt: vec![],
                requery_ca_pmt: vec![],
                pmt_raw: vec![],
            },
        );
        assert!(m.record_reply(1, Some(CaEnable::Possible), true).is_some());
        assert_eq!(m.record_reply(1, Some(CaEnable::Possible), true), None);
    }

    #[test]
    fn record_reply_none_never_fires_but_updates_last() {
        let mut m = ManagedCa::new();
        m.record(
            1,
            ManagedService {
                es_pids: vec![0x100],
                ca_pids: vec![0x64],
                cmd: CaPmtCmdId::OkDescrambling,
                last_ca_enable: None,
                last_descrambling_ok: false,
                built_ca_pmt: vec![],
                requery_ca_pmt: vec![],
                pmt_raw: vec![],
            },
        );
        assert!(m.record_reply(1, Some(CaEnable::Possible), true).is_some());
        // Withdrawn: None never fires.
        assert_eq!(m.record_reply(1, None, false), None);
        // Re-affirmed with the SAME value as before the withdrawal: fires
        // again, because `last` was overwritten to `None` in between.
        assert_eq!(
            m.record_reply(1, Some(CaEnable::Possible), true),
            Some((CaEnable::Possible, true))
        );
    }

    #[test]
    fn record_reply_unknown_program_is_a_no_op() {
        let mut m = ManagedCa::new();
        assert_eq!(m.record_reply(99, Some(CaEnable::Possible), true), None);
    }

    // --- #763 Task 6: remove + clear ---

    #[test]
    fn remove_drops_tracked_service_and_recomputes_descramble_pids_false_for_untracked() {
        let mut m = ManagedCa::new();
        m.record(
            1,
            ManagedService {
                es_pids: vec![0x100, 0x101],
                ca_pids: vec![0x64],
                cmd: CaPmtCmdId::OkDescrambling,
                last_ca_enable: None,
                last_descrambling_ok: false,
                built_ca_pmt: vec![],
                requery_ca_pmt: vec![],
                pmt_raw: vec![],
            },
        );
        m.record(
            2,
            ManagedService {
                es_pids: vec![0x200],
                ca_pids: vec![0x65],
                cmd: CaPmtCmdId::OkDescrambling,
                last_ca_enable: None,
                last_descrambling_ok: false,
                built_ca_pmt: vec![],
                requery_ca_pmt: vec![],
                pmt_raw: vec![],
            },
        );

        assert!(
            !m.remove(99),
            "removing an untracked program_number must return false"
        );
        assert_eq!(
            m.services().len(),
            2,
            "an untracked remove must not disturb the tracked set"
        );

        assert!(
            m.remove(1),
            "removing a tracked program_number must return true"
        );
        assert!(m.services().get(&1).is_none());
        assert_eq!(
            m.descramble_pids(),
            &[0x200],
            "descramble_pids must recompute (drop program 1's PIDs) after remove"
        );
    }

    #[test]
    fn clear_resets_module_state_but_preserves_requery_interval() {
        use dvb_si::tables::cat::CatCaEntry;

        let mut m = ManagedCa::new();
        m.set_requery_interval(Duration::from_secs(3));
        m.record(
            1,
            ManagedService {
                es_pids: vec![0x100],
                ca_pids: vec![0x64],
                cmd: CaPmtCmdId::OkDescrambling,
                last_ca_enable: None,
                last_descrambling_ok: false,
                built_ca_pmt: vec![],
                requery_ca_pmt: vec![],
                pmt_raw: vec![],
            },
        );
        m.set_cat(&[CatCaEntry {
            ca_system_id: 0x0648,
            ca_pid: 0x1FF0,
            private_data: Vec::new(),
        }]);
        m.set_cam_caids([0x0648].into_iter().collect());
        assert!(!m.emm_pids().is_empty(), "precondition: emm_pids populated");
        assert!(
            !m.descramble_pids().is_empty(),
            "precondition: descramble_pids populated"
        );

        m.clear();

        assert!(m.services().is_empty(), "services must be cleared");
        assert!(m.emm_pids().is_empty(), "emm_pids must be cleared");
        assert!(
            m.descramble_pids().is_empty(),
            "descramble_pids must be cleared"
        );
        assert_eq!(
            m.requery_interval(),
            Duration::from_secs(3),
            "requery_interval is host config, must survive clear()"
        );
    }
}
