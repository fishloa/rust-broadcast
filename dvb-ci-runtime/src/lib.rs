//! Pure-Rust EN 50221 DVB Common Interface **runtime** — the driver loop over
//! the [`dvb_ci`] codecs.
//!
//! [`dvb_ci`] is `no_std` and owns the *wire* layer (TPDU / SPDU / APDU
//! parse+serialize, CA_PMT building, CI Plus extension APDUs). This crate adds the
//! *runtime*: device I/O, the TPDU poll loop, SPDU session management, and the
//! per-resource state machines that together drive a physical CAM, per
//! ETSI EN 50221.
//!
//! # What it does / what it does not
//!
//! **Does:** drives a CI module end to end — slot reset, transport connection,
//! Resource Manager handshake, the six host-provided resources, `ca_pmt` for the
//! services you add, and (with a `ci0` data device) scrambled-in /
//! descrambled-out TS. Sync and sans-IO at the core: the caller owns the clock.
//!
//! **Does not:** run CI Plus (authentication, Content Control, multi-stream), serve
//! Low-Speed Communications or any TS 101 699 extension resource, re-tune on a
//! `host_control` request (it surfaces the request), or descramble itself.
//!
//! # Standards supported
//!
//! | Standard | Edition | Scope | Status |
//! |---|---|---|---|
//! | CENELEC EN 50221 | February 1997 | TPDU poll loop, session layer, host-provided Resource Manager / Application Information / Conditional Access / Date-Time / MMI / Host Control | Partial (no Low-Speed Communications; Host Control surfaced, not acted on) |
//! | ETSI TS 101 699 V1.1.1 | 1999-11 | none at runtime (typed in `dvb-ci`) | Not implemented |
//! | Linux DVB CA API (`dvb_ca_en50221`) | kernel ABI | `CA_RESET`, `CA_GET_SLOT_INFO`, `[slot, connection_id, ...]` frames, `ciM` data plane | Full for the calls used |
//!
//! # Features
//!
//! | Feature | Default | Effect |
//! |---|---|---|
//! | (none) | yes | Portable sync core and mocks; needs `std` |
//! | `linux` | no | `LinuxCaDevice`, `LinuxCiDataDevice`, `DeviceOpError`, the `ci-probe` binary |
//!
//! # Design
//!
//! The whole runtime is written against the [`CaDevice`] trait (the
//! hardware-abstraction boundary), so it runs against either:
//! - a real Linux CA device (`/dev/dvb/adapterN/caM`, the `linux` feature), or
//! - the in-memory [`MockCaDevice`], which makes the state machines testable
//!   without hardware.
//!
//! `#![deny(unsafe_code)]`: only the Linux device leaf uses `unsafe`. Long-form
//! `length_field`s follow `dvb-ci`'s preserved `LengthForm`; PID accessors are
//! typed `mpeg_ts::pid::Pid`.
//!
//! # What's implemented
//!
//! - **Transport** (TPDU, Annex A): `Create_T_C` handshake, empty-`T_Data_Last`
//!   poll cadence (half-duplex), `T_SB` data-available → `T_RCV`,
//!   `T_Data_More/Last` reassembly, reply timeout.
//! - **Session** (SPDU): session table; `open_session_request`/response,
//!   host-initiated `create_session`, `close_session`; `session_number` + APDU
//!   routing.
//! - **Resources** (all host-provided): Resource Manager handshake (profile
//!   exchange → [`Notification::CamReady`]), application_information,
//!   conditional_access (`ca_pmt`/`ca_pmt_reply`), date_time (MJD + BCD), mmi
//!   (menus/enquiries as [`Notification::Mmi`], answered via
//!   `Driver::mmi_menu_answer` / `mmi_enquiry_answer` / `mmi_cancel`), and
//!   [`resource::HostControl`] (`tune`/`replace`/`clear_replace`/`ask_release`
//!   surfaced as [`Notification::HostControl`]).
//! - **Descramble helper**: [`Driver::descramble`](crate::driver::Driver::descramble)
//!   / [`HostRequest::Descramble`] runs the `ca_pmt` query → reply →
//!   ok_descrambling sequence, CAID-filtered to the CAM's `ca_info`.
//! - **Devices**: the in-memory [`MockCaDevice`] + [`MockCiDataDevice`], and the
//!   Linux `caM` (control) + `ciM` (TS data-plane) devices behind the `linux`
//!   feature. The data plane ([`CiDataDevice`]) carries the scrambled-in /
//!   descrambled-out TS for separate-CI hardware.
//! - **Diagnostics**: [`RecordingCaDevice`] captures the link in both directions;
//!   [`trace::decode_frame`]/[`decode_log`](crate::trace::decode_log) annotate a
//!   capture (TPDU → SPDU → APDU).
//!
//! # Managed CAS layer
//!
//! A single-slot CA orchestration layer sits on top of the raw `ca_pmt` /
//! `ca_pmt_reply` surface, fed **parsed `dvb-si` structs** (never raw bytes):
//!
//! - [`Driver::add_service`](crate::Driver::add_service) /
//!   [`remove_service`](crate::Driver::remove_service) — build + send the
//!   `ca_pmt` from a [`dvb_si::tables::pmt::PmtSection`] and track the slot's
//!   active service set.
//! - [`Driver::set_cat`](crate::Driver::set_cat) — feed the CAT; the EMM-PID
//!   set becomes the CAT's EMM PIDs ∩ the CAM's advertised `ca_info` CAIDs.
//! - [`Driver::emm_pids`](crate::Driver::emm_pids) /
//!   [`descramble_pids`](crate::Driver::descramble_pids) /
//!   [`ca_pids`](crate::Driver::ca_pids) /
//!   [`required_pids`](crate::Driver::required_pids) — the PIDs to route into
//!   the `ci0` data plane (EMM ∪ ES ∪ ECM ∪ PCR = `required_pids`).
//! - [`Driver::set_requery_interval`](crate::Driver::set_requery_interval) —
//!   opt-in periodic `ca_pmt` re-query; a per-programme status transition
//!   surfaces as an edge-triggered [`Notification::Entitlement`].
//! - [`CaDescrambler`] — turnkey wrapper that also owns the `ci0`
//!   [`CiDataDevice`]: `feed_ts` filters a scrambled TS chunk to
//!   `required_pids()` and returns the descrambled TS. One `CaDescrambler` =
//!   one CI slot = one TS path.
//!
//! # Hardware verification
//!
//! Only what the CHANGELOG records: an AlphaCrypt / Irdeto module (live) for the
//! Resource Manager handshake, `application_information` and `ca_info` (18
//! CA_system_ids); a Digital Devices CXD2099 `ci0` for the split-fd data plane
//! (150 s, 111k `feed_ts` calls, 0 errors). The periodic re-query is not
//! verified on any CAM.
//!
//! # `ci-probe`
//!
//! With the `linux` feature this crate also builds a **`ci-probe`** binary that
//! discovers and engages an installed CAM: `ci-probe list` / `info` /
//! `descramble <pmt>` / `mmi`, with `--trace` for an annotated link dump.
//!
//! # Related crates
//!
//! Depends on [`dvb_ci`] (re-exported), `dvb-si`, `mpeg-ts` and
//! `broadcast-common`. MSRV 1.95.0; MIT OR Apache-2.0.
//!
//! # Example
//!
//! Drive a CAM with the [`MockCaDevice`] — the same `Driver` API works against
//! the real Linux device:
//!
//! ```
//! use std::time::Duration;
//! use dvb_ci_runtime::{Driver, HotPlug, MockCaDevice, Notification};
//! use dvb_ci_runtime::dvb_ci::tpdu::tags;
//!
//! # fn main() -> std::io::Result<()> {
//! // Script a module that accepts the transport connection.
//! let dev = MockCaDevice::new([vec![tags::C_T_C_REPLY, 0x01, 0x01]]);
//! let mut driver = Driver::new(dev);
//!
//! driver.init()?; // reset + open the transport connection
//!
//! // Pump the device: reads frames when readable, otherwise advances the
//! // EN 50221 poll cadence by the timeout.
//! for _ in 0..4 {
//!     driver.pump(Duration::from_millis(100))?;
//! }
//!
//! // Host-facing events (CamReady, ApplicationInfo, CaInfo, Mmi, HotPlug, …)
//! // surface here — either poll-drained:
//! for note in driver.take_notifications() {
//!     match note {
//!         Notification::CamReady => { /* now safe to send a ca_pmt */ }
//!         other => { let _ = other; }
//!     }
//! }
//!
//! // …or, since this crate is sync/sans-IO (no channels), delivered by a
//! // closure callback per pump — `pump_with` for every notification, or
//! // `pump_hotplug` to react only to CAM/card hot-plug transitions:
//! driver.pump_hotplug(Duration::from_millis(100), |hp| match hp {
//!     HotPlug::CamPresent => { /* CAM (re)inserted — re-send any pending ca_pmt */ }
//!     HotPlug::CamRemoved => { /* CAM removed — stop descrambling */ }
//!     _ => {}
//! })?;
//! # Ok(())
//! # }
//! ```

