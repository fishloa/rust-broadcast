//! The driver — the one place I/O happens. It pumps a [`CaDevice`] against the
//! sans-IO [`CiStack`]: reads frames in, executes the stack's [`Action`]s
//! (writes/ioctls) out, tracks the requested poll timer, and collects
//! [`Notification`]s for the host application.

use std::collections::BTreeSet;
use std::io;
use std::time::{Duration, Instant};

use broadcast_common::Serialize;
use dvb_ci::builder::{build_ca_pmt, build_ca_pmt_for_caids};
use dvb_ci::objects::ca_pmt::{CaPmtCmdId, CaPmtListManagement};
use dvb_si::tables::cat::CatSection;
use dvb_si::tables::pmt::PmtSection;

use broadcast_common::Parse;

use crate::device::{CaDevice, SlotInfo};
use crate::event::{Action, Event, HostRequest, HotPlug, MmiEvent, Notification};
use crate::managed::{self, CaError, ManagedCa};
use crate::stack::CiStack;

/// Substrings (case-insensitive) in MMI menu/list/enquiry text that
/// heuristically indicate the smart card is absent. **Best-effort**: EN 50221
/// defines no card-detect signal, so this is free-text sniffing of real CAM
/// MMI copy, not a spec-defined mechanism.
const MMI_CARD_ABSENT_KEYWORDS: &[&str] = &[
    "no card",
    "insert card",
    "insert smart card",
    "card removed",
    "please insert",
];

/// Substrings (case-insensitive) in MMI menu/list/enquiry text that
/// heuristically indicate a valid smart card is present (entitlements
/// readable). **Best-effort**, same caveat as
/// [`MMI_CARD_ABSENT_KEYWORDS`].
const MMI_CARD_PRESENT_KEYWORDS: &[&str] = &["entitlement", "card valid", "subscription active"];

/// Clock source for [`Driver::pump`]'s elapsed-time measurement (r10-W-21).
/// A boxed closure (not a bare `fn` pointer) so a test's [`Driver::with_clock`]
/// can capture mutable state to simulate a jump of arbitrary size in one call,
/// which real wall-clock time cannot do deterministically.
type Clock = Box<dyn Fn() -> Instant>;

fn default_clock() -> Clock {
    Box::new(Instant::now)
}

/// Drives a [`CaDevice`] with the [`CiStack`].
pub struct Driver<D: CaDevice> {
    device: D,
    stack: CiStack,
    notifications: Vec<Notification>,
    /// Delay the stack last asked to be polled after (`None` = none pending).
    next_timer: Option<Duration>,
    /// Read buffer for one link-layer frame.
    buf: Vec<u8>,
    /// Last observed slot status (Part A hot-plug edge detection, #726).
    /// `None` means no [`SlotInfo`] has been observed yet — the first
    /// observation only establishes the baseline; it never itself fires
    /// [`Notification::HotPlug`] carrying [`HotPlug::CamPresent`]/
    /// [`CamRemoved`](HotPlug::CamRemoved), so `Driver::init` on an
    /// already-inserted module doesn't spuriously re-drive its own handshake.
    last_slot: Option<SlotInfo>,
    /// Last `ca_info` CAID set seen for the current module (Part B card
    /// inference, best-effort). `None` = not seen yet (baseline only).
    last_caids: Option<BTreeSet<u16>>,
    /// Last `ca_pmt_reply` `descrambling_ok` seen for the current module
    /// (Part B card inference, best-effort). `None` = not seen yet.
    last_descrambling_ok: Option<bool>,
    /// Card state last inferred from MMI text (`Some(true)` present,
    /// `Some(false)` absent, `None` = no card keyword seen yet): the keyword
    /// heuristic only reports a card *transition*, so a CAM that re-sends
    /// the same "insert card" menu does not repeat the notification (audit
    /// r10-O-13).
    last_mmi_card_present: Option<bool>,
    /// The slot's managed CAS-layer state (#763 Layer 1) — active services
    /// built via [`add_service`](Self::add_service).
    managed: ManagedCa,
    /// Wall-clock time of the start of the previous [`Self::pump`] call
    /// (r10-W-21). `None` before the first call — that call's `timeout`
    /// argument is used as-is, since there is no previous call to measure
    /// from.
    last_pump: Option<Instant>,
    /// Clock source for [`Self::pump`] — see [`Clock`]/[`Self::with_clock`].
    clock: Clock,
    /// r10-W-20: a re-query-tick serialization failure recorded for later
    /// reporting (a corrupt service must not silently swallow the healthy
    /// services' resend); drained with priority on the next tick.
    requery_pending: Option<CaError>,
}

impl<D: CaDevice> Driver<D> {
    /// New driver over `device`, single transport connection.
    #[must_use]
    pub fn new(device: D) -> Self {
        Self {
            device,
            stack: CiStack::new(),
            notifications: Vec::new(),
            next_timer: None,
            buf: vec![0u8; 4096],
            last_slot: None,
            last_caids: None,
            last_descrambling_ok: None,
            last_mmi_card_present: None,
            managed: ManagedCa::new(),
            last_pump: None,
            clock: default_clock(),
            requery_pending: None,
        }
    }

    /// Override [`Self::pump`]'s clock source (r10-W-21) — `Instant::now`
    /// by default. A test uses this to simulate an arbitrary elapsed jump
    /// deterministically, in place of a real sleep.
    #[must_use]
    pub fn with_clock(mut self, clock: impl Fn() -> Instant + 'static) -> Self {
        self.clock = Box::new(clock);
        self
    }

    /// The slot's managed CAS-layer state (#763 Layer 1) — the active
    /// service set built via [`add_service`](Self::add_service).
    pub fn managed_ca(&self) -> &ManagedCa {
        &self.managed
    }

    /// Borrow the underlying device (e.g. to inspect a mock's recorded ops).
    pub fn device(&self) -> &D {
        &self.device
    }

    /// Mutably borrow the underlying device (e.g. to script a mock's inbound
    /// frames between pumps).
    pub fn device_mut(&mut self) -> &mut D {
        &mut self.device
    }

    /// The poll delay the stack most recently requested, if any.
    pub fn next_timer(&self) -> Option<Duration> {
        self.next_timer
    }

    /// Drain the notifications collected so far.
    pub fn take_notifications(&mut self) -> Vec<Notification> {
        core::mem::take(&mut self.notifications)
    }

    /// Bring the interface up (reset + open the transport connection).
    pub fn init(&mut self) -> io::Result<()> {
        let actions = self.stack.handle(Event::Host(HostRequest::Init));
        self.run(actions)
    }

    /// Request the module descramble the services in `ca_pmt` (a serialized
    /// `ca_pmt` APDU body, e.g. from `dvb_ci::build_ca_pmt`).
    pub fn send_ca_pmt(&mut self, ca_pmt: &[u8]) -> io::Result<()> {
        let actions = self
            .stack
            .handle(Event::Host(HostRequest::SendCaPmt(ca_pmt)));
        self.run(actions)
    }

    /// Descramble the services in a PMT section: the stack filters the PMT's
    /// `CA_descriptor`s to the CAM's advertised CAIDs and sends a `ca_pmt`
    /// (`list_management = only`, `cmd_id = ok_descrambling`). The outcome
    /// surfaces as [`Notification::CaPmtReply`]. Call after the CAM is ready and
    /// its `ca_info` has been received (otherwise no CAID filter is applied).
    pub fn descramble(&mut self, pmt_section: &[u8]) -> io::Result<()> {
        let actions = self
            .stack
            .handle(Event::Host(HostRequest::Descramble(pmt_section)));
        self.run(actions)
    }

    /// Descramble a set of programmes in one CA-PMT list (`first`/`more`/`last`),
    /// replacing any previously selected set. Each element is a raw PMT section.
    pub fn descramble_programs(&mut self, pmt_sections: &[&[u8]]) -> io::Result<()> {
        let actions = self
            .stack
            .handle(Event::Host(HostRequest::DescramblePrograms(pmt_sections)));
        self.run(actions)
    }

    /// Add one programme to the descrambled set (`list_management = add`) without
    /// re-listing the others — for a capacity manager adding a viewer's service.
    pub fn add_program(&mut self, pmt_section: &[u8]) -> io::Result<()> {
        let actions = self
            .stack
            .handle(Event::Host(HostRequest::AddProgram(pmt_section)));
        self.run(actions)
    }

    /// Remove one programme from the descrambled set (`list_management = update`,
    /// `cmd_id = not_selected`) — tells the CAM to stop descrambling it.
    pub fn remove_program(&mut self, pmt_section: &[u8]) -> io::Result<()> {
        let actions = self
            .stack
            .handle(Event::Host(HostRequest::RemoveProgram(pmt_section)));
        self.run(actions)
    }

    /// Build the `ca_pmt` projection of `pmt`, CAID-filtered to the CAM's
    /// advertised `ca_info` CAIDs once known (#1067): a CICAM rejects a
    /// `ca_pmt` carrying a `CA_descriptor` for a `CA_system_id` it does not
    /// support, declining even the streams it could descramble
    /// (`dvb_ci::builder::build_ca_pmt_for_caids`'s own doc). Shared by
    /// [`add_service`](Self::add_service) and the entitlement re-query timer
    /// ([`requery_tick`](Self::requery_tick)) so both apply the same filter
    /// the raw [`descramble`](Self::descramble) path already does
    /// (`CiStack::build_ca_pmt_bytes`). Falls back to every `CA_descriptor`
    /// unfiltered before any `ca_info` has been observed, matching that same
    /// fallback.
    fn build_ca_pmt_filtered(
        &self,
        pmt: &PmtSection<'_>,
        list_management: CaPmtListManagement,
        cmd_id: CaPmtCmdId,
    ) -> dvb_ci::builder::CaPmtBuilt {
        let cam_caids = self.managed.cam_caids();
        if cam_caids.is_empty() {
            build_ca_pmt(pmt, list_management, cmd_id)
        } else {
            let allowed: Vec<u16> = cam_caids.iter().copied().collect();
            build_ca_pmt_for_caids(pmt, &allowed, list_management, cmd_id)
        }
    }

    /// Build + send the `ca_pmt` for `pmt` (via
    /// [`dvb_ci::builder::build_ca_pmt`], ETSI EN 50221 §8.4.3.4 Table 25) and
    /// track it in the slot's managed active-service set (#763 Layer 1).
    /// Additive alongside the raw [`send_ca_pmt`](Self::send_ca_pmt) and the
    /// existing multi-programme API
    /// ([`descramble_programs`](Self::descramble_programs)/
    /// [`add_program`](Self::add_program)).
    ///
    /// `list_management` (EN 50221 Table 25) is auto-selected from the tracked
    /// set: `Only` when this is the first service added to an empty managed
    /// set, `Add` when joining an already-active set. (Contrast the raw
    /// [`add_program`](Self::add_program), which always sends `Add` and leaves
    /// list-management sequencing to the caller.)
    ///
    /// # Errors
    /// [`CaError::NoCaDescriptor`] if `pmt` carries no `CA_descriptor`
    /// (ETSI EN 300 468 §6.2.16, tag `0x09`) at programme or
    /// elementary-stream level — there would be nothing for the CAM to
    /// descramble. [`CaError::Io`] if sending the built `ca_pmt` fails.
    pub fn add_service(&mut self, pmt: &PmtSection<'_>) -> Result<(), CaError> {
        if !managed::pmt_has_ca(pmt) {
            return Err(CaError::NoCaDescriptor {
                program_number: pmt.program_number,
            });
        }
        let list_management = if self.managed.is_empty() {
            CaPmtListManagement::Only
        } else {
            CaPmtListManagement::Add
        };
        let cmd_id = CaPmtCmdId::OkDescrambling;
        // r10-W-20: `try_to_bytes`, not `to_bytes` — a PMT whose descriptor
        // loop is corrupted past the section end projects into a `ca_pmt`
        // with no valid wire encoding; reject that as `Err` instead of
        // panicking inside the serializer. Checked BEFORE recording, so a
        // PMT whose ca_pmt cannot be encoded is never added to the managed
        // set (the re-query timer would fail the same encoding forever).
        let built_bytes = self
            .build_ca_pmt_filtered(pmt, list_management, cmd_id)
            .try_to_bytes()?;
        // `PmtSection` has no raw-bytes accessor — re-serialize (byte-identical
        // round-trip, a project invariant) to recover owned PMT bytes so
        // `remove_service` (#763 Task 6) can later re-drive `remove_program`
        // (which needs the raw section), and so `requery_tick` (#765) can
        // rebuild a fresh `query`-variant `ca_pmt` against the *current*
        // active set on every re-query tick rather than freezing
        // `list_management` at this moment.
        let mut pmt_raw = vec![0u8; pmt.serialized_len()];
        let n = pmt.serialize_into(&mut pmt_raw)?;
        pmt_raw.truncate(n);
        self.send_ca_pmt(&built_bytes)?;
        self.managed.record(
            pmt.program_number,
            managed::service_of(pmt, cmd_id, built_bytes, pmt_raw),
        );
        Ok(())
    }

    /// Stop descrambling a previously-added service (#763 Task 6): sends the
    /// removal `ca_pmt` (`list_management = update`, `cmd_id = not_selected`,
    /// EN 50221 §8.4.3.4 Table 25) via the existing
    /// [`remove_program`](Self::remove_program) path — re-driving it with the
    /// raw PMT bytes stashed at [`add_service`](Self::add_service) time — then
    /// drops the service from the managed set.
    ///
    /// Removing a `program_number` that isn't currently tracked (never
    /// `add_service`'d, or already removed) is a **no-op**, not an error:
    /// [`CaError`] has no not-found arm, and `remove_service` is idempotent.
    ///
    /// # Errors
    /// [`CaError::Io`] if sending the removal `ca_pmt` fails.
    pub fn remove_service(&mut self, program_number: u16) -> Result<(), CaError> {
        let raw = self
            .managed
            .services()
            .get(&program_number)
            .map(|s| s.pmt_raw.clone());
        let Some(raw) = raw else {
            return Ok(());
        };
        self.remove_program(&raw)?;
        self.managed.remove(program_number);
        Ok(())
    }

    /// Set the entitlement re-query cadence (#763 Task 5): every
    /// `interval`, the driver re-sends each actively-managed service's
    /// `ca_pmt` as a `list_management = update` pair — `cmd_id = query`
    /// immediately followed by `cmd_id = ok_descrambling` — see
    /// `requery_tick` for why both, #1032. `Duration::ZERO` disables
    /// re-query. **Opt-in**: defaults to [`managed::REQUERY_DEFAULT`]
    /// (`Duration::ZERO`) at construction — see that constant's doc for why
    /// (#1032: not hardware-verified).
    pub fn set_requery_interval(&mut self, interval: Duration) {
        self.managed.set_requery_interval(interval);
    }

