//! The sans-IO event/action model.
//!
//! The protocol core is pure: it consumes [`Event`]s and produces [`Action`]s,
//! with no device, threads, or clock of its own. The driver loop executes the
//! actions against a [`CaDevice`](crate::CaDevice) and feeds events back. This
//! keeps every state machine deterministic and testable without
//! hardware — a test (or a differential comparison against an external
//! reference) drives a sequence of events and asserts the emitted action
//! sequence.

use std::time::Duration;

use dvb_ci::objects::ca_pmt_reply::CaEnable;
use dvb_ci::resource::ResourceId;

/// An input to the protocol core.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event<'a> {
    /// One link-layer frame was read from the device.
    Readable(&'a [u8]),
    /// Logical time advanced by `elapsed` since the last tick (drives poll
    /// cadence and resource timers without a real clock in the core).
    Tick {
        /// Time since the previous tick.
        elapsed: Duration,
    },
    /// A request from the host application.
    Host(HostRequest<'a>),
}

/// A request the host application makes of the stack.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HostRequest<'a> {
    /// Bring the interface up: reset the slot and open the transport connection.
    Init,
    /// Send a serialized `ca_pmt` APDU body to the CAM's conditional-access
    /// resource (descrambling request).
    SendCaPmt(&'a [u8]),
    /// Answer an MMI `menu`/`list` by 1-based `choice_ref` (0 = "back"/cancel),
    /// sent as `menu_answ` to the module.
    MmiMenuAnswer(u8),
    /// Answer an MMI `enquiry` with the user's input text (EN 300 468 Annex A
    /// bytes), sent as `answ` (`answ_id = answer`).
    MmiEnquiryAnswer(&'a [u8]),
    /// Abort the current MMI enquiry (`answ` with `answ_id = cancel`).
    MmiCancel,
    /// Ask the module to open its MMI menu (`enter_menu` on the
    /// application_information session) — e.g. to read card / entitlement info.
    EnterMenu,
    /// Descramble the services in a PMT section (raw `dvb-si` PMT bytes). The
    /// stack filters the PMT's `CA_descriptor`s to the CAM's advertised CAIDs
    /// (from its `ca_info`) and sends a `ca_pmt` with `list_management = only`,
    /// `cmd_id = ok_descrambling`. The reply outcome surfaces as
    /// [`Notification::CaPmtReply`].
    Descramble(&'a [u8]),
    /// Descramble a **set** of programmes in one CA-PMT list (`first`/`more`/
    /// `last`, all `ok_descrambling`), replacing any prior set. Each element is a
    /// raw PMT section.
    DescramblePrograms(&'a [&'a [u8]]),
    /// Add one programme to the descrambled set (`list_management = add`).
    AddProgram(&'a [u8]),
    /// Remove one programme from the descrambled set (`list_management = update`,
    /// `cmd_id = not_selected`).
    RemoveProgram(&'a [u8]),
    /// Tear the interface down (close sessions + transport connection).
    Shutdown,
}

/// An output the driver loop must perform.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Action {
    /// Write one link-layer frame to the device.
    Write(Vec<u8>),
    /// Issue the `CA_RESET` ioctl.
    Reset,
    /// Issue the `CA_GET_SLOT_INFO` ioctl.
    QuerySlot,
    /// Arm the poll/timer to fire after `after` (coalesced: the latest wins).
    SetTimer {
        /// Delay before the next [`Event::Tick`] should be delivered.
        after: Duration,
    },
    /// Surface a host-facing [`Notification`].
    Notify(Notification),
}

/// A host-facing event surfaced by the stack (the useful outputs of a CI
/// session — what an application reacts to).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Notification {
    /// The module is present and the resource-manager handshake completed.
    CamReady,
    /// `application_information` was received.
    ApplicationInfo {
        /// `application_type` (0x01 = CA).
        application_type: u8,
        /// `application_manufacturer`.
        manufacturer: u16,
        /// `manufacturer_code`.
        code: u16,
        /// The decoded menu string.
        menu: String,
    },
    /// `ca_info` was received — the CA system ids the module supports.
    CaInfo {
        /// `CA_system_id` values the CAM can descramble.
        ca_system_ids: Vec<u16>,
    },
    /// A `ca_pmt_reply` was received for a prior CA_PMT.
    CaPmtReply {
        /// `program_number` the reply pertains to.
        program_number: u16,
        /// Programme-level `CA_enable` (EN 50221 §8.4.3.5 Table 26). `None`
        /// iff the programme `CA_enable_flag` bit was clear (no
        /// programme-level status given) — plumbed straight through from the
        /// `dvb_ci` `CaPmtReply` object's own `Option<CaEnable>`, never
        /// collapsed to a sentinel.
        ca_enable: Option<CaEnable>,
        /// Whether descrambling is (or was) possible, derived from
        /// `ca_enable`. Kept for back-compat with the pre-#763 boolean-only
        /// surface.
        descrambling_ok: bool,
    },
    /// A per-programme entitlement status transition (#763). Edge-triggered:
    /// fires once per `program_number` only when the programme-level
    /// `CA_enable` status (EN 50221 §8.4.3.5, Table 26) changes versus the
    /// last observed `ca_pmt_reply` for that programme. The transition is
    /// detected by the periodic re-query (`Driver::set_requery_interval`),
    /// which re-sends the active `ca_pmt`s with `ca_pmt_cmd_id = query` so
    /// the CAM re-evaluates and replies. Complements the coarse #726
    /// `HotPlug` module/card layer with the fine-grained per-service layer.
    /// (Programme-level status only; the ES-level `CA_enable` entries are
    /// not evaluated for this event.)
    Entitlement {
        /// `program_number` the status pertains to.
        program_number: u16,
        /// Programme-level `CA_enable` status.
        ca_enable: CaEnable,
        /// Whether descrambling is possible per the current status.
        descrambling_ok: bool,
    },
    /// An MMI menu/enquiry the host should display.
    Mmi(MmiEvent),
    /// A `host_control` request the CAM made of the host (EN 50221 §8.5.1). The
    /// host acts on it out-of-band (retune / PID replace); the runtime only
    /// surfaces the decoded request.
    HostControl(HostControlEvent),
    /// A session for `resource` was opened.
    SessionOpened {
        /// The resource the session serves.
        resource: ResourceId,
    },
    /// A session closed.
    SessionClosed {
        /// The `session_nb` that closed.
        session_nb: u16,
    },
    /// A protocol error surfaced by the stack (non-fatal; informational).
    Error {
        /// Human-readable detail.
        detail: String,
    },
    /// A CAM/card hot-plug transition (#726). See [`HotPlug`].
    HotPlug(HotPlug),
}

impl Notification {
    /// This notification's [`HotPlug`] transition, if it is one — a cheap
    /// classifier for poll-mode consumers that only care about hot-plug
    /// edges.
    #[must_use]
    pub fn hotplug(&self) -> Option<HotPlug> {
        if let Notification::HotPlug(h) = self {
            Some(*h)
        } else {
            None
        }
    }
}

/// A CAM/card hot-plug transition. `Cam*` are real DVB-CA slot-status edges;
/// `Card*` are best-effort EN 50221 app-layer inference (no card-detect line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HotPlug {
    /// The module transitioned absent → present *and* ready (DVB-CA slot
    /// status: `CA_CI_MODULE_PRESENT` and `CA_CI_MODULE_READY` both set —
    /// Linux uapi `linux/dvb/ca.h` `ca_slot_info.flags`). A real hardware
    /// signal from [`SlotInfo`](crate::device::SlotInfo), surfaced once on the
    /// edge (not on every poll); the driver re-drives the handshake (a fresh
    /// [`Init`](crate::event::HostRequest::Init)) so the newly-inserted module
    /// gets a clean resource-manager session.
    CamPresent,
    /// The module transitioned present → absent (DVB-CA slot status:
    /// `CA_CI_MODULE_PRESENT` clear). A real hardware signal; the driver tears
    /// down all session/handshake state so a later re-insert re-handshakes
    /// cleanly rather than reusing stale session numbers.
    CamRemoved,
    /// A smart card was inferred to have been inserted into the module.
    ///
    /// **Best-effort app-layer inference** — EN 50221 slots are module-level
    /// only; there is no card-detect line (verified against DD ddbridge /
    /// cxd2099 driver behaviour). Raised from one of: an `ca_info` CAID-set
    /// transition from empty to non-empty, or a `ca_pmt_reply`
    /// `descrambling_ok` transition from `false` to `true`. Some CAMs instead
    /// give a strong signal by resetting the module on card change, which
    /// surfaces as [`CamPresent`](HotPlug::CamPresent) rather than this
    /// variant.
    CardInserted,
    /// A smart card was inferred to have been removed from the module.
    ///
    /// **Best-effort app-layer inference** (see [`CardInserted`](HotPlug::CardInserted)
    /// for why no hardware signal exists). Raised from one of: an `ca_info`
    /// CAID-set transition from non-empty to empty, a `ca_pmt_reply`
    /// `descrambling_ok` transition from `true` to `false`, or MMI menu/list/
    /// enquiry text matching a "no card" style keyword.
    CardRemoved,
    /// The inserted smart card was inferred to have changed (swapped without
    /// an intervening removal the runtime observed).
    ///
    /// **Best-effort app-layer inference** (see [`CardInserted`](HotPlug::CardInserted)
    /// for why no hardware signal exists). Raised when a later `ca_info`
    /// reports a different non-empty CAID set than the last one seen for this
    /// module.
    CardChanged,
}

impl HotPlug {
    /// Stable lowercase spec-ish token for this transition (#204 label
    /// convention).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::CamPresent => "cam-present",
            Self::CamRemoved => "cam-removed",
            Self::CardInserted => "card-inserted",
            Self::CardRemoved => "card-removed",
            Self::CardChanged => "card-changed",
        }
    }
}

