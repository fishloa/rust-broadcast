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
        /// Raw `CA_enable`/descrambling-possibility bytes (per-program/ES).
        descrambling_ok: bool,
    },
    /// An MMI menu/enquiry the host should display.
    Mmi(MmiEvent),
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
}

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