    /// Feed a freshly-parsed CAT (ISO/IEC 13818-1 §2.4.4.5) to the managed
    /// CAS-layer state: extracts its `CA_descriptor`s (EN 300 468 §6.2.16,
    /// CAID → EMM PID) and recomputes [`emm_pids`](Self::emm_pids) against
    /// the CAM's advertised CAIDs (last `Notification::CaInfo`, captured
    /// automatically as it arrives — see [`pump`](Self::pump)).
    ///
    /// Calling this before any `ca_info` has been observed is **not** an
    /// error: [`emm_pids`](Self::emm_pids) stays empty until the CAM
    /// advertises its CAIDs, then recomputes against the CAT stored here —
    /// `set_cat` need not be re-called once `ca_info` arrives.
    ///
    /// # Errors
    /// [`CaError::Cat`] if the CAT's descriptor loop carries a truncated
    /// `CA_descriptor`.
    pub fn set_cat(&mut self, cat: &CatSection<'_>) -> Result<(), CaError> {
        let entries = cat.ca_descriptors().map_err(CaError::Cat)?;
        self.managed.set_cat(&entries);
        Ok(())
    }

    /// The EMM PIDs to route into `ci0` — the last [`set_cat`](Self::set_cat)'s
    /// CAID → EMM-PID map intersected with the CAM's advertised CAIDs (#763
    /// Task 4).
    #[must_use]
    pub fn emm_pids(&self) -> &[u16] {
        self.managed.emm_pids()
    }

    /// The PIDs to route into `ci0` for descrambling — the union of every
    /// actively-managed service's elementary-stream PIDs (#763 Task 4).
    #[must_use]
    pub fn descramble_pids(&self) -> &[u16] {
        self.managed.descramble_pids()
    }

    /// The union of every actively-managed service's `CA_PID`s (ECM PIDs —
    /// ISO/IEC 13818-1 §2.6.16 `CA_descriptor` `CA_PID`, programme + ES level
    /// combined) — the control-word channel, without which the module has ES
    /// to descramble but no control words to do it with (#763 Task 7).
    #[must_use]
    pub fn ca_pids(&self) -> &[u16] {
        self.managed.ca_pids()
    }

    /// `descramble_pids() ∪ ca_pids() ∪ emm_pids() ∪ PCR` — every PID class
    /// this slot needs on `ci0` (ES to descramble ∪ ECM for control words ∪
    /// EMM for entitlements ∪ each active service's PCR PID — ISO/IEC
    /// 13818-1 §2.4.4.8 — so the descrambled TS keeps its clock reference
    /// even when the PCR rides a dedicated PID). #763 Task 7's turnkey
    /// [`CaDescrambler`](crate::descrambler::CaDescrambler) filters its
    /// `feed_ts` input to exactly this set.
    #[must_use]
    pub fn required_pids(&self) -> Vec<u16> {
        self.managed.required_pids()
    }

    /// Answer an MMI menu/list by 1-based `choice_ref` (0 = back/cancel).
    pub fn mmi_menu_answer(&mut self, choice_ref: u8) -> io::Result<()> {
        let actions = self
            .stack
            .handle(Event::Host(HostRequest::MmiMenuAnswer(choice_ref)));
        self.run(actions)
    }

    /// Answer an MMI enquiry with the user's input (EN 300 468 Annex A bytes).
    pub fn mmi_enquiry_answer(&mut self, text: &[u8]) -> io::Result<()> {
        let actions = self
            .stack
            .handle(Event::Host(HostRequest::MmiEnquiryAnswer(text)));
        self.run(actions)
    }

    /// Abort the current MMI dialogue (`answ` with `answ_id = cancel`).
    pub fn mmi_cancel(&mut self) -> io::Result<()> {
        let actions = self.stack.handle(Event::Host(HostRequest::MmiCancel));
        self.run(actions)
    }

    /// Ask the module to open its MMI menu (`enter_menu`) — e.g. to read card /
    /// entitlement info from the module's own menus.
    pub fn enter_menu(&mut self) -> io::Result<()> {
        let actions = self.stack.handle(Event::Host(HostRequest::EnterMenu));
        self.run(actions)
    }

    /// One pump step: if the device is readable within `timeout`, read a frame
    /// and feed it; otherwise advance the stack's timers by `timeout` (driving
    /// the poll cadence). Returns whether a frame was processed.
    ///
    /// Also samples [`SlotInfo`] once per call (the DVB-CA slot has no
    /// interrupt/event of its own; `CA_GET_SLOT_INFO` is a poll) so a hot-plug
    /// edge is caught between reads — see [`Notification::HotPlug`] carrying
    /// [`HotPlug::CamPresent`]/[`CamRemoved`](HotPlug::CamRemoved) (#726).
    pub fn pump(&mut self, timeout: Duration) -> io::Result<bool> {
        // r10-W-21: advance the stack's timers by the REAL wall-clock time
        // since the previous call, not by `timeout` — `timeout` is only how
        // long THIS call's `poll` was willing to wait; the actual elapsed
        // time can be far less (poll returns as soon as the device is
        // readable, or a signal interrupts it early) or far more (the
        // caller's own loop didn't call `pump` again promptly). Using
        // `timeout` as if it were the measurement let the reply-timeout/poll
        // cadence drift out of sync with real time in either direction.
        // `None` (first call) has no previous point to measure from, so
        // `timeout` is used as a reasonable initial estimate, same as before.
        let now = (self.clock)();
        let elapsed = self
            .last_pump
            .map_or(timeout, |prev| now.saturating_duration_since(prev));
        self.last_pump = Some(now);

        self.run(vec![Action::QuerySlot])?;
        if self.device.poll(timeout)? {
            let n = self.device.read(&mut self.buf)?;
            if n > 0 {
                // `stack` and `buf` are disjoint fields: borrow the received
                // bytes in place instead of copying each frame (audit
                // r10-O-11).
                let actions = self.stack.handle(Event::Readable(&self.buf[..n]));
                self.run(actions)?;
                return Ok(true);
            }
        }
        let actions = self.stack.handle(Event::Tick { elapsed });
        self.run(actions)?;
        self.requery_tick(elapsed)?;
        Ok(false)
    }

    /// Advance the #763 Task 5 entitlement re-query cadence by `elapsed`
    /// ([`ManagedCa::tick`](crate::managed::ManagedCa::tick), mirroring
    /// `resource.rs`'s `DateTime::tick` accumulate-then-fire pattern). When
    /// the interval elapses, rebuild and re-send, for every actively-managed
    /// service, a `list_management = update` **pair**:
    ///
    /// 1. `cmd_id = query` (EN 50221 §8.4.3.5: "host expects a CA PMT Reply;
    ///    application not allowed to start descrambling or MMI before a new
    ///    CA PMT with `ok_descrambling`/`ok_mmi`") — solicits a fresh
    ///    `ca_pmt_reply` from a CAM that answers queries, surfacing as
    ///    [`Notification::CaPmtReply`] and, on a status change,
    ///    [`Notification::Entitlement`].
    /// 2. `cmd_id = ok_descrambling` (§8.4.3.5: "host expects no answer; the
    ///    application may start descrambling ... immediately"), sent
    ///    unconditionally right after — so descrambling is never left barred
    ///    by the query above, whether or not the CAM answers it.
    ///
    /// **#1032 fix**: the pre-fix code sent only `only`/`first` + `query`.
    /// EN 50221 §8.4.3.4 Table 25's `first`/`only` *replaces* every
    /// previously-selected programme, and §8.4.3.5's `query` bars
    /// descrambling until an `ok_descrambling` that was never sent — so
    /// descrambling silently stopped ~`interval` after it started. `update`
    /// ("the CA PMT of a programme already in the list is sent again ...
    /// List management commands act only at programme level", §8.4.3.4)
    /// never replaces the list, and the unconditional follow-up
    /// `ok_descrambling` guarantees the bar is always lifted even though
    /// `stack.rs`'s `descramble` already documents that a live
    /// AlphaCrypt/Irdeto module never answers `query` at all — this is why
    /// [`managed::REQUERY_DEFAULT`] is `Duration::ZERO`: the feature is
    /// opt-in until the `query` half is verified to elicit a reply on a
    /// caller's own hardware.
    ///
    /// **#765 (retained)**: the re-query set is rebuilt fresh from each
    /// service's stored `pmt_raw` on *every* tick against the *current*
    /// active set (`services`, a `BTreeMap`) — a removed service is not
    /// resent, and a survivor is rebuilt from its own current PMT — rather
    /// than freezing anything at `add_service` time. `update` needs no
    /// first/more/last position bookkeeping (§8.4.3.4: "act only at
    /// programme level"), so unlike the pre-fix code this no longer depends
    /// on the active set's size or a service's position within it.
    ///
    /// CAID-filtered via [`build_ca_pmt_filtered`](Self::build_ca_pmt_filtered)
    /// (#1067), same as [`add_service`](Self::add_service).
    fn requery_tick(&mut self, elapsed: Duration) -> io::Result<()> {
        // r10-W-20: a previously-recorded rejection is reported FIRST, on
        // every pump until the caller fixes its state — never swallowed by
        // an earlier `tick` return.
        if let Some(err) = self.requery_pending.take() {
            return Err(io::Error::other(err));
        }
        if !self.managed.tick(elapsed) {
            return Ok(());
        }
        let mut ca_pmts: Vec<(Vec<u8>, Vec<u8>)> =
            Vec::with_capacity(self.managed.services().len());
        for s in self.managed.services().values() {
            // r10-W-20: no `expect`/`to_bytes` here — `pmt_raw` is only
            // as trustworthy as whatever the caller's CAM (or a
            // corrupted capture) supplied, so a re-parse or re-serialize
            // failure must end this re-query as `Err`, not a panic.
            let outcome: Result<(Vec<u8>, Vec<u8>), CaError> = (|| {
                let pmt = PmtSection::parse(&s.pmt_raw).map_err(CaError::PmtParse)?;
                let query = self
                    .build_ca_pmt_filtered(&pmt, CaPmtListManagement::Update, CaPmtCmdId::Query)
                    .try_to_bytes()?;
                let ok_descrambling = self
                    .build_ca_pmt_filtered(
                        &pmt,
                        CaPmtListManagement::Update,
                        CaPmtCmdId::OkDescrambling,
                    )
                    .try_to_bytes()?;
                Ok((query, ok_descrambling))
            })();
            match outcome {
                Ok(pair) => ca_pmts.push(pair),
                Err(err) => {
                    // One corrupt (or CAM-supplied) service must not poison
                    // the healthy services' resend this tick: record it and
                    // return the failure after the good pair(s) went out.
                    self.requery_pending = Some(err);
                }
            }
        }
        for (query, ok_descrambling) in ca_pmts {
            self.send_ca_pmt(&query)?;
            self.send_ca_pmt(&ok_descrambling)?;
        }
        if let Some(err) = self.requery_pending.take() {
            return Err(io::Error::other(err));
        }
        Ok(())
    }

    /// Pump once ([`pump`](Self::pump)), then invoke `handler` for each
    /// [`Notification`] produced this cycle (drain-and-dispatch via
    /// [`take_notifications`](Self::take_notifications)). Returns the same
    /// bool as `pump`. The closure is per-call — nothing is stored, so there
    /// are no lifetime constraints beyond the call itself. This crate is
    /// sync/sans-IO (no channels/async runtime), so a closure callback is the
    /// idiomatic push-style alternative to poll-draining `take_notifications`
    /// yourself.
    pub fn pump_with<F: FnMut(&Notification)>(
        &mut self,
        timeout: Duration,
        mut handler: F,
    ) -> io::Result<bool> {
        let progressed = self.pump(timeout)?;
        for n in self.take_notifications() {
            handler(&n);
        }
        Ok(progressed)
    }

    /// Convenience over [`pump_with`](Self::pump_with): invoke `handler` only
    /// for [`HotPlug`] transitions, ignoring every other [`Notification`]
    /// produced this cycle.
    pub fn pump_hotplug<F: FnMut(HotPlug)>(
        &mut self,
        timeout: Duration,
        mut handler: F,
    ) -> io::Result<bool> {
        self.pump_with(timeout, |n| {
            if let Some(h) = n.hotplug() {
                handler(h);
            }
        })
    }

    /// Execute the stack's actions against the device.
    fn run(&mut self, actions: Vec<Action>) -> io::Result<()> {
        for action in actions {
            match action {
                Action::Write(bytes) => self.device.write(&bytes)?,
                Action::Reset => self.device.reset()?,
                Action::QuerySlot => {
                    let info = self.device.slot_info()?;
                    self.handle_slot_info(info)?;
                }
                Action::SetTimer { after } => self.next_timer = Some(after),
                Action::Notify(n) => {
                    let inferred = self.infer_card(&n);
                    self.notifications.push(n);
                    self.notifications.extend(inferred);
                }
            }
        }
        Ok(())
    }

    /// Compare a freshly-queried [`SlotInfo`] against the last one observed
    /// and react to a `module_present` edge (Part A, #726): the *first*
    /// observation ever (`self.last_slot == None`) only establishes the
    /// baseline — it must not fire a notification, or `Driver::init` against
    /// an already-inserted module would spuriously report a hot-plug and
    /// recurse into re-driving its own in-progress handshake.
    fn handle_slot_info(&mut self, info: SlotInfo) -> io::Result<()> {
        let prev = self.last_slot.replace(info);
        match prev {
            Some(prev) if !prev.module_present && info.module_present => {
                self.notifications
                    .push(Notification::HotPlug(HotPlug::CamPresent));
                self.reset_module_state();
                // Re-drive the same reset/init path `Driver::init` uses, so
                // the newly-inserted module gets a clean resource-manager
                // handshake (no duplicated handshake logic).
                let actions = self.stack.handle(Event::Host(HostRequest::Init));
                self.run(actions)?;
            }
            Some(prev) if prev.module_present && !info.module_present => {
                self.notifications
                    .push(Notification::HotPlug(HotPlug::CamRemoved));
                self.reset_module_state();
            }
            _ => {}
        }
        Ok(())
    }

    /// Reset per-module protocol + card-inference state after a CAM
    /// insert/remove edge: a fresh [`CiStack`] (so a re-insert re-handshakes
    /// cleanly instead of reusing stale session numbers) and cleared Part B
    /// baselines (so the next module's `ca_info`/`ca_pmt_reply` establishes
    /// its own fresh baseline rather than diffing against the departed
    /// module's), AND the managed CAS-layer state (#763 Task 6 fix): a stale
    /// `services`/`descramble_pids`/`emm_pids` set must not survive a
    /// departed or freshly-inserted module — the host must re-provision from
    /// scratch (the next `add_service` then correctly picks `Only` again).
    fn reset_module_state(&mut self) {
        self.stack = CiStack::new();
        self.next_timer = None;
        self.last_caids = None;
        self.last_descrambling_ok = None;
        self.last_mmi_card_present = None;
        self.managed.clear();
    }

