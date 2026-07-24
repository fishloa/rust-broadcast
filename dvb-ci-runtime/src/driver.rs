//! The driver — the one place I/O happens. It pumps a [`CaDevice`] against the
//! sans-IO [`CiStack`]: reads frames in, executes the stack's [`Action`]s
//! (writes/ioctls) out, tracks the requested poll timer, and collects
//! [`Notification`]s for the host application.

use std::collections::BTreeSet;
use std::io;
use std::time::Duration;

use broadcast_common::Serialize;
use dvb_ci::builder::build_ca_pmt;
use dvb_ci::objects::ca_pmt::{CaPmtCmdId, CaPmtListManagement};
use dvb_si::tables::cat::CatSection;
use dvb_si::tables::pmt::PmtSection;

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
    /// The slot's managed CAS-layer state (#763 Layer 1) — active services
    /// built via [`add_service`](Self::add_service).
    managed: ManagedCa,
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
            managed: ManagedCa::new(),
        }
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
        let built = build_ca_pmt(pmt, list_management, cmd_id);
        let built_bytes = built.to_bytes();
        // Also build the `query`-variant bytes (same list_management) for the
        // Task 5 re-query timer to resend — `ok_descrambling` solicits no
        // reply (EN 50221 §8.4.3.5), so only `query` is fit for that purpose.
        let requery_bytes = build_ca_pmt(pmt, list_management, CaPmtCmdId::Query).to_bytes();
        // `PmtSection` has no raw-bytes accessor — re-serialize (byte-identical
        // round-trip, a project invariant) to recover owned PMT bytes so
        // `remove_service` (#763 Task 6) can later re-drive `remove_program`,
        // which needs the raw section.
        let mut pmt_raw = vec![0u8; pmt.serialized_len()];
        let n = pmt
            .serialize_into(&mut pmt_raw)
            .expect("PmtSection::serialize_into on a freshly-sized buffer cannot fail");
        pmt_raw.truncate(n);
        self.send_ca_pmt(&built_bytes)?;
        self.managed.record(
            pmt.program_number,
            managed::service_of(pmt, cmd_id, built_bytes, requery_bytes, pmt_raw),
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
    /// `ca_pmt` (EN 50221 §8.4.3.4 Table 25, `cmd_id = query` — not the
    /// `ok_descrambling` variant originally sent to start descrambling; per
    /// §8.4.3.5, `ok_descrambling` solicits no reply) so the CAM re-evaluates
    /// and replies, surfacing as [`Notification::CaPmtReply`] and — on a
    /// status change — [`Notification::Entitlement`]. `Duration::ZERO`
    /// disables re-query. Defaults to [`managed::REQUERY_DEFAULT`] (10s) at
    /// construction.
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
        self.run(vec![Action::QuerySlot])?;
        if self.device.poll(timeout)? {
            let n = self.device.read(&mut self.buf)?;
            if n > 0 {
                let frame = self.buf[..n].to_vec();
                let actions = self.stack.handle(Event::Readable(&frame));
                self.run(actions)?;
                return Ok(true);
            }
        }
        let actions = self.stack.handle(Event::Tick { elapsed: timeout });
        self.run(actions)?;
        self.requery_tick(timeout)?;
        Ok(false)
    }

    /// Advance the #763 Task 5 entitlement re-query cadence by `elapsed`
    /// ([`ManagedCa::tick`](crate::managed::ManagedCa::tick), mirroring
    /// `resource.rs`'s `DateTime::tick` accumulate-then-fire pattern). When
    /// the interval elapses, re-send every actively-managed service's
    /// `query`-variant `ca_pmt` (`ManagedService::requery_ca_pmt`) via the
    /// same [`send_ca_pmt`](Self::send_ca_pmt) path
    /// [`add_service`](Self::add_service) uses (EN 50221 §8.4.3.4 Table 25) —
    /// `cmd_id = query`, not the `ok_descrambling` bytes originally sent, is
    /// required for a conformant CAM to re-evaluate and reply (§8.4.3.5:
    /// `ok_descrambling` solicits no reply).
    fn requery_tick(&mut self, elapsed: Duration) -> io::Result<()> {
        if !self.managed.tick(elapsed) {
            return Ok(());
        }
        let ca_pmts: Vec<Vec<u8>> = self
            .managed
            .services()
            .values()
            .map(|s| s.requery_ca_pmt.clone())
            .collect();
        for ca_pmt in ca_pmts {
            self.send_ca_pmt(&ca_pmt)?;
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
            Notification::Mmi(ev) => match Self::mmi_text(ev) {
                Some(text) => {
                    let lower = text.to_lowercase();
                    if MMI_CARD_ABSENT_KEYWORDS.iter().any(|k| lower.contains(k)) {
                        vec![Notification::HotPlug(HotPlug::CardRemoved)]
                    } else if MMI_CARD_PRESENT_KEYWORDS.iter().any(|k| lower.contains(k)) {
                        vec![Notification::HotPlug(HotPlug::CardInserted)]
                    } else {
                        Vec::new()
                    }
                }
                None => Vec::new(),
            },
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
            if let DeviceOp::Write(w) = op {
                if let Some(pos) = w.windows(3).position(|x| x == want) {
                    if let Ok(p) = Profile::parse(&w[pos..]) {
                        return p.resources.contains(&HOST_CONTROL);
                    }
                }
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
        let mut d = Driver::new(dev);
        d.init().unwrap();
        // first pump reads the C_T_C_Reply (activates the connection)
        assert!(d.pump(Duration::from_millis(100)).unwrap());
        // next pump has nothing to read → ticks → emits a poll write
        assert!(!d.pump(Duration::from_millis(100)).unwrap());
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

        let mut d = driver_with_sessions();
        d.take_notifications();

        let pmt_bytes = build_ca_pmt_fixture(1546);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();
        d.add_service(&pmt).unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();
        d.take_notifications();

        // The initial `add_service` send is `ok_descrambling` — assert it
        // happened, so the test proves the resend below (a distinct `query`
        // cmd_id) is a genuinely different wire message, not the same bytes.
        let expected_initial_ca_pmt =
            build_ca_pmt(&pmt, CaPmtListManagement::Only, CaPmtCmdId::OkDescrambling).to_bytes();
        assert_apdu_on_session(&d, CA_SESSION, &expected_initial_ca_pmt);

        // The re-query timer resends the `query`-variant bytes (EN 50221
        // §8.4.3.5: only `query`/`ok_mmi` solicit a `ca_pmt_reply` from a
        // conformant CAM — `ok_descrambling` does not).
        let expected_ca_pmt =
            build_ca_pmt(&pmt, CaPmtListManagement::Only, CaPmtCmdId::Query).to_bytes();
        let sends_before_requery = count_apdu_on_session(&d, CA_SESSION, &expected_ca_pmt);

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

        // Advance the clock past the default 10s re-query interval: a single
        // pump ticks the stack with elapsed = 11s (nothing readable this
        // turn), which the #763 Task 5 re-query timer picks up and queues
        // the tracked service's exact ca_pmt for resend (EN 50221 §8.4.3.4
        // Table 25). EN 50221's link is half-duplex — this tick's own
        // keep-alive poll already claimed the turn, so the resend is
        // written on the module's next `T_SB` (the #337 one-write-per-turn
        // rule), same as any other queued host write in this test suite.
        d.pump(Duration::from_secs(11)).unwrap();
        all_notes.extend(d.take_notifications());
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();

        let sends_after_requery = count_apdu_on_session(&d, CA_SESSION, &expected_ca_pmt);
        assert_eq!(
            sends_after_requery,
            sends_before_requery + 1,
            "expected the re-query timer to resend the exact ca_pmt exactly once"
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
    fn set_requery_interval_zero_disables_resend() {
        use broadcast_common::Parse;

        let mut d = driver_with_sessions();
        d.set_requery_interval(Duration::ZERO);
        d.take_notifications();

        let pmt_bytes = build_ca_pmt_fixture(1549);
        let pmt = PmtSection::parse(&pmt_bytes).unwrap();
        d.add_service(&pmt).unwrap();
        d.device_mut().inbound.push_back(sb());
        d.pump(Duration::from_millis(10)).unwrap();

        // Count the `query`-variant bytes — the ones the timer would resend
        // if it fired — not the `ok_descrambling` bytes `add_service` sent.
        let expected_ca_pmt =
            build_ca_pmt(&pmt, CaPmtListManagement::Only, CaPmtCmdId::Query).to_bytes();
        let sends_before = count_apdu_on_session(&d, CA_SESSION, &expected_ca_pmt);

        // Even a very long tick must not trigger a re-query once disabled.
        d.pump(Duration::from_secs(1000)).unwrap();

        let sends_after = count_apdu_on_session(&d, CA_SESSION, &expected_ca_pmt);
        assert_eq!(
            sends_after, sends_before,
            "Duration::ZERO must disable the re-query resend"
        );
    }

    #[test]
    fn requery_timer_resends_every_active_service_not_just_one() {
        use broadcast_common::Parse;

        let mut d = driver_with_sessions();
        d.take_notifications();

        // Two services on the managed set: 1546 (`Only`, first-ever) and
        // 1547 (`Add`, joining the active set).
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
        d.take_notifications();

        // The `query`-variant bytes the re-query timer resends for each
        // service — same `list_management` each got at `add_service` time.
        let expected1 =
            build_ca_pmt(&pmt1, CaPmtListManagement::Only, CaPmtCmdId::Query).to_bytes();
        let expected2 = build_ca_pmt(&pmt2, CaPmtListManagement::Add, CaPmtCmdId::Query).to_bytes();
        let sends_before1 = count_apdu_on_session(&d, CA_SESSION, &expected1);
        let sends_before2 = count_apdu_on_session(&d, CA_SESSION, &expected2);

        // Advance the clock past the default 10s re-query interval: this
        // queues BOTH services' resends (`requery_tick` iterates the whole
        // active set), but EN 50221's half-duplex link (the #337
        // one-write-per-turn rule) only lets one out per turn — feed enough
        // `T_SB` acks to flush both queued writes, mirroring `feed`'s own
        // multi-turn drain loop.
        d.pump(Duration::from_secs(11)).unwrap();
        feed(&mut d, sb());

        let sends_after1 = count_apdu_on_session(&d, CA_SESSION, &expected1);
        let sends_after2 = count_apdu_on_session(&d, CA_SESSION, &expected2);
        assert_eq!(
            sends_after1,
            sends_before1 + 1,
            "expected service 1546's query ca_pmt resent exactly once on the shared tick"
        );
        assert_eq!(
            sends_after2,
            sends_before2 + 1,
            "expected service 1547's query ca_pmt resent exactly once on the shared tick"
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