broadcast_common::impl_spec_display!(HotPlug);

/// A decoded high-level MMI `menu()` / `list()` ready for display (§8.6.5,
/// Tables 49/51). The three header lines and the choice list are kept separate
/// so a UI can render them directly — a title bar, two sub-lines, and a list of
/// selectable rows — without re-parsing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MmiMenu {
    /// Title line.
    pub title: String,
    /// Sub-title line (often a section heading).
    pub subtitle: String,
    /// Bottom line (often a hint such as "Select item and press OK").
    pub bottom: String,
    /// The selectable choices, in wire order. Answer the Nth (1-based) with
    /// [`Driver::mmi_menu_answer`](crate::Driver::mmi_menu_answer)`(N)`; `0`
    /// cancels / goes back.
    pub choices: Vec<String>,
}

/// MMI (man-machine interface) host events.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MmiEvent {
    /// A `menu()` to display — the user picks one choice. Answer via
    /// [`Driver::mmi_menu_answer`](crate::Driver::mmi_menu_answer).
    Menu(MmiMenu),
    /// A `list()` to display — informational (e.g. an entitlement listing); the
    /// host typically dismisses it with
    /// [`Driver::mmi_menu_answer`](crate::Driver::mmi_menu_answer)`(0)`.
    List(MmiMenu),
    /// An `enquiry` (text prompt) expecting an answer. Reply via
    /// [`Driver::mmi_enquiry_answer`](crate::Driver::mmi_enquiry_answer) or
    /// [`Driver::mmi_cancel`](crate::Driver::mmi_cancel).
    Enquiry {
        /// Prompt text.
        prompt: String,
        /// Whether the answer should be hidden (e.g. PIN).
        blind: bool,
        /// Expected answer length.
        answer_len: u8,
    },
    /// The module closed the MMI dialog.
    Close,
}