    /// Best-effort app-layer card-presence inference (Part B, #726): EN 50221
    /// CI slots are module-level only — there is no card-detect line (verified
    /// against real DD ddbridge / cxd2099 driver behaviour) — so this derives
    /// card insert/remove/change from signals the module already sends for
    /// other reasons. Returns any inferred [`Notification`]s (0 or 1); `note`
    /// itself is pushed by the caller.
    fn infer_card(&mut self, note: &Notification) -> Vec<Notification> {
        match note {
            Notification::CaInfo { ca_system_ids } => {
                let new_set: BTreeSet<u16> = ca_system_ids.iter().copied().collect();
                let mut out = Vec::new();
                if let Some(prev) = &self.last_caids {
                    if prev.is_empty() && !new_set.is_empty() {
                        out.push(Notification::HotPlug(HotPlug::CardInserted));
                    } else if !prev.is_empty() && new_set.is_empty() {
                        out.push(Notification::HotPlug(HotPlug::CardRemoved));
                    } else if !prev.is_empty() && !new_set.is_empty() && *prev != new_set {
                        out.push(Notification::HotPlug(HotPlug::CardChanged));
                    }
                }
                // #763 Task 4: feed the CAM's advertised CAIDs to the managed
                // CAS-layer state so `emm_pids` recomputes against the
                // already-stored CAT map (if `set_cat` ran first).
                self.managed.set_cam_caids(new_set.clone());
                self.last_caids = Some(new_set);
                out
            }
            Notification::CaPmtReply {
                program_number,
                ca_enable,
                descrambling_ok,
            } => {
                let mut out = Vec::new();
                if let Some(prev) = self.last_descrambling_ok {
                    if !prev && *descrambling_ok {
                        out.push(Notification::HotPlug(HotPlug::CardInserted));
                    } else if prev && !*descrambling_ok {
                        out.push(Notification::HotPlug(HotPlug::CardRemoved));
                    }
                }
                self.last_descrambling_ok = Some(*descrambling_ok);
                // #763 Task 5: diff this reply's programme-level status
                // against the last one recorded for `program_number` and
                // surface the edge-triggered `Notification::Entitlement`.
                if let Some((v, ok)) =
                    self.managed
                        .record_reply(*program_number, *ca_enable, *descrambling_ok)
                {
                    out.push(Notification::Entitlement {
                        program_number: *program_number,
                        ca_enable: v,
                        descrambling_ok: ok,
                    });
                }
                out
            }
            Notification::Mmi(ev) => {
                let present = Self::mmi_text(ev).and_then(|text| {
                    let lower = text.to_lowercase();
                    if MMI_CARD_ABSENT_KEYWORDS.iter().any(|k| lower.contains(k)) {
                        Some(false)
                    } else if MMI_CARD_PRESENT_KEYWORDS.iter().any(|k| lower.contains(k)) {
                        Some(true)
                    } else {
                        None
                    }
                });
                match present {
                    // Edge only: the same state repeated is not news.
                    Some(p) if self.last_mmi_card_present != Some(p) => {
                        self.last_mmi_card_present = Some(p);
                        vec![Notification::HotPlug(if p {
                            HotPlug::CardInserted
                        } else {
                            HotPlug::CardRemoved
                        })]
                    }
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    /// The free-text an [`MmiEvent`] carries, for the keyword heuristic above
    /// (title/subtitle/bottom/choices for a menu/list, the prompt for an
    /// enquiry; a `Close` carries no text).
    fn mmi_text(ev: &MmiEvent) -> Option<String> {
        match ev {
            MmiEvent::Menu(m) | MmiEvent::List(m) => {
                let mut s = format!("{} {} {}", m.title, m.subtitle, m.bottom);
                for choice in &m.choices {
                    s.push(' ');
                    s.push_str(choice);
                }
                Some(s)
            }
            MmiEvent::Enquiry { prompt, .. } => Some(prompt.clone()),
            MmiEvent::Close => None,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::device::{DeviceOp, MockCaDevice};
    use crate::event::{HostControlEvent, HotPlug, Notification};
    use broadcast_common::Serialize;
    use dvb_ci::tpdu::tags;

    pub(crate) fn ser<S: Serialize>(s: &S) -> Vec<u8> {
        let mut b = vec![0u8; s.serialized_len()];
        match s.serialize_into(&mut b) {
            Ok(n) => b.truncate(n),
            Err(_) => b.clear(),
        }
        b
    }

    /// A manually-advanced fake clock for [`Driver::with_clock`] (r10-W-21):
    /// `pump`'s elapsed-time measurement is now real wall-clock time, so a
    /// test that wants to simulate "N of simulated time passed" in one
    /// `pump` call (rather than actually sleeping) advances this by that
    /// same `N` immediately before the call.
    #[derive(Clone)]
    pub(crate) struct TestClock(std::rc::Rc<std::cell::Cell<Instant>>);

    impl TestClock {
        pub(crate) fn new() -> Self {
            Self(std::rc::Rc::new(std::cell::Cell::new(Instant::now())))
        }

        /// Advance the simulated clock by `d`.
        pub(crate) fn advance(&self, d: Duration) {
            self.0.set(self.0.get() + d);
        }

        /// The `Fn() -> Instant` closure to pass to [`Driver::with_clock`].
        pub(crate) fn as_fn(&self) -> impl Fn() -> Instant + 'static {
            let cell = self.0.clone();
            move || cell.get()
        }
    }

    /// Wrap an SPDU as a module→host `T_Data_Last` R_TPDU (+ trailing T_SB,
    /// data_available clear) on transport connection `tcid`.
    fn r_data(tcid: u8, spdu: &[u8]) -> Vec<u8> {
        use dvb_ci::tpdu::{SbValue, tags as tpdu_tags};
        let mut v = vec![tpdu_tags::DATA_LAST, (1 + spdu.len()) as u8, tcid];
        v.extend_from_slice(spdu);
        v.extend_from_slice(&[tpdu_tags::SB, 0x02, tcid, SbValue::new(false).0]);
        v
    }

    /// Wrap an APDU for delivery on `session_nb` (session_number prefix), then as
    /// a module→host R_TPDU on tcid 1.
    pub(crate) fn r_apdu(session_nb: u16, apdu: &[u8]) -> Vec<u8> {
        use dvb_ci::spdu::SessionNumber;
        let mut spdu = ser(&SessionNumber { session_nb });
        spdu.extend_from_slice(apdu);
        r_data(1, &spdu)
    }

    /// A standalone module→host `T_SB` (data_available clear) ack — flushes one
    /// queued host write per turn (#337).
    pub(crate) fn sb() -> Vec<u8> {
        use dvb_ci::tpdu::{SbValue, tags as tpdu_tags};
        vec![tpdu_tags::SB, 0x02, 0x01, SbValue::new(false).0]
    }

    /// Feed one scripted module frame into the mock and pump it, then pump a
    /// handful of SB acks so any queued host writes flush.
    pub(crate) fn feed(d: &mut Driver<MockCaDevice>, frame: Vec<u8>) {
        d.device_mut().inbound.push_back(frame);
        d.pump(Duration::from_millis(10)).unwrap();
        for _ in 0..8 {
            d.device_mut().inbound.push_back(sb());
            d.pump(Duration::from_millis(10)).unwrap();
        }
    }

    /// Drive the EN 50221 handshake through the `Driver` until host_control and
    /// the other module-provided sessions are open (mirrors the stack-level
    /// `stack_with_ca_session`, but exercises the real driver I/O path).
    pub(crate) fn driver_with_sessions() -> Driver<MockCaDevice> {
        use dvb_ci::objects::resource_manager::Profile;
        use dvb_ci::resource::{
            APPLICATION_INFORMATION, CONDITIONAL_ACCESS_SUPPORT, HOST_CONTROL, MMI,
            RESOURCE_MANAGER,
        };
        use dvb_ci::spdu::{CreateSessionResponse, OpenSessionRequest, SessionStatus};

        let mut d = Driver::new(MockCaDevice::new([]));
        d.init().unwrap();
        // module accepts the transport connection
        feed(&mut d, vec![tags::C_T_C_REPLY, 0x01, 0x01]);
        // module opens the host's resource_manager → RM session 1
        feed(
            &mut d,
            r_data(
                1,
                &ser(&OpenSessionRequest {
                    resource: RESOURCE_MANAGER,
                }),
            ),
        );
        // module's profile → host: CamReady + profile_change + create_session for
        // each module-provided resource.
        feed(
            &mut d,
            r_apdu(
                1,
                &ser(&Profile {
                    resources: vec![
                        APPLICATION_INFORMATION,
                        CONDITIONAL_ACCESS_SUPPORT,
                        MMI,
                        HOST_CONTROL,
                    ],
                }),
            ),
        );
        // module accepts each create_session (session nbs 2..=5 in registration order)
        for (nb, res) in [
            (2u16, APPLICATION_INFORMATION),
            (3, CONDITIONAL_ACCESS_SUPPORT),
            (4, MMI),
            (5, HOST_CONTROL),
        ] {
            feed(
                &mut d,
                r_data(
                    1,
                    &ser(&CreateSessionResponse {
                        status: SessionStatus::Ok,
                        resource: res,
                        session_nb: nb,
                    }),
                ),
            );
        }
        d
    }

    // Session numbers the module allocates in `driver_with_sessions`, in
    // registration order: RM=1, app_info=2, conditional_access=3, mmi=4,
    // host_control=5. (Asserted by `handshake_opens_expected_sessions`.)
    const RM_SESSION: u16 = 1;
    pub(crate) const CA_SESSION: u16 = 3;
    const MMI_SESSION: u16 = 4;
    const HOST_CONTROL_SESSION: u16 = 5;

    #[test]
    fn host_control_tune_apdu_surfaces_notification_via_driver() {
        use dvb_ci::objects::host_control::Tune;

        let mut d = driver_with_sessions();
        let hc_nb = HOST_CONTROL_SESSION;
        d.take_notifications(); // drop handshake notifications

        // Module (CAM) sends a Tune request on its host_control session.
        let tune = Tune {
            network_id: 0x1122,
            original_network_id: 0x3344,
            transport_stream_id: 0x5566,
            service_id: 0x7788,
        };
        feed(&mut d, r_apdu(hc_nb, &ser(&tune)));

        // The runtime surfaces the decoded HostControl(Tune) notification.
        let notes = d.take_notifications();
        assert!(
            notes.contains(&Notification::HostControl(HostControlEvent::Tune {
                network_id: 0x1122,
                original_network_id: 0x3344,
                transport_stream_id: 0x5566,
                service_id: 0x7788,
            })),
            "expected HostControl(Tune) notification, got {notes:?}"
        );
    }

    /// A padded `tune` from the CAM is refused by dvb-ci's strict parser; the
    /// runtime must surface that as `Notification::Error`, not drop it.
    #[test]
    fn padded_tune_from_the_cam_surfaces_an_error_notification() {
        let mut d = driver_with_sessions();
        d.take_notifications();
        // Tune: 9F 84 00, length 9 (one byte over the fixed 8), then 9 bytes.
        let padded = [
            0x9F, 0x84, 0x00, 0x09, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0xEE,
        ];
        feed(&mut d, r_apdu(HOST_CONTROL_SESSION, &padded));
        let notes = d.take_notifications();
        assert!(
            notes.iter().any(
                |n| matches!(n, Notification::Error { detail } if detail.contains("trailing bytes"))
            ),
            "expected an Error notification naming the trailing bytes, got {notes:?}"
        );
        assert!(
            !notes
                .iter()
                .any(|n| matches!(n, Notification::HostControl(_))),
            "a refused tune must not also surface as a HostControl event: {notes:?}"
        );
    }

    /// A malformed APDU on other resources is reported too (MMI `enq` with a
    /// truncated body here), not silently ignored.
    #[test]
    fn malformed_mmi_apdu_from_the_cam_surfaces_an_error_notification() {
        let mut d = driver_with_sessions();
        d.take_notifications();
        // enq (9F 88 07) with a 1-byte body: shorter than its fixed prefix.
        feed(&mut d, r_apdu(MMI_SESSION, &[0x9F, 0x88, 0x07, 0x01, 0x00]));
        let notes = d.take_notifications();
        assert!(
            notes
                .iter()
                .any(|n| matches!(n, Notification::Error { .. })),
            "expected an Error notification, got {notes:?}"
        );
    }

    #[test]
    fn profile_reply_advertises_host_control() {
        use broadcast_common::Parse;
        use dvb_ci::objects::resource_manager::{Profile, ProfileEnq};
        use dvb_ci::resource::{HOST_CONTROL, RESOURCE_MANAGER};

        let mut d = Driver::new(MockCaDevice::new([]));
        d.init().unwrap();
        feed(&mut d, vec![tags::C_T_C_REPLY, 0x01, 0x01]);
        // Open RM, then the module enquires the host profile.
        feed(
            &mut d,
            r_data(
                1,
                &ser(&dvb_ci::spdu::OpenSessionRequest {
                    resource: RESOURCE_MANAGER,
                }),
            ),
        );
        // Module → profile_enq on the RM session → host replies with its profile.
        feed(&mut d, r_apdu(RM_SESSION, &ser(&ProfileEnq)));

        // Find the host's `profile` reply (tag 9F 80 11) in the written frames and
        // confirm it lists HOST_CONTROL.
        let want = dvb_ci::tag::PROFILE.to_bytes();
        let found = d.device().ops.iter().any(|op| {
            if let DeviceOp::Write(w) = op
                && let Some(pos) = w.windows(3).position(|x| x == want)
                && let Ok(p) = Profile::parse(&w[pos..])
            {
                return p.resources.contains(&HOST_CONTROL);
            }
            false
        });
        assert!(found, "profile reply must advertise HOST_CONTROL");
    }

    #[test]
    fn mmi_menu_answ_and_answ_are_byte_exact_on_the_mmi_session() {
        use dvb_ci::objects::mmi_high::{Answ, AnswId, MenuAnsw};

        let mut d = driver_with_sessions();
        let mmi_nb = MMI_SESSION;

        // menu_answ(choice_ref = 2): the driver method must put the exact dvb-ci
        // MenuAnsw serialization on the wire, on the MMI session.
        d.mmi_menu_answer(2).unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();
        assert_apdu_on_session(&d, mmi_nb, &ser(&MenuAnsw { choice_ref: 2 }));

        // answ(answer, "1234"): byte-exact Answ serialization on the MMI session.
        d.mmi_enquiry_answer(b"1234").unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();
        assert_apdu_on_session(
            &d,
            mmi_nb,
            &ser(&Answ {
                answ_id: AnswId::Answer,
                text_chars: b"1234",
            }),
        );
    }

    /// Assert some host write carries `session_number(session_nb)` immediately
    /// followed by the exact `apdu` bytes (byte-exact APDU on the right session).
    fn assert_apdu_on_session(d: &Driver<MockCaDevice>, session_nb: u16, apdu: &[u8]) {
        use dvb_ci::spdu::SessionNumber;
        let mut want = ser(&SessionNumber { session_nb });
        want.extend_from_slice(apdu);
        let hit = d.device().ops.iter().any(|op| match op {
            DeviceOp::Write(w) => w.windows(want.len()).any(|x| x == want.as_slice()),
            _ => false,
        });
        assert!(
            hit,
            "expected APDU {apdu:02X?} on session {session_nb} (session-prefixed {want:02X?}) in writes"
        );
    }

    /// How many host writes carry `session_number(session_nb)` immediately
    /// followed by the exact `apdu` bytes — used to distinguish an initial
    /// send from a later re-send (#763 Task 5's re-query timer).
    fn count_apdu_on_session(d: &Driver<MockCaDevice>, session_nb: u16, apdu: &[u8]) -> usize {
        use dvb_ci::spdu::SessionNumber;
        let mut want = ser(&SessionNumber { session_nb });
        want.extend_from_slice(apdu);
        d.device()
            .ops
            .iter()
            .filter(|op| match op {
                DeviceOp::Write(w) => w.windows(want.len()).any(|x| x == want.as_slice()),
                _ => false,
            })
            .count()
    }

    #[test]
    fn init_drives_reset_slotinfo_and_create_tc_to_device() {
        let mut d = Driver::new(MockCaDevice::new([]));
        d.init().unwrap();
        let ops = &d.device().ops;
        assert_eq!(ops[0], DeviceOp::Reset);
        assert_eq!(ops[1], DeviceOp::SlotInfo);
        assert!(matches!(&ops[2], DeviceOp::Write(w) if w[0] == tags::CREATE_T_C));
    }

    #[test]
    fn reads_reply_then_polls_on_pump() {
        // Script the module accepting the connection.
        let dev = MockCaDevice::new([vec![tags::C_T_C_REPLY, 0x01, 0x01]]);
        let clock = TestClock::new();
        let mut d = Driver::new(dev).with_clock(clock.as_fn());
        d.init().unwrap();
        // first pump reads the C_T_C_Reply (activates the connection)
        assert!(d.pump(Duration::from_millis(100)).unwrap());
        // r10-W-21: `pump` now measures real (simulated, via TestClock)
        // elapsed time rather than trusting the `timeout` argument, so the
        // test advances the clock by the same amount `timeout` used to
        // stand in for.
        clock.advance(Duration::from_millis(100));
        // next pump has nothing to read → ticks → emits a poll write
        assert!(!d.pump(Duration::from_millis(100)).unwrap());
        let last = d.device().ops.last().unwrap();
        assert!(matches!(last, DeviceOp::Write(w) if w.first() == Some(&tags::DATA_LAST)));
    }

    /// r10-W-21: `pump`'s stack-clock advance must reflect real (here,
    /// simulated) elapsed time, not the `timeout` argument on its own —
    /// pre-fix, calling `pump(1s)` twice back-to-back with almost no real
    /// time between them still advanced the stack's clock by a full second
    /// each time, exactly as if a full second really had passed.
    #[test]
    fn pump_advances_the_stack_clock_by_real_elapsed_time_not_by_timeout() {
        let dev = MockCaDevice::new([vec![tags::C_T_C_REPLY, 0x01, 0x01]]);
        let clock = TestClock::new();
        let mut d = Driver::new(dev).with_clock(clock.as_fn());
        d.init().unwrap();
        assert!(d.pump(Duration::from_secs(1)).unwrap());

        // Only 1ms of (simulated) real time actually passed, even though
        // `timeout` claims a full second — nowhere near the 100ms poll
        // cadence, so no poll write should go out.
        clock.advance(Duration::from_millis(1));
        assert!(!d.pump(Duration::from_secs(1)).unwrap());
        assert!(
            d.device().ops.last().is_none_or(|op| !matches!(
                op,
                DeviceOp::Write(w) if w.first() == Some(&tags::DATA_LAST)
            )),
            "must not poll after only 1ms of real elapsed time, regardless of the 1s `timeout` argument"
        );

        // Now really cross the poll interval.
        clock.advance(Duration::from_millis(100));
        assert!(!d.pump(Duration::from_secs(1)).unwrap());
        let last = d.device().ops.last().unwrap();
        assert!(matches!(last, DeviceOp::Write(w) if w.first() == Some(&tags::DATA_LAST)));
    }

    // --- #726: CAM + card hot-plug notifications ---

    #[test]
    fn cam_insert_edge_emits_cam_present_once_and_redrives_handshake() {
        let mut dev = MockCaDevice::new([]);
        dev.slot = SlotInfo {
            num: 0,
            module_ready: false,
            module_present: false,
        };
        let mut d = Driver::new(dev);
        d.init().unwrap();
        // The first-ever slot observation only establishes the baseline
        // (absent) — it must not itself claim a hot-plug edge.
        let notes = d.take_notifications();
        assert!(
            !notes.contains(&Notification::HotPlug(HotPlug::CamPresent)),
            "baseline observation must not fire CamPresent, got {notes:?}"
        );
        let resets_before = d
            .device()
            .ops
            .iter()
            .filter(|o| **o == DeviceOp::Reset)
            .count();

        // Module physically inserted and ready.
        d.device_mut().slot = SlotInfo {
            num: 0,
            module_ready: true,
            module_present: true,
        };
        d.pump(Duration::from_millis(10)).unwrap();

        let notes = d.take_notifications();
        let cam_present_count = notes
            .iter()
            .filter(|n| **n == Notification::HotPlug(HotPlug::CamPresent))
            .count();
        assert_eq!(
            cam_present_count, 1,
            "expected exactly one CamPresent, got {notes:?}"
        );
        // Handshake re-driven: a fresh Reset, and the last write is CREATE_T_C.
        let resets_after = d
            .device()
            .ops
            .iter()
            .filter(|o| **o == DeviceOp::Reset)
            .count();
        assert_eq!(
            resets_after,
            resets_before + 1,
            "expected one fresh Reset on re-insert"
        );
        assert!(
            matches!(d.device().ops.last(), Some(DeviceOp::Write(w)) if w[0] == tags::CREATE_T_C),
            "expected the handshake re-driven (CREATE_T_C written), got {:?}",
            d.device().ops.last()
        );
    }

    #[test]
    fn cam_remove_edge_emits_cam_removed_and_re_insert_re_handshakes() {
        let mut d = driver_with_sessions();
        d.take_notifications();

        // Module physically removed.
        d.device_mut().slot.module_present = false;
        d.pump(Duration::from_millis(10)).unwrap();
        let notes = d.take_notifications();
        assert!(
            notes.contains(&Notification::HotPlug(HotPlug::CamRemoved)),
            "expected CamRemoved, got {notes:?}"
        );

        // Session state was torn down: the MMI session from
        // `driver_with_sessions` no longer exists on the fresh stack, so an
        // answer to it now errors instead of silently going nowhere.
        d.mmi_menu_answer(0).unwrap();
        let notes = d.take_notifications();
        assert!(
            notes
                .iter()
                .any(|n| matches!(n, Notification::Error { .. })),
            "expected no open MMI session after teardown, got {notes:?}"
        );

        // Re-insert: a fresh handshake starts (Reset + CamPresent).
        let resets_before = d
            .device()
            .ops
            .iter()
            .filter(|o| **o == DeviceOp::Reset)
            .count();
        d.device_mut().slot.module_present = true;
        d.device_mut().slot.module_ready = true;
        d.pump(Duration::from_millis(10)).unwrap();
        let notes = d.take_notifications();
        assert!(
            notes.contains(&Notification::HotPlug(HotPlug::CamPresent)),
            "expected CamPresent on re-insert, got {notes:?}"
        );
        let resets_after = d
            .device()
            .ops
            .iter()
            .filter(|o| **o == DeviceOp::Reset)
            .count();
        assert_eq!(resets_after, resets_before + 1, "expected a fresh Reset");
    }

    #[test]
    fn slot_status_unchanged_across_polls_emits_no_hotplug_notifications() {
        let mut d = Driver::new(MockCaDevice::new([]));
        d.init().unwrap();
        d.take_notifications();

        for _ in 0..5 {
            d.pump(Duration::from_millis(10)).unwrap();
        }
        let notes = d.take_notifications();
        assert!(
            !notes.iter().any(|n| matches!(
                n,
                Notification::HotPlug(HotPlug::CamPresent | HotPlug::CamRemoved)
            )),
            "unchanged slot status must not emit hot-plug notifications, got {notes:?}"
        );
    }

    #[test]
    fn ca_info_caid_set_change_infers_card_inserted_then_changed() {
        use dvb_ci::objects::ca_info::CaInfo;

        let mut d = driver_with_sessions();
        d.take_notifications();

        // First ca_info: no CAIDs (baseline only, no notification).
        feed(
            &mut d,
            r_apdu(
                CA_SESSION,
                &ser(&CaInfo {
                    ca_system_ids: vec![],
                }),
            ),
        );
        let notes = d.take_notifications();
        assert!(
            !notes.iter().any(|n| matches!(
                n,
                Notification::HotPlug(
                    HotPlug::CardInserted | HotPlug::CardChanged | HotPlug::CardRemoved
                )
            )),
            "first ca_info must only establish the baseline, got {notes:?}"
        );

        // CAID set becomes populated: card inserted.
        feed(
            &mut d,
            r_apdu(
                CA_SESSION,
                &ser(&CaInfo {
                    ca_system_ids: vec![0x0B00],
                }),
            ),
        );
        let notes = d.take_notifications();
        assert!(
            notes.contains(&Notification::HotPlug(HotPlug::CardInserted)),
            "expected CardInserted, got {notes:?}"
        );

        // CAID set changes to a different non-empty set: card changed.
        feed(
            &mut d,
            r_apdu(
                CA_SESSION,
                &ser(&CaInfo {
                    ca_system_ids: vec![0x1800],
                }),
            ),
        );
        let notes = d.take_notifications();
        assert!(
            notes.contains(&Notification::HotPlug(HotPlug::CardChanged)),
            "expected CardChanged, got {notes:?}"
        );
    }

    #[test]
    fn ca_pmt_reply_descrambling_transition_infers_card_present_then_removed() {
        use dvb_ci::objects::ca_pmt_reply::{CaEnable, CaPmtReply};

        fn reply(ca_enable: Option<CaEnable>) -> CaPmtReply {
            CaPmtReply {
                program_number: 1,
                version_number: 1,
                current_next_indicator: true,
                ca_enable,
                streams: vec![],
            }
        }

        let mut d = driver_with_sessions();
        d.take_notifications();

        // Baseline: descrambling not (yet) possible.
        feed(&mut d, r_apdu(CA_SESSION, &ser(&reply(None))));
        let notes = d.take_notifications();
        assert!(
            !notes.iter().any(|n| matches!(
                n,
                Notification::HotPlug(HotPlug::CardInserted | HotPlug::CardRemoved)
            )),
            "first ca_pmt_reply must only establish the baseline, got {notes:?}"
        );

        // false -> true: card-present inference.
        feed(
            &mut d,
            r_apdu(CA_SESSION, &ser(&reply(Some(CaEnable::Possible)))),
        );
        let notes = d.take_notifications();
        assert!(
            notes.contains(&Notification::HotPlug(HotPlug::CardInserted)),
            "expected CardInserted, got {notes:?}"
        );

        // true -> false: card removed.
        feed(&mut d, r_apdu(CA_SESSION, &ser(&reply(None))));
        let notes = d.take_notifications();
        assert!(
            notes.contains(&Notification::HotPlug(HotPlug::CardRemoved)),
            "expected CardRemoved, got {notes:?}"
        );
    }

    #[test]
    fn ca_pmt_reply_surfaces_typed_ca_enable() {
        use dvb_ci::objects::ca_pmt_reply::{CaEnable, CaPmtReply};

        let mut d = driver_with_sessions();
        d.take_notifications();

        // `CA_enable` = 0x03 (possible under conditions, technical dialogue) —
        // EN 50221 §8.4.3.5 Table 26.
        feed(
            &mut d,
            r_apdu(
                CA_SESSION,
                &ser(&CaPmtReply {
                    program_number: 7,
                    version_number: 1,
                    current_next_indicator: true,
                    ca_enable: Some(CaEnable::PossibleTechnicalDialogue),
                    streams: vec![],
                }),
            ),
        );
        let notes = d.take_notifications();
        assert!(
            notes.contains(&Notification::CaPmtReply {
                program_number: 7,
                ca_enable: Some(CaEnable::PossibleTechnicalDialogue),
                descrambling_ok: true,
            }),
            "expected typed ca_enable on CaPmtReply, got {notes:?}"
        );
    }

    #[test]
    fn ca_pmt_reply_flag_clear_surfaces_none() {
        use dvb_ci::objects::ca_pmt_reply::CaPmtReply;

        let mut d = driver_with_sessions();
        d.take_notifications();

        // Programme `CA_enable_flag` clear -> no programme-level status given
        // — EN 50221 §8.4.3.5 Table 26.
        feed(
            &mut d,
            r_apdu(
                CA_SESSION,
                &ser(&CaPmtReply {
                    program_number: 7,
                    version_number: 1,
                    current_next_indicator: true,
                    ca_enable: None,
                    streams: vec![],
                }),
            ),
        );
        let notes = d.take_notifications();
        assert!(
            notes.contains(&Notification::CaPmtReply {
                program_number: 7,
                ca_enable: None,
                descrambling_ok: false,
            }),
            "expected ca_enable None on flag-clear CaPmtReply, got {notes:?}"
        );
    }

    #[test]
    fn mmi_no_card_text_infers_card_removed() {
        use dvb_ci::objects::mmi_high::Enq;

        let mut d = driver_with_sessions();
        d.take_notifications();

        feed(
            &mut d,
            r_apdu(
                MMI_SESSION,
                &ser(&Enq {
                    blind_answer: false,
                    answer_text_length: 0,
                    text_chars: b"NO CARD detected - please insert your smart card",
                }),
            ),
        );

        let notes = d.take_notifications();
        assert!(
            notes.contains(&Notification::HotPlug(HotPlug::CardRemoved)),
            "expected CardRemoved inferred from MMI 'no card' text, got {notes:?}"
        );
    }

    /// r10-O-13: the MMI keyword heuristic reports a card *transition*, not
    /// every menu that repeats the same text.
    #[test]
    fn mmi_card_keyword_heuristic_is_edge_triggered() {
        use dvb_ci::objects::mmi_high::Enq;

        let enq = |text: &'static [u8]| {
            r_apdu(
                MMI_SESSION,
                &ser(&Enq {
                    blind_answer: false,
                    answer_text_length: 0,
                    text_chars: text,
                }),
            )
        };
        let mut d = driver_with_sessions();
        d.take_notifications();
        let count = |notes: &[Notification], hp: HotPlug| {
            notes
                .iter()
                .filter(|n| **n == Notification::HotPlug(hp))
                .count()
        };

        // The same "no card" prompt three times: one CardRemoved, not three.
        for _ in 0..3 {
            feed(&mut d, enq(b"No card - please insert your smart card"));
        }
        let notes = d.take_notifications();
        assert_eq!(count(&notes, HotPlug::CardRemoved), 1, "{notes:?}");
        assert_eq!(count(&notes, HotPlug::CardInserted), 0);

        // A present-card prompt is an edge: one CardInserted, then silence.
        for _ in 0..2 {
            feed(&mut d, enq(b"Your entitlement is valid"));
        }
        let notes = d.take_notifications();
        assert_eq!(count(&notes, HotPlug::CardInserted), 1, "{notes:?}");
        assert_eq!(count(&notes, HotPlug::CardRemoved), 0);

        // Back to absent: a fresh edge, reported once.
        feed(&mut d, enq(b"Card removed"));
        feed(&mut d, enq(b"Card removed"));
        let notes = d.take_notifications();
        assert_eq!(count(&notes, HotPlug::CardRemoved), 1, "{notes:?}");
    }

    #[test]
    fn pump_hotplug_delivers_cam_present_via_closure_exactly_once() {
        let mut dev = MockCaDevice::new([]);
        dev.slot = SlotInfo {
            num: 0,
            module_ready: false,
            module_present: false,
        };
        let mut d = Driver::new(dev);
        d.init().unwrap();
        d.take_notifications(); // drop the baseline observation

        // Module physically inserted and ready.
        d.device_mut().slot = SlotInfo {
            num: 0,
            module_ready: true,
            module_present: true,
        };

        let mut seen = Vec::new();
        d.pump_hotplug(Duration::from_millis(10), |hp| seen.push(hp))
            .unwrap();

        assert_eq!(
            seen,
            vec![HotPlug::CamPresent],
            "expected the closure to receive HotPlug::CamPresent exactly once, got {seen:?}"
        );
    }

    // --- #763 Task 3: ManagedCa + add_service ---

    /// A `CA_descriptor` TLV (ISO/IEC 13818-1 §2.6.16): tag `0x09`, len `4`,
    /// `CA_system_id`(2), `reserved(3)`/`CA_PID`(13).
    pub(crate) fn ca_descriptor(ca_system_id: u16, pid: u16) -> [u8; 6] {
        [
            0x09,
            0x04,
            (ca_system_id >> 8) as u8,
            ca_system_id as u8,
            0xE0 | ((pid >> 8) as u8 & 0x1F),
            pid as u8,
        ]
    }

    /// A synthetic scrambled-service PMT: programme-level `CA_descriptor`
    /// (`CA_system_id` `0x0500` = Viaccess, a real assigned value per the
    /// TSDuck CA-system registry consumed by `dvb_si::descriptors::ca::ca_system_name`),
    /// one scrambled H.264 video ES (own `CA_descriptor`), and one clear AAC
    /// audio ES.
    ///
    /// **Provenance:** no committed capture in this repo's fixture corpus
    /// carries a scrambled PMT — `fixtures/dvb-si/tnt-5w-12732v-isi6-10s.ts`'s
    /// five PMTs (verified via `cargo run -p dvb-tools -- dump ... --json`)
    /// are all clear/FTA services, and no CA-descriptor-bearing capture exists
    /// under `private/fixtures/` either. This hand-rolls the wire bytes per
    /// ISO/IEC 13818-1 §2.4.4.8's PMT syntax instead, mirroring the exact
    /// precedent already established by `dvb-ci/src/builder.rs`'s
    /// `build_test_pmt()` (a hand-rolled buffer "that mirrors a real
    /// CA-protected service") — real `CA_system_id`/`stream_type` values, real
    /// CRC, just not sourced from an off-air capture.
    pub(crate) fn build_ca_pmt_fixture(program_number: u16) -> Vec<u8> {
        const VIACCESS: u16 = 0x0500;
        let prog_ca = ca_descriptor(VIACCESS, 0x0064);
        let es0_ca = ca_descriptor(VIACCESS, 0x0065);

        let mut body = Vec::new();
        body.push(0x02); // table_id (PMT)
        body.push(0); // section_length placeholder (fixed up below)
        body.push(0);
        body.extend_from_slice(&program_number.to_be_bytes());
        body.push(0xC3); // reserved(2)='11' | version(5)=1 | current_next=1
        body.push(0x00); // section_number
        body.push(0x00); // last_section_number
        body.push(0xE0 | 0x01); // reserved(3) | PCR_PID(13) = 0x0100
        body.push(0x00);
        body.push(0xF0 | ((prog_ca.len() >> 8) as u8 & 0x0F));
        body.push(prog_ca.len() as u8);
        body.extend_from_slice(&prog_ca);
        // ES0: H.264 video, pid 0x0100, scrambled (own CA_descriptor).
        body.push(0x1B);
        body.push(0xE0 | 0x01);
        body.push(0x00);
        body.push(0xF0 | ((es0_ca.len() >> 8) as u8 & 0x0F));
        body.push(es0_ca.len() as u8);
        body.extend_from_slice(&es0_ca);
        // ES1: AAC ADTS audio, pid 0x0101, clear.
        body.push(0x0F);
        body.push(0xE0 | 0x01);
        body.push(0x01);
        body.push(0xF0);
        body.push(0x00);

        let section_length = body.len() - 3 + 4;
        body[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        body[2] = section_length as u8;
        let crc = broadcast_common::crc32_mpeg2::compute(&body);
        body.extend_from_slice(&crc.to_be_bytes());
        body
    }

    /// Same layout as [`build_ca_pmt_fixture`] but with the PCR carried on its
    /// own **dedicated** `PCR_PID` (`0x00FF`) — distinct from every ES PID
    /// (`0x0100`/`0x0101`) and CA PID (`0x0064`/`0x0065`) — a legitimate DVB
    /// config (ISO/IEC 13818-1 §2.4.4.8) that `build_ca_pmt_fixture`'s
    /// `PCR_PID == video ES PID` masks: the #763 final-review regression
    /// fixture for `required_pids`/`feed_ts` PCR routing.
    pub(crate) fn build_ca_pmt_fixture_dedicated_pcr(program_number: u16) -> Vec<u8> {
        const VIACCESS: u16 = 0x0500;
        let prog_ca = ca_descriptor(VIACCESS, 0x0064);
        let es0_ca = ca_descriptor(VIACCESS, 0x0065);

        let mut body = Vec::new();
        body.push(0x02); // table_id (PMT)
        body.push(0); // section_length placeholder (fixed up below)
        body.push(0);
        body.extend_from_slice(&program_number.to_be_bytes());
        body.push(0xC3); // reserved(2)='11' | version(5)=1 | current_next=1
        body.push(0x00); // section_number
        body.push(0x00); // last_section_number
        body.push(0xE0); // reserved(3) | PCR_PID(13) high byte = 0x00FF >> 8
        body.push(0xFF); // PCR_PID low byte — dedicated, outside the ES/CA set
        body.push(0xF0 | ((prog_ca.len() >> 8) as u8 & 0x0F));
        body.push(prog_ca.len() as u8);
        body.extend_from_slice(&prog_ca);
        // ES0: H.264 video, pid 0x0100, scrambled (own CA_descriptor).
        body.push(0x1B);
        body.push(0xE0 | 0x01);
        body.push(0x00);
        body.push(0xF0 | ((es0_ca.len() >> 8) as u8 & 0x0F));
        body.push(es0_ca.len() as u8);
        body.extend_from_slice(&es0_ca);
        // ES1: AAC ADTS audio, pid 0x0101, clear.
        body.push(0x0F);
        body.push(0xE0 | 0x01);
        body.push(0x01);
        body.push(0xF0);
        body.push(0x00);

        let section_length = body.len() - 3 + 4;
        body[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        body[2] = section_length as u8;
        let crc = broadcast_common::crc32_mpeg2::compute(&body);
        body.extend_from_slice(&crc.to_be_bytes());
        body
    }

    /// Same layout as [`build_ca_pmt_fixture`] but with no `CA_descriptor`
    /// anywhere (an ordinary clear/FTA service) — the negative-control PMT for
    /// [`CaError::NoCaDescriptor`].
    pub(crate) fn build_clear_pmt_fixture(program_number: u16) -> Vec<u8> {
        let mut body = Vec::new();
        body.push(0x02);
        body.push(0);
        body.push(0);
        body.extend_from_slice(&program_number.to_be_bytes());
        body.push(0xC3);
        body.push(0x00);
        body.push(0x00);
        body.push(0xE0 | 0x01);
        body.push(0x00);
        body.push(0xF0); // program_info_length = 0
        body.push(0x00);
        // ES0: H.264 video, pid 0x0100, clear.
        body.push(0x1B);
        body.push(0xE0 | 0x01);
        body.push(0x00);
        body.push(0xF0);
        body.push(0x00);

        let section_length = body.len() - 3 + 4;
        body[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        body[2] = section_length as u8;
        let crc = broadcast_common::crc32_mpeg2::compute(&body);
        body.extend_from_slice(&crc.to_be_bytes());
        body
    }

    #[test]
    fn add_service_builds_and_sends_ca_pmt_matching_builder_oracle() {
        use broadcast_common::Parse;

        let mut d = driver_with_sessions();
        d.take_notifications();

        let pmt_bytes = build_ca_pmt_fixture(1546);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();

        d.add_service(&pmt).unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();

        // Oracle: the same PMT built directly via dvb_ci::builder::build_ca_pmt
        // with `Only` (first-ever service on an empty managed set) +
        // `ok_descrambling`.
        let expected =
            build_ca_pmt(&pmt, CaPmtListManagement::Only, CaPmtCmdId::OkDescrambling).to_bytes();
        assert_apdu_on_session(&d, CA_SESSION, &expected);

        // The service was recorded with its ES/CA PIDs.
        let svc = d
            .managed_ca()
            .services()
            .get(&1546)
            .expect("program_number 1546 must be tracked after add_service");
        assert_eq!(svc.es_pids, vec![0x0100, 0x0101]);
        assert_eq!(svc.ca_pids, vec![0x0064, 0x0065]);
        assert_eq!(svc.cmd, CaPmtCmdId::OkDescrambling);
        assert_eq!(svc.last_ca_enable, None);
    }

    #[test]
    fn add_service_rejects_pmt_without_ca_descriptor() {
        use broadcast_common::Parse;

        let mut d = driver_with_sessions();
        let pmt_bytes = build_clear_pmt_fixture(999);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();

        let err = d.add_service(&pmt).unwrap_err();
        assert!(
            matches!(
                err,
                CaError::NoCaDescriptor {
                    program_number: 999
                }
            ),
            "expected NoCaDescriptor{{program_number: 999}}, got {err:?}"
        );
        assert!(
            d.managed_ca().services().is_empty(),
            "a rejected PMT must not be recorded"
        );
    }

    /// r10-W-20: the re-query timer must surface a corrupt stored PMT as an
    /// `Err` from the public `pump` entry point — never a panic — and the
    /// failure must be sticky: a corrupt (or CAM-supplied) service must not
    /// silently swallow the healthy services' resend, and must keep
    /// reporting on every later tick. A service with truncated stored
    /// `pmt_raw` (corruption of the caller's own state after a successful
    /// add — the only corrupt shape the stored bytes can hold, since
    /// `add_service` re-serializes them) joins a healthy service added
    /// through the public `add_service`; when the 5s timer fires, the
    /// healthy pair still goes out, the corrupt one's re-parse failure is
    /// recorded, and `pump` returns it as `Err` — twice in a row.
    #[test]
    fn requery_tick_serialize_failure_errors_from_pump_instead_of_panicking() {
        use broadcast_common::Parse;

        let clock = TestClock::new();
        let mut d = driver_with_sessions().with_clock(clock.as_fn());

        // A healthy service through the public path (feeds its CaInfo like
        // `requery_timer_resends_ca_pmt_then_reply_change_emits_one_entitlement`).
        let good_bytes = build_ca_pmt_fixture(1);
        let good = PmtSection::parse(&good_bytes).unwrap();
        assert_eq!(good.program_number, 1, "fixture precondition");
        d.add_service(&good).unwrap();

        // The corrupt service, recorded via the same internals
        // `add_service` uses (its raw bytes cannot be written by the
        // serializer, so they never reach the stored set through the
        // public path).
        let pmt_bytes = build_ca_pmt_fixture(1546);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();
        let built = d
            .build_ca_pmt_filtered(
                &pmt,
                CaPmtListManagement::Update,
                CaPmtCmdId::OkDescrambling,
            )
            .try_to_bytes()
            .expect("fixture precondition: the good PMT encodes");
        let corrupt_raw = pmt_bytes[..pmt_bytes.len() - 6].to_vec();
        d.managed.record(
            1546,
            managed::service_of(&pmt, CaPmtCmdId::OkDescrambling, built, corrupt_raw),
        );
        d.take_notifications();
        // Drain every queued host write with module T_SBs (the transport
        // flushes one write per module turn, #337; `feed`'s tail leaves
        // handshake writes and `add_service` added one more).
        for _ in 0..16 {
            d.device_mut().inbound.push_back(sb());
            d.pump(Duration::from_millis(10)).unwrap();
        }
        d.set_requery_interval(Duration::from_secs(5));

        // Advance past the interval and pump idle: the timer fires, the
        // healthy service's re-query pair goes out, and the corrupt one
        // fails its re-parse — an Err out of `pump`, not a panic.
        clock.advance(Duration::from_secs(5));
        let err = d
            .pump(Duration::ZERO)
            .expect_err("re-query of a corrupt stored PMT must error, not panic");
        assert!(
            matches!(
                err.get_ref(),
                Some(e) if e.downcast_ref::<CaError>()
                    .is_some_and(|ca| matches!(ca, CaError::PmtParse(_)))
            ),
            "expected CaError::PmtParse from the corrupt stored bytes, got {err:?}"
        );
        assert!(
            err.to_string().contains("stored PMT re-parse failed"),
            "error must name the re-parse failure, got: {err}"
        );
        // Sticky-but-not-swallowing: the failure was returned after the
        // healthy pair went out this tick; drain it, and the NEXT fire
        // re-detects the same corrupt stored bytes (never silently quiet).
        assert!(d.pump(Duration::ZERO).is_ok());
        clock.advance(Duration::from_secs(5));
        let err2 = d
            .pump(Duration::ZERO)
            .expect_err("the corrupt service must fail every re-query");
        assert!(
            err2.to_string().contains("stored PMT re-parse failed"),
            "failure must persist, got: {err2}"
        );
    }

    /// The constructor-level corrupt PMT: a 5000-byte programme CA loop
    /// (CAID `0x0B00`), past dvb-ci's 4095-byte `program_info_length` cap.
    /// dvb-si's parse only 12-bit-checks lengths against the physical
    /// buffer, so this shape exists only via `PmtSection::new` — standing
    /// in for a corrupt reassembly reaching `add_service` as a value.
    fn corrupt_ca_section_ctor() -> PmtSection<'static> {
        const LOOP: usize = 5000;
        let mut prog_info: Vec<u8> = Vec::with_capacity(LOOP);
        for _ in 0..LOOP / 4 {
            // CA_descriptor: CAID 0x0B00, CA_PID 0x0100 (4-byte body).
            prog_info.extend_from_slice(&[0x09u8, 0x02, 0x0B, 0x00]);
        }
        assert_eq!(prog_info.len(), LOOP);
        PmtSection::new(
            1,
            1,
            true,
            0,
            0,
            0x0200,
            dvb_si::descriptors::DescriptorLoop::new(Box::leak(prog_info.into_boxed_slice())),
            Vec::new(),
        )
    }

    /// r10-W-20: `add_service` must reject a PMT whose CA loop cannot be
    /// encoded into a `ca_pmt` (over-range `program_info_length`) as
    /// `CaError::Serialize` from the public entry point — pre-fix this
    /// reached `.to_bytes()` and panicked inside the dvb-ci serializer.
    #[test]
    fn add_service_serialize_failure_returns_err_instead_of_panicking() {
        let mut d = driver_with_sessions();
        d.managed.set_cam_caids([0x0B00u16].into_iter().collect());
        let pmt = corrupt_ca_section_ctor();
        let err = d
            .add_service(&pmt)
            .expect_err("a ca_pmt with no valid encoding must error, not panic");
        assert!(
            matches!(err, CaError::Serialize(_)),
            "expected CaError::Serialize, got {err:?}"
        );
        assert!(
            d.managed_ca().services().is_empty(),
            "a rejected PMT must not be recorded"
        );
    }

    #[test]
    fn add_service_second_call_uses_add_list_management() {
        use broadcast_common::Parse;

        let mut d = driver_with_sessions();
        d.take_notifications();

        let pmt1_bytes = build_ca_pmt_fixture(1546);
        let pmt1 = PmtSection::parse(&pmt1_bytes).unwrap();
        d.add_service(&pmt1).unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();

        let pmt2_bytes = build_ca_pmt_fixture(1547);
        let pmt2 = PmtSection::parse(&pmt2_bytes).unwrap();
        d.add_service(&pmt2).unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();

        // Second service joins an already-active set → `Add`, not `Only`.
        let expected2 =
            build_ca_pmt(&pmt2, CaPmtListManagement::Add, CaPmtCmdId::OkDescrambling).to_bytes();
        assert_apdu_on_session(&d, CA_SESSION, &expected2);

        assert_eq!(d.managed_ca().services().len(), 2);
    }

    // --- #1067: managed add_service/requery must CAID-filter like the raw
    // `descramble` path already does ---

    /// Same layout as [`build_ca_pmt_fixture`] but with TWO programme-level
    /// `CA_descriptor`s (Viaccess `0x0500` and Nagravision `0x1800` — both
    /// real assigned values per the TSDuck CA-system registry, mirroring
    /// `dvb_ci::builder`'s own two-CAID `build_test_pmt` fixture) and no
    /// ES-level CA — the #1067 regression fixture: a CAM that advertises
    /// only one of the two CAIDs must have the other filtered out of the
    /// sent `ca_pmt`.
    fn build_ca_pmt_fixture_two_caids(program_number: u16) -> Vec<u8> {
        const VIACCESS: u16 = 0x0500;
        const NAGRA: u16 = 0x1800;
        let prog_ca_a = ca_descriptor(VIACCESS, 0x0064);
        let prog_ca_b = ca_descriptor(NAGRA, 0x0066);
        let mut prog_ca = Vec::new();
        prog_ca.extend_from_slice(&prog_ca_a);
        prog_ca.extend_from_slice(&prog_ca_b);

        let mut body = Vec::new();
        body.push(0x02); // table_id (PMT)
        body.push(0);
        body.push(0);
        body.extend_from_slice(&program_number.to_be_bytes());
        body.push(0xC3);
        body.push(0x00);
        body.push(0x00);
        body.push(0xE0 | 0x01); // PCR_PID = 0x0100
        body.push(0x00);
        body.push(0xF0 | ((prog_ca.len() >> 8) as u8 & 0x0F));
        body.push(prog_ca.len() as u8);
        body.extend_from_slice(&prog_ca);
        // ES0: clear AAC audio, pid 0x0100 — no ES-level CA, so this
        // fixture isolates the programme-level filter.
        body.push(0x0F);
        body.push(0xE0 | 0x01);
        body.push(0x00);
        body.push(0xF0);
        body.push(0x00);

        let section_length = body.len() - 3 + 4;
        body[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        body[2] = section_length as u8;
        let crc = broadcast_common::crc32_mpeg2::compute(&body);
        body.extend_from_slice(&crc.to_be_bytes());
        body
    }

    #[test]
    fn add_service_filters_ca_descriptors_to_cam_advertised_caids() {
        use broadcast_common::Parse;
        use dvb_ci::objects::ca_info::CaInfo;

        let mut d = driver_with_sessions();
        d.take_notifications();

        // CAM advertises ONLY Viaccess (0x0500); the PMT carries both
        // Viaccess and Nagravision (0x1800) programme-level CA_descriptors.
        feed(
            &mut d,
            r_apdu(
                CA_SESSION,
                &ser(&CaInfo {
                    ca_system_ids: vec![0x0500],
                }),
            ),
        );
        d.take_notifications();

        let pmt_bytes = build_ca_pmt_fixture_two_caids(1560);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();

        // Oracle: `dvb_ci::builder::build_ca_pmt_for_caids` — the exact
        // filter the raw `descramble` path already applies
        // (`CiStack::build_ca_pmt_bytes`) — vs. the unfiltered `build_ca_pmt`
        // a pre-fix `add_service` sent.
        let expected_filtered = dvb_ci::builder::build_ca_pmt_for_caids(
            &pmt,
            &[0x0500],
            CaPmtListManagement::Only,
            CaPmtCmdId::OkDescrambling,
        )
        .to_bytes();
        let unfiltered =
            build_ca_pmt(&pmt, CaPmtListManagement::Only, CaPmtCmdId::OkDescrambling).to_bytes();
        assert_ne!(
            expected_filtered, unfiltered,
            "precondition: the fixture's two CAIDs must make filtered != unfiltered"
        );
        let sends_filtered_before = count_apdu_on_session(&d, CA_SESSION, &expected_filtered);
        let sends_unfiltered_before = count_apdu_on_session(&d, CA_SESSION, &unfiltered);

        d.add_service(&pmt).unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();

        assert_eq!(
            count_apdu_on_session(&d, CA_SESSION, &expected_filtered),
            sends_filtered_before + 1,
            "add_service must send the CAID-filtered ca_pmt"
        );
        assert_eq!(
            count_apdu_on_session(&d, CA_SESSION, &unfiltered),
            sends_unfiltered_before,
            "add_service must NOT send the unfiltered ca_pmt once the CAM's CAIDs are known"
        );
    }

    #[test]
    fn requery_timer_filters_ca_descriptors_to_cam_advertised_caids() {
        use broadcast_common::Parse;
        use dvb_ci::objects::ca_info::CaInfo;

        let clock = TestClock::new();
        let mut d = driver_with_sessions().with_clock(clock.as_fn());
        // #1032: re-query is opt-in (REQUERY_DEFAULT = Duration::ZERO) —
        // enable it explicitly to exercise the timer.
        d.set_requery_interval(Duration::from_secs(10));
        d.take_notifications();

        let pmt_bytes = build_ca_pmt_fixture_two_caids(1561);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();
        // add_service before any ca_info is known: unfiltered, per the
        // documented fallback (no CAID info to filter against yet).
        d.add_service(&pmt).unwrap();
        d.device_mut().inbound.push_back(sb());
        clock.advance(Duration::from_millis(10));
        d.pump(Duration::from_millis(10)).unwrap();

        // The CAM's ca_info now arrives, advertising only Viaccess (0x0500).
        feed(
            &mut d,
            r_apdu(
                CA_SESSION,
                &ser(&CaInfo {
                    ca_system_ids: vec![0x0500],
                }),
            ),
        );
        d.take_notifications();

        // #1032: the timer resends a `list_management = update` PAIR —
        // `query` then `ok_descrambling` — both CAID-filtered (#1067).
        let expected_query_filtered = dvb_ci::builder::build_ca_pmt_for_caids(
            &pmt,
            &[0x0500],
            CaPmtListManagement::Update,
            CaPmtCmdId::Query,
        )
        .to_bytes();
        let expected_ok_filtered = dvb_ci::builder::build_ca_pmt_for_caids(
            &pmt,
            &[0x0500],
            CaPmtListManagement::Update,
            CaPmtCmdId::OkDescrambling,
        )
        .to_bytes();
        let unfiltered_query =
            build_ca_pmt(&pmt, CaPmtListManagement::Update, CaPmtCmdId::Query).to_bytes();
        let unfiltered_ok = build_ca_pmt(
            &pmt,
            CaPmtListManagement::Update,
            CaPmtCmdId::OkDescrambling,
        )
        .to_bytes();
        assert_ne!(
            expected_query_filtered, unfiltered_query,
            "precondition: the fixture's two CAIDs must make filtered != unfiltered (query)"
        );
        assert_ne!(
            expected_ok_filtered, unfiltered_ok,
            "precondition: the fixture's two CAIDs must make filtered != unfiltered (ok_descrambling)"
        );
        let sends_query_before = count_apdu_on_session(&d, CA_SESSION, &expected_query_filtered);
        let sends_ok_before = count_apdu_on_session(&d, CA_SESSION, &expected_ok_filtered);
        let sends_unfiltered_query_before =
            count_apdu_on_session(&d, CA_SESSION, &unfiltered_query);
        let sends_unfiltered_ok_before = count_apdu_on_session(&d, CA_SESSION, &unfiltered_ok);

        clock.advance(Duration::from_secs(11));
        d.pump(Duration::from_secs(11)).unwrap();
        feed(&mut d, sb());

        assert_eq!(
            count_apdu_on_session(&d, CA_SESSION, &expected_query_filtered),
            sends_query_before + 1,
            "the re-query `query` half must be CAID-filtered to the CAM's advertised CAIDs"
        );
        assert_eq!(
            count_apdu_on_session(&d, CA_SESSION, &expected_ok_filtered),
            sends_ok_before + 1,
            "the re-query `ok_descrambling` half must be CAID-filtered to the CAM's advertised CAIDs"
        );
        assert_eq!(
            count_apdu_on_session(&d, CA_SESSION, &unfiltered_query),
            sends_unfiltered_query_before,
            "the re-query resend must NOT be the unfiltered `query` ca_pmt"
        );
        assert_eq!(
            count_apdu_on_session(&d, CA_SESSION, &unfiltered_ok),
            sends_unfiltered_ok_before,
            "the re-query resend must NOT be the unfiltered `ok_descrambling` ca_pmt"
        );
    }

    // --- #763 Task 4: set_cat + emm_pids/descramble_pids ---

    /// A hand-built CAT section (ISO/IEC 13818-1 §2.4.4.5): table_id 0x01, a
    /// flat descriptor loop of `CA_descriptor`s (EN 300 468 §6.2.16, tag
    /// 0x09; the `ca_descriptor` helper above builds the same TLV used for
    /// PMTs). No off-air CAT capture exists in this repo's fixture corpus
    /// (verified: none of the committed `.ts` captures carry PID 0x0001),
    /// mirroring the same hand-rolled-fixture precedent as
    /// `build_ca_pmt_fixture` and `dvb_si::tables::cat`'s own unit tests.
    pub(crate) fn build_cat_fixture(descriptors: &[u8]) -> Vec<u8> {
        const EXTENSION_HEADER_LEN: u16 = 5;
        const CRC_LEN: u16 = 4;
        let section_length = EXTENSION_HEADER_LEN + descriptors.len() as u16 + CRC_LEN;
        let mut v = Vec::new();
        v.push(0x01); // table_id (CAT)
        v.push(0xB0 | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(&[0xFF, 0xFF]); // table_id_extension (reserved for CAT)
        v.push(0xC1); // reserved(2)='11' | version(5)=0 | current_next=1
        v.push(0x00); // section_number
        v.push(0x00); // last_section_number
        v.extend_from_slice(descriptors);
        let crc = broadcast_common::crc32_mpeg2::compute(&v);
        v.extend_from_slice(&crc.to_be_bytes());
        v
    }

    #[test]
    fn set_cat_computes_emm_pids_as_cat_inter_ca_info_caids() {
        use broadcast_common::Parse;
        use dvb_ci::objects::ca_info::CaInfo;
        use dvb_si::tables::cat::CatSection;

        let mut d = driver_with_sessions();
        d.take_notifications();

        // ca_info arrives first: the CAM advertises CAIDs 0x0648, 0x0100.
        feed(
            &mut d,
            r_apdu(
                CA_SESSION,
                &ser(&CaInfo {
                    ca_system_ids: vec![0x0648, 0x0100],
                }),
            ),
        );
        d.take_notifications();

        // CAT maps 0x0648 -> 0x1FF0 (advertised) and 0x0500 -> 0x1FF1 (not
        // advertised by this CAM).
        let mut descriptors = Vec::new();
        descriptors.extend_from_slice(&ca_descriptor(0x0648, 0x1FF0));
        descriptors.extend_from_slice(&ca_descriptor(0x0500, 0x1FF1));
        let cat_bytes = build_cat_fixture(&descriptors);
        let cat = CatSection::parse(&cat_bytes).unwrap();

        d.set_cat(&cat).unwrap();

        assert_eq!(
            d.emm_pids(),
            &[0x1FF0],
            "0x0500 -> 0x1FF1 must be excluded: the CAM never advertised CAID 0x0500"
        );
    }

    #[test]
    fn set_cat_before_ca_info_is_not_an_error_and_recomputes_once_ca_info_arrives() {
        use broadcast_common::Parse;
        use dvb_ci::objects::ca_info::CaInfo;
        use dvb_si::tables::cat::CatSection;

        let mut d = driver_with_sessions();
        d.take_notifications();

        let mut descriptors = Vec::new();
        descriptors.extend_from_slice(&ca_descriptor(0x0648, 0x1FF0));
        descriptors.extend_from_slice(&ca_descriptor(0x0500, 0x1FF1));
        let cat_bytes = build_cat_fixture(&descriptors);
        let cat = CatSection::parse(&cat_bytes).unwrap();

        // set_cat with no ca_info observed yet: not an error, emm_pids stays
        // empty (nothing to intersect against).
        d.set_cat(&cat).unwrap();
        assert!(
            d.emm_pids().is_empty(),
            "emm_pids must be empty before any ca_info arrives, got {:?}",
            d.emm_pids()
        );

        // ca_info now arrives: emm_pids recomputes against the CAT stored
        // earlier, without a second set_cat call.
        feed(
            &mut d,
            r_apdu(
                CA_SESSION,
                &ser(&CaInfo {
                    ca_system_ids: vec![0x0648, 0x0100],
                }),
            ),
        );
        d.take_notifications();

        assert_eq!(
            d.emm_pids(),
            &[0x1FF0],
            "emm_pids must recompute once ca_info arrives, using the CAT stored by the earlier set_cat"
        );
    }

    /// Task 4 review fix (MEDIUM): `recompute_emm_pids` must dedup like its
    /// sibling `recompute_service_pids` does — two CAT `CA_descriptor`s
    /// (distinct `CA_system_id`s, both CAM-advertised) that happen to share
    /// one `EMM_PID` (a real multi-CAS-on-one-EMM-PID broadcast setup) must
    /// list that PID exactly once, not twice.
    #[test]
    fn set_cat_emm_pids_dedups_when_two_caids_share_one_emm_pid() {
        use broadcast_common::Parse;
        use dvb_ci::objects::ca_info::CaInfo;
        use dvb_si::tables::cat::CatSection;

        let mut d = driver_with_sessions();
        d.take_notifications();

        // CAM advertises both CAIDs.
        feed(
            &mut d,
            r_apdu(
                CA_SESSION,
                &ser(&CaInfo {
                    ca_system_ids: vec![0x0648, 0x0100],
                }),
            ),
        );
        d.take_notifications();

        // CAT maps BOTH CAIDs to the SAME EMM PID.
        let mut descriptors = Vec::new();
        descriptors.extend_from_slice(&ca_descriptor(0x0648, 0x1FF0));
        descriptors.extend_from_slice(&ca_descriptor(0x0100, 0x1FF0));
        let cat_bytes = build_cat_fixture(&descriptors);
        let cat = CatSection::parse(&cat_bytes).unwrap();

        d.set_cat(&cat).unwrap();

        assert_eq!(
            d.emm_pids(),
            &[0x1FF0],
            "0x1FF0 must appear exactly once even though two CAM-advertised CAIDs map to it, got {:?}",
            d.emm_pids()
        );
    }

    /// Same layout as [`build_ca_pmt_fixture`] but with a distinct PCR/ES PID
    /// set, so a second added service proves `descramble_pids` is a real
    /// union rather than one programme's PIDs happening to repeat.
    fn build_ca_pmt_fixture_distinct_pids(program_number: u16) -> Vec<u8> {
        const VIACCESS: u16 = 0x0500;
        let prog_ca = ca_descriptor(VIACCESS, 0x0074);
        let es0_ca = ca_descriptor(VIACCESS, 0x0075);

        let mut body = Vec::new();
        body.push(0x02); // table_id (PMT)
        body.push(0);
        body.push(0);
        body.extend_from_slice(&program_number.to_be_bytes());
        body.push(0xC3);
        body.push(0x00);
        body.push(0x00);
        body.push(0xE0 | 0x02); // PCR_PID = 0x0200
        body.push(0x00);
        body.push(0xF0 | ((prog_ca.len() >> 8) as u8 & 0x0F));
        body.push(prog_ca.len() as u8);
        body.extend_from_slice(&prog_ca);
        // ES0: H.264 video, pid 0x0200, scrambled.
        body.push(0x1B);
        body.push(0xE0 | 0x02);
        body.push(0x00);
        body.push(0xF0 | ((es0_ca.len() >> 8) as u8 & 0x0F));
        body.push(es0_ca.len() as u8);
        body.extend_from_slice(&es0_ca);
        // ES1: AAC ADTS audio, pid 0x0201, clear.
        body.push(0x0F);
        body.push(0xE0 | 0x02);
        body.push(0x01);
        body.push(0xF0);
        body.push(0x00);

        let section_length = body.len() - 3 + 4;
        body[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        body[2] = section_length as u8;
        let crc = broadcast_common::crc32_mpeg2::compute(&body);
        body.extend_from_slice(&crc.to_be_bytes());
        body
    }

    #[test]
    fn descramble_pids_is_the_union_of_active_services_es_pids() {
        use broadcast_common::Parse;

        let mut d = driver_with_sessions();
        d.take_notifications();

        assert!(
            d.descramble_pids().is_empty(),
            "no service added yet: descramble_pids must be empty"
        );

        let pmt1_bytes = build_ca_pmt_fixture(1546);
        let pmt1 = PmtSection::parse(&pmt1_bytes).unwrap();
        d.add_service(&pmt1).unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();

        assert_eq!(d.descramble_pids(), &[0x0100, 0x0101]);

        let pmt2_bytes = build_ca_pmt_fixture_distinct_pids(1547);
        let pmt2 = PmtSection::parse(&pmt2_bytes).unwrap();
        d.add_service(&pmt2).unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();

        // Union of both programmes' ES PIDs, sorted.
        assert_eq!(
            d.descramble_pids(),
            &[0x0100, 0x0101, 0x0200, 0x0201],
            "descramble_pids must be the union across both added services"
        );
    }

    // --- #763 Task 5: re-query timer + edge-triggered Entitlement ---

    /// Build a `ca_pmt_reply` (EN 50221 §8.4.3.5, Table 26) for `program_number`
    /// carrying programme-level `ca_enable` (`None` = `CA_enable_flag` clear).
    pub(crate) fn ca_pmt_reply_for(
        program_number: u16,
        ca_enable: Option<dvb_ci::objects::ca_pmt_reply::CaEnable>,
    ) -> dvb_ci::objects::ca_pmt_reply::CaPmtReply {
        dvb_ci::objects::ca_pmt_reply::CaPmtReply {
            program_number,
            version_number: 1,
            current_next_indicator: true,
            ca_enable,
            streams: vec![],
        }
    }

    #[test]
    fn requery_timer_resends_ca_pmt_then_reply_change_emits_one_entitlement() {
        use broadcast_common::Parse;
        use dvb_ci::objects::ca_pmt_reply::CaEnable;

        let clock = TestClock::new();
        let mut d = driver_with_sessions().with_clock(clock.as_fn());
        // #1032: re-query is opt-in (REQUERY_DEFAULT = Duration::ZERO) —
        // enable it explicitly to exercise the timer.
        d.set_requery_interval(Duration::from_secs(10));
        d.take_notifications();

        let pmt_bytes = build_ca_pmt_fixture(1546);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();
        d.add_service(&pmt).unwrap();
        d.device_mut().inbound.push_back(sb());
        clock.advance(Duration::from_millis(10));
        d.pump(Duration::from_millis(10)).unwrap();
        d.take_notifications();

        // The initial `add_service` send is `ok_descrambling` — assert it
        // happened, so the test proves the resend below (a distinct `query`
        // cmd_id) is a genuinely different wire message, not the same bytes.
        let expected_initial_ca_pmt =
            build_ca_pmt(&pmt, CaPmtListManagement::Only, CaPmtCmdId::OkDescrambling).to_bytes();
        assert_apdu_on_session(&d, CA_SESSION, &expected_initial_ca_pmt);

        // #1032: the re-query timer resends a `list_management = update`
        // PAIR — `query` (to solicit a fresh `ca_pmt_reply`) immediately
        // followed by `ok_descrambling` (so descrambling is never left
        // barred, EN 50221 §8.4.3.5, whether or not the CAM answers the
        // query) — never the pre-fix `only`/`first` + `query` alone.
        let expected_query =
            build_ca_pmt(&pmt, CaPmtListManagement::Update, CaPmtCmdId::Query).to_bytes();
        let expected_ok_descrambling = build_ca_pmt(
            &pmt,
            CaPmtListManagement::Update,
            CaPmtCmdId::OkDescrambling,
        )
        .to_bytes();
        let query_sends_before = count_apdu_on_session(&d, CA_SESSION, &expected_query);
        let ok_sends_before = count_apdu_on_session(&d, CA_SESSION, &expected_ok_descrambling);

        let mut all_notes = Vec::new();

        // Reply 1: not entitled (baseline — first-ever reply for this
        // program; `descrambling_ok` is derived: `NotPossibleNoEntitlement`
        // is not in the "possible" set, so `false`).
        feed(
            &mut d,
            r_apdu(
                CA_SESSION,
                &ser(&ca_pmt_reply_for(
                    1546,
                    Some(CaEnable::NotPossibleNoEntitlement),
                )),
            ),
        );
        all_notes.extend(d.take_notifications());

        // Advance the clock past the 10s re-query interval: a single pump
        // ticks the stack with elapsed = 11s (nothing readable this turn),
        // which the #763 Task 5 re-query timer picks up and queues the
        // tracked service's `query`+`ok_descrambling` pair for resend (EN
        // 50221 §8.4.3.4 Table 25). EN 50221's link is half-duplex — this
        // tick's own keep-alive poll already claimed the turn, so the pair
        // is written across the module's next two `T_SB`s (the #337
        // one-write-per-turn rule), same as any other queued host write in
        // this test suite.
        clock.advance(Duration::from_secs(11));
        d.pump(Duration::from_secs(11)).unwrap();
        all_notes.extend(d.take_notifications());
        feed(&mut d, sb());

        assert_eq!(
            count_apdu_on_session(&d, CA_SESSION, &expected_query),
            query_sends_before + 1,
            "expected the re-query timer to resend the `query` half exactly once"
        );
        assert_eq!(
            count_apdu_on_session(&d, CA_SESSION, &expected_ok_descrambling),
            ok_sends_before + 1,
            "expected the re-query timer to resend the `ok_descrambling` half exactly once"
        );

        // Reply 2: the CAM's re-evaluated answer to the re-query says
        // descrambling is now possible.
        feed(
            &mut d,
            r_apdu(
                CA_SESSION,
                &ser(&ca_pmt_reply_for(1546, Some(CaEnable::Possible))),
            ),
        );
        all_notes.extend(d.take_notifications());

        let hits = all_notes
            .iter()
            .filter(|n| {
                matches!(
                    n,
                    Notification::Entitlement {
                        program_number: 1546,
                        ca_enable: CaEnable::Possible,
                        descrambling_ok: true,
                    }
                )
            })
            .count();
        assert_eq!(
            hits, 1,
            "expected exactly one Entitlement{{program_number:1546, ca_enable:Possible, descrambling_ok:true}}, got {all_notes:?}"
        );
    }

    #[test]
    fn requery_timer_unchanged_reply_across_two_requeries_emits_no_entitlement() {
        use broadcast_common::Parse;
        use dvb_ci::objects::ca_pmt_reply::CaEnable;

        let mut d = driver_with_sessions();
        // #1032: re-query is opt-in (REQUERY_DEFAULT = Duration::ZERO) —
        // enable it explicitly so the loop below exercises real re-queries.
        d.set_requery_interval(Duration::from_secs(10));
        d.take_notifications();

        let pmt_bytes = build_ca_pmt_fixture(1547);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();
        d.add_service(&pmt).unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();
        d.take_notifications();

        // Baseline reply: descrambling possible. First-ever reply — this
        // establishes the baseline and DOES emit once (per the transition
        // rule); drop it so the loop below only asserts on the re-queries.
        feed(
            &mut d,
            r_apdu(
                CA_SESSION,
                &ser(&ca_pmt_reply_for(1547, Some(CaEnable::Possible))),
            ),
        );
        d.take_notifications();

        // Two re-queries, the CAM replying with the SAME unchanged status
        // both times: no Entitlement either time (negative control).
        for _ in 0..2 {
            d.pump(Duration::from_secs(11)).unwrap();
            d.take_notifications();
            feed(
                &mut d,
                r_apdu(
                    CA_SESSION,
                    &ser(&ca_pmt_reply_for(1547, Some(CaEnable::Possible))),
                ),
            );
            let notes = d.take_notifications();
            assert!(
                !notes
                    .iter()
                    .any(|n| matches!(n, Notification::Entitlement { .. })),
                "unchanged status across a re-query must not emit Entitlement, got {notes:?}"
            );
        }
    }

    #[test]
    fn requery_reply_withdrawn_to_none_emits_no_entitlement() {
        use broadcast_common::Parse;
        use dvb_ci::objects::ca_pmt_reply::CaEnable;

        let mut d = driver_with_sessions();
        d.take_notifications();

        let pmt_bytes = build_ca_pmt_fixture(1548);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();
        d.add_service(&pmt).unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();
        d.take_notifications();

        // Baseline: descrambling possible (drop the baseline Entitlement).
        feed(
            &mut d,
            r_apdu(
                CA_SESSION,
                &ser(&ca_pmt_reply_for(1548, Some(CaEnable::Possible))),
            ),
        );
        d.take_notifications();

        // Programme `CA_enable_flag` now clear (`None`) — status withdrawn.
        // Per the transition rule this NEVER emits Entitlement (#726 HotPlug
        // covers the coarse withdrawal signal instead).
        feed(
            &mut d,
            r_apdu(CA_SESSION, &ser(&ca_pmt_reply_for(1548, None))),
        );
        let notes = d.take_notifications();
        assert!(
            !notes
                .iter()
                .any(|n| matches!(n, Notification::Entitlement { .. })),
            "ca_enable transitioning to None must not emit Entitlement, got {notes:?}"
        );
    }

    #[test]
    fn requery_disabled_by_default_sends_nothing() {
        use broadcast_common::Parse;

        // #1032: no `set_requery_interval` call at all — `REQUERY_DEFAULT`
        // (`Duration::ZERO`) must leave re-query off by default (the live
        // AlphaCrypt `stack.rs` notes never answers `query`, so the `query`
        // half is not hardware-verified).
        let mut d = driver_with_sessions();
        d.take_notifications();

        let pmt_bytes = build_ca_pmt_fixture(1549);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();
        d.add_service(&pmt).unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();

        // Count the `update`-variant bytes — the ones the timer would
        // resend if enabled — not the `only` bytes `add_service` sent.
        let expected_query =
            build_ca_pmt(&pmt, CaPmtListManagement::Update, CaPmtCmdId::Query).to_bytes();
        let expected_ok_descrambling = build_ca_pmt(
            &pmt,
            CaPmtListManagement::Update,
            CaPmtCmdId::OkDescrambling,
        )
        .to_bytes();
        let query_sends_before = count_apdu_on_session(&d, CA_SESSION, &expected_query);
        let ok_sends_before = count_apdu_on_session(&d, CA_SESSION, &expected_ok_descrambling);

        // Even a very long tick must not trigger a re-query by default.
        d.pump(Duration::from_secs(1_000_000)).unwrap();
        feed(&mut d, sb());
        feed(&mut d, sb());
        assert_eq!(
            count_apdu_on_session(&d, CA_SESSION, &expected_query),
            query_sends_before,
            "the default (Duration::ZERO) must never send a re-query `query`"
        );
        assert_eq!(
            count_apdu_on_session(&d, CA_SESSION, &expected_ok_descrambling),
            ok_sends_before,
            "the default (Duration::ZERO) must never send a re-query `ok_descrambling`"
        );
    }

    #[test]
    fn set_requery_interval_zero_disables_resend_after_being_enabled() {
        use broadcast_common::Parse;

        let mut d = driver_with_sessions();
        // Enable, then explicitly disable again — proves `Duration::ZERO`
        // works as an override, not merely as an unexercised default.
        d.set_requery_interval(Duration::from_secs(5));
        d.set_requery_interval(Duration::ZERO);
        d.take_notifications();

        let pmt_bytes = build_ca_pmt_fixture(1549);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();
        d.add_service(&pmt).unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();

        // Count the `update`-variant bytes — the ones the timer would resend
        // if it fired — not the `only` bytes `add_service` sent (both are
        // `cmd_id = ok_descrambling`; they differ only in `list_management`).
        let expected_ca_pmt = build_ca_pmt(
            &pmt,
            CaPmtListManagement::Update,
            CaPmtCmdId::OkDescrambling,
        )
        .to_bytes();
        let sends_before = count_apdu_on_session(&d, CA_SESSION, &expected_ca_pmt);

        // Even a very long tick must not trigger a re-query once disabled.
        d.pump(Duration::from_secs(1000)).unwrap();
        feed(&mut d, sb());
        feed(&mut d, sb());

        let sends_after = count_apdu_on_session(&d, CA_SESSION, &expected_ca_pmt);
        assert_eq!(
            sends_after, sends_before,
            "Duration::ZERO must disable the re-query resend"
        );
    }

    #[test]
    fn requery_timer_resends_every_active_service_not_just_one() {
        use broadcast_common::Parse;

        let clock = TestClock::new();
        let mut d = driver_with_sessions().with_clock(clock.as_fn());
        // #1032: re-query is opt-in (REQUERY_DEFAULT = Duration::ZERO) —
        // enable it explicitly to exercise the timer.
        d.set_requery_interval(Duration::from_secs(10));
        d.take_notifications();

        // Two services on the managed set: 1546 (`Only`, first-ever) and
        // 1547 (`Add`, joining the active set).
        let pmt1_bytes = build_ca_pmt_fixture(1546);
        let pmt1 = PmtSection::parse(&pmt1_bytes).unwrap();
        d.add_service(&pmt1).unwrap();
        d.device_mut().inbound.push_back(sb());
        clock.advance(Duration::from_millis(10));
        d.pump(Duration::from_millis(10)).unwrap();

        let pmt2_bytes = build_ca_pmt_fixture(1547);
        let pmt2 = PmtSection::parse(&pmt2_bytes).unwrap();
        d.add_service(&pmt2).unwrap();
        d.device_mut().inbound.push_back(sb());
        clock.advance(Duration::from_millis(10));
        d.pump(Duration::from_millis(10)).unwrap();
        d.take_notifications();

        // The `update`-variant PAIR the re-query timer rebuilds for each
        // service (#1032: `list_management = update`, `query` then
        // `ok_descrambling`, for every active service — `update` acts only
        // at programme level per EN 50221 §8.4.3.4, so unlike the pre-fix
        // `first`/`more`/`last` scheme it needs no position bookkeeping).
        let expected1_query =
            build_ca_pmt(&pmt1, CaPmtListManagement::Update, CaPmtCmdId::Query).to_bytes();
        let expected1_ok = build_ca_pmt(
            &pmt1,
            CaPmtListManagement::Update,
            CaPmtCmdId::OkDescrambling,
        )
        .to_bytes();
        let expected2_query =
            build_ca_pmt(&pmt2, CaPmtListManagement::Update, CaPmtCmdId::Query).to_bytes();
        let expected2_ok = build_ca_pmt(
            &pmt2,
            CaPmtListManagement::Update,
            CaPmtCmdId::OkDescrambling,
        )
        .to_bytes();
        let sends_before1_query = count_apdu_on_session(&d, CA_SESSION, &expected1_query);
        let sends_before1_ok = count_apdu_on_session(&d, CA_SESSION, &expected1_ok);
        let sends_before2_query = count_apdu_on_session(&d, CA_SESSION, &expected2_query);
        let sends_before2_ok = count_apdu_on_session(&d, CA_SESSION, &expected2_ok);

        // Advance the clock past the 10s re-query interval: this queues
        // BOTH services' `query`+`ok_descrambling` pairs (`requery_tick`
        // iterates the whole active set), but EN 50221's half-duplex link
        // (the #337 one-write-per-turn rule) only lets one out per turn —
        // feed enough `T_SB` acks to flush all four queued writes.
        clock.advance(Duration::from_secs(11));
        d.pump(Duration::from_secs(11)).unwrap();
        feed(&mut d, sb());
        feed(&mut d, sb());

        assert_eq!(
            count_apdu_on_session(&d, CA_SESSION, &expected1_query),
            sends_before1_query + 1,
            "expected service 1546's `query` resent exactly once on the shared tick"
        );
        assert_eq!(
            count_apdu_on_session(&d, CA_SESSION, &expected1_ok),
            sends_before1_ok + 1,
            "expected service 1546's `ok_descrambling` resent exactly once on the shared tick"
        );
        assert_eq!(
            count_apdu_on_session(&d, CA_SESSION, &expected2_query),
            sends_before2_query + 1,
            "expected service 1547's `query` resent exactly once on the shared tick"
        );
        assert_eq!(
            count_apdu_on_session(&d, CA_SESSION, &expected2_ok),
            sends_before2_ok + 1,
            "expected service 1547's `ok_descrambling` resent exactly once on the shared tick"
        );
    }

    // --- #765 (retained under #1032): re-query must reflect the CURRENT
    // active set, rebuilt fresh each tick, not a stale/frozen list ---

    #[test]
    fn requery_after_remove_resends_only_the_surviving_service() {
        use broadcast_common::Parse;

        let clock = TestClock::new();
        let mut d = driver_with_sessions().with_clock(clock.as_fn());
        // #1032: re-query is opt-in (REQUERY_DEFAULT = Duration::ZERO) —
        // enable it explicitly to exercise the timer.
        d.set_requery_interval(Duration::from_secs(10));
        d.take_notifications();

        let pmt1_bytes = build_ca_pmt_fixture(1546);
        let pmt1 = PmtSection::parse(&pmt1_bytes).unwrap();
        d.add_service(&pmt1).unwrap();
        d.device_mut().inbound.push_back(sb());
        clock.advance(Duration::from_millis(10));
        d.pump(Duration::from_millis(10)).unwrap();

        let pmt2_bytes = build_ca_pmt_fixture_distinct_pids(1547);
        let pmt2 = PmtSection::parse(&pmt2_bytes).unwrap();
        d.add_service(&pmt2).unwrap();
        d.device_mut().inbound.push_back(sb());
        clock.advance(Duration::from_millis(10));
        d.pump(Duration::from_millis(10)).unwrap();

        // Remove 1546: 1547 is now the SOLE survivor.
        d.remove_service(1546).unwrap();
        d.device_mut().inbound.push_back(sb());
        clock.advance(Duration::from_millis(10));
        d.pump(Duration::from_millis(10)).unwrap();
        d.take_notifications();

        // The #765 bite (still relevant under #1032's `update`-based
        // resend): the timer must rebuild its re-query set from the CURRENT
        // active set on every tick, not a list frozen at `add_service` time
        // — so 1546 (removed) must never be resent, and 1547 (surviving)
        // must be resent as the `query`+`ok_descrambling` pair.
        let expected_survivor_query =
            build_ca_pmt(&pmt2, CaPmtListManagement::Update, CaPmtCmdId::Query).to_bytes();
        let expected_survivor_ok = build_ca_pmt(
            &pmt2,
            CaPmtListManagement::Update,
            CaPmtCmdId::OkDescrambling,
        )
        .to_bytes();
        let expected_removed_query =
            build_ca_pmt(&pmt1, CaPmtListManagement::Update, CaPmtCmdId::Query).to_bytes();
        let expected_removed_ok = build_ca_pmt(
            &pmt1,
            CaPmtListManagement::Update,
            CaPmtCmdId::OkDescrambling,
        )
        .to_bytes();
        let sends_survivor_query_before =
            count_apdu_on_session(&d, CA_SESSION, &expected_survivor_query);
        let sends_survivor_ok_before = count_apdu_on_session(&d, CA_SESSION, &expected_survivor_ok);
        let sends_removed_query_before =
            count_apdu_on_session(&d, CA_SESSION, &expected_removed_query);
        let sends_removed_ok_before = count_apdu_on_session(&d, CA_SESSION, &expected_removed_ok);

        clock.advance(Duration::from_secs(11));
        d.pump(Duration::from_secs(11)).unwrap();
        feed(&mut d, sb());

        assert_eq!(
            count_apdu_on_session(&d, CA_SESSION, &expected_survivor_query),
            sends_survivor_query_before + 1,
            "the surviving service's `query` must be re-sent"
        );
        assert_eq!(
            count_apdu_on_session(&d, CA_SESSION, &expected_survivor_ok),
            sends_survivor_ok_before + 1,
            "the surviving service's `ok_descrambling` must be re-sent"
        );
        assert_eq!(
            count_apdu_on_session(&d, CA_SESSION, &expected_removed_query),
            sends_removed_query_before,
            "the removed service's `query` must NOT be re-sent"
        );
        assert_eq!(
            count_apdu_on_session(&d, CA_SESSION, &expected_removed_ok),
            sends_removed_ok_before,
            "the removed service's `ok_descrambling` must NOT be re-sent"
        );
    }

    // --- #763 Task 6: remove_service + clear managed state on CAM hot-plug ---

    #[test]
    fn remove_service_sends_update_not_selected_and_drops_from_managed_state() {
        use broadcast_common::Parse;

        let mut d = driver_with_sessions();
        d.take_notifications();

        // 1546 (`Only`, distinct PIDs 0x100/0x101) and 1547 (`Add`, distinct
        // PIDs 0x200/0x201) — distinct PID sets so removing 1546 is
        // observably different from removing 1547.
        let pmt1_bytes = build_ca_pmt_fixture(1546);
        let pmt1 = PmtSection::parse(&pmt1_bytes).unwrap();
        d.add_service(&pmt1).unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();

        let pmt2_bytes = build_ca_pmt_fixture_distinct_pids(1547);
        let pmt2 = PmtSection::parse(&pmt2_bytes).unwrap();
        d.add_service(&pmt2).unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();

        d.remove_service(1546).unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();

        // Oracle: the same PMT re-built directly via
        // dvb_ci::builder::build_ca_pmt with `Update`/`NotSelected` (EN 50221
        // §8.4.3.4 Table 25) — the exact bytes `remove_program` sends.
        let expected =
            build_ca_pmt(&pmt1, CaPmtListManagement::Update, CaPmtCmdId::NotSelected).to_bytes();
        assert_apdu_on_session(&d, CA_SESSION, &expected);

        assert_eq!(
            d.descramble_pids(),
            &[0x0200, 0x0201],
            "1546's ES PIDs must be gone; 1547's must remain"
        );
        assert!(
            d.managed_ca().services().get(&1546).is_none(),
            "1546 must no longer be tracked"
        );
        assert!(
            d.managed_ca().services().get(&1547).is_some(),
            "1547 must remain tracked"
        );
    }

    #[test]
    fn remove_service_of_untracked_program_is_a_no_op() {
        let mut d = driver_with_sessions();
        d.take_notifications();

        let ops_before = d.device().ops.len();
        d.remove_service(0xFFFF).unwrap();
        assert_eq!(
            d.device().ops.len(),
            ops_before,
            "removing an untracked program must not send anything to the device"
        );
        assert!(
            d.managed_ca().services().is_empty(),
            "removing an untracked program must not disturb the (empty) managed set"
        );
    }

    #[test]
    fn cam_removed_edge_clears_managed_state() {
        use broadcast_common::Parse;
        use dvb_ci::objects::ca_info::CaInfo;
        use dvb_si::tables::cat::CatSection;

        let mut d = driver_with_sessions();
        d.take_notifications();

        let pmt_bytes = build_ca_pmt_fixture(1546);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();
        d.add_service(&pmt).unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();

        // Populate emm_pids too, via ca_info + set_cat, so the test proves
        // the fix clears more than just `services`.
        feed(
            &mut d,
            r_apdu(
                CA_SESSION,
                &ser(&CaInfo {
                    ca_system_ids: vec![0x0648],
                }),
            ),
        );
        d.take_notifications();
        let mut descriptors = Vec::new();
        descriptors.extend_from_slice(&ca_descriptor(0x0648, 0x1FF0));
        let cat_bytes = build_cat_fixture(&descriptors);
        let cat = CatSection::parse(&cat_bytes).unwrap();
        d.set_cat(&cat).unwrap();

        assert!(
            !d.managed_ca().services().is_empty(),
            "precondition: a service is tracked"
        );
        assert!(
            !d.descramble_pids().is_empty(),
            "precondition: descramble_pids populated"
        );
        assert!(!d.emm_pids().is_empty(), "precondition: emm_pids populated");

        // Module physically removed: a CamRemoved hot-plug edge.
        d.device_mut().slot.module_present = false;
        d.pump(Duration::from_millis(10)).unwrap();
        let notes = d.take_notifications();
        assert!(
            notes.contains(&Notification::HotPlug(HotPlug::CamRemoved)),
            "expected CamRemoved, got {notes:?}"
        );

        assert!(
            d.managed_ca().services().is_empty(),
            "services must be cleared on CamRemoved"
        );
        assert!(
            d.descramble_pids().is_empty(),
            "descramble_pids must be cleared on CamRemoved"
        );
        assert!(
            d.emm_pids().is_empty(),
            "emm_pids must be cleared on CamRemoved"
        );
    }
}