// The portable core is unsafe-free; only the optional Linux device leaf
// (`#[allow(unsafe_code)]` in `linux`) uses ioctls, so this is `deny`, not
// `forbid`.
#![deny(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod dataplane;
pub mod descrambler;
pub mod device;
pub mod driver;
pub mod event;
pub mod managed;
pub mod resource;
pub mod session;
pub mod stack;
pub mod trace;
pub mod transport;

#[cfg(all(feature = "linux", target_os = "linux"))]
pub mod linux;

pub use dataplane::{
    CiDataDevice, MockCiDataDevice, PartialWrite, TS_PACKET_LEN, partial_write_progress,
};
pub use descrambler::CaDescrambler;
pub use device::{
    CaDevice, DeviceOp, DeviceOpError, LinkEvent, MockCaDevice, RecordingCaDevice, SlotInfo,
};
pub use driver::Driver;
pub use event::{Action, Event, HostRequest, HotPlug, Notification};
#[cfg(all(feature = "linux", target_os = "linux"))]
pub use linux::{LinuxCaDevice, LinuxCiDataDevice};
pub use managed::{CaError, ManagedCa, ManagedService, REQUERY_DEFAULT};
pub use stack::CiStack;

/// Re-export of the wire-codec crate this runtime drives.
pub use dvb_ci;