/// A `host_control` request the CAM makes of the host (ETSI EN 50221 §8.5.1,
/// Tables 27-30). The runtime surfaces the decoded request; the host performs
/// the retune / PID replacement itself (out of band) — there is no re-tune
/// logic in the stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HostControlEvent {
    /// `tune()` (Table 27): retune to the identified service.
    Tune {
        /// `network_id`.
        network_id: u16,
        /// `original_network_id`.
        original_network_id: u16,
        /// `transport_stream_id`.
        transport_stream_id: u16,
        /// `service_id`.
        service_id: u16,
    },
    /// `replace()` (Table 28): temporarily replace one component PID with
    /// another from the same multiplex.
    Replace {
        /// `replacement_ref` — matched later by a Clear Replace.
        replacement_ref: u8,
        /// 13-bit `replaced_PID`.
        replaced_pid: u16,
        /// 13-bit `replacement_PID`.
        replacement_pid: u16,
    },
    /// `clear_replace()` (Table 29): undo all Replace operations sharing this
    /// `replacement_ref`.
    ClearReplace {
        /// `replacement_ref` shared with one or more prior Replace requests.
        replacement_ref: u8,
    },
    /// `ask_release()` (Table 30): the CAM asks the host to release any
    /// replacements it holds (header-only request).
    AskRelease,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_entitlement_construction() {
        let note = Notification::Entitlement {
            program_number: 1234,
            ca_enable: CaEnable::Possible,
            descrambling_ok: true,
        };
        match note {
            Notification::Entitlement {
                program_number,
                ca_enable,
                descrambling_ok,
            } => {
                assert_eq!(program_number, 1234);
                assert_eq!(ca_enable, CaEnable::Possible);
                assert!(descrambling_ok);
            }
            _ => panic!("expected Entitlement variant"),
        }
    }
}
