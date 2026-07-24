//! Pure-Rust EN 50221 DVB Common Interface **runtime** — the driver loop over
//! the [`dvb_ci`] codecs.
//!
//! [`dvb_ci`] is `no_std` and owns the *wire* layer (TPDU / SPDU / APDU
//! parse+serialize, CA_PMT building, CI Plus extensions). This crate adds the
//! *runtime*: device I/O, the TPDU poll loop, SPDU session management, and the
//! per-resource state machines that together drive a physical CAM, per
//! ETSI EN 50221 and TS 101 699.
//!
//! # Design
//!
//! The whole runtime is written against the [`CaDevice`] trait (the
//! hardware-abstraction boundary), so it runs against either:
//! - a real Linux CA device (`/dev/dvb/adapterN/caM`, the `linux` feature), or
//! - the in-memory [`MockCaDevice`], which makes the state machines testable
//!   without hardware and enables differential testing against an external
//!   reference (drive both with the same scripted mock CAM and assert the
//!   emitted `write`/ioctl byte sequences match).
//!
//! Implemented from the EN 50221 specification.
//!
//! # What's implemented
//!
//! - **Transport** (TPDU, §A.4): `Create_T_C` handshake, empty-`T_Data_Last`
//!   poll cadence, `T_SB` data-available → `T_RCV`, `T_Data_More/Last`
//!   reassembly, reply timeout.
//! - **Session** (SPDU, §7.2): session table; `open_session_request`/response,
//!   host-initiated `create_session`, `close_session`; `session_number` + APDU
//!   routing.
//! - **Resources** (§8): Resource Manager handshake (profile exchange →
//!   [`Notification::CamReady`], then opens module resources),
//!   application_information, conditional_access (`ca_pmt`/`ca_pmt_reply`),
//!   date_time (MJD + BCD), and mmi (surfaces module menus/enquiries as
//!   [`Notification::Mmi`]).
//! - **Descramble helper**: [`Driver::descramble`](crate::driver::Driver::descramble)
//!   / [`HostRequest::Descramble`] runs the full `ca_pmt` query → reply →
//!   ok_descrambling sequence, CAID-filtered to the CAM's `ca_info`.
//! - **Devices**: the in-memory [`MockCaDevice`] + [`MockCiDataDevice`], and the
//!   Linux `/dev/dvb/adapterN/caM` (control) + `ciM` (TS data-plane) devices
//!   behind the `linux` feature. The data plane ([`CiDataDevice`]) carries the
//!   scrambled-in / descrambled-out TS for separate-CI hardware.
//!
//! - **Diagnostics**: [`RecordingCaDevice`] captures the link in both directions;
//!   [`trace::decode_frame`]/[`decode_log`](crate::trace::decode_log) annotate a
//!   capture (TPDU → SPDU → APDU) for live-CAM debugging.
//!
//! # Managed CAS layer (#763)
//!
//! A single-slot CA orchestration layer sits on top of the raw `ca_pmt` /
//! `ca_pmt_reply` surface, fed **parsed `dvb-si` structs** (never raw bytes):
//!
//! - [`Driver::add_service`](crate::Driver::add_service) /
//!   [`remove_service`](crate::Driver::remove_service) — build + send the
//!   `ca_pmt` from a [`dvb_si::tables::pmt::PmtSection`] and track the slot's
//!   active service set (multi-programme list-management handled internally).
//! - [`Driver::set_cat`](crate::Driver::set_cat) — feed the CAT; the EMM-PID
//!   set becomes the CAT's EMM PIDs ∩ the CAM's advertised `ca_info` CAIDs.
//! - [`Driver::emm_pids`](crate::Driver::emm_pids) /
//!   [`descramble_pids`](crate::Driver::descramble_pids) /
//!   [`ca_pids`](crate::Driver::ca_pids) /
//!   [`required_pids`](crate::Driver::required_pids) — the PIDs to route into
//!   the `ci0` data plane (EMM ∪ ES ∪ ECM ∪ PCR = `required_pids`).
//! - [`Driver::set_requery_interval`](crate::Driver::set_requery_interval) — a
//!   periodic `ca_pmt` re-query (`cmd_id = query`) so a card entitled *after*
//!   the initial `ca_pmt` still refreshes; a per-programme status transition
//!   surfaces as an edge-triggered [`Notification::Entitlement`].
//! - [`CaDescrambler`] — a turnkey wrapper that additionally owns the `ci0`
//!   [`CiDataDevice`]: `feed_ts` filters a scrambled TS chunk to
//!   `required_pids()` and returns the descrambled TS. One `CaDescrambler` =
//!   one CI slot = one TS path (multi-tuner ⇒ one per slot).
//!
//! # `ci-probe`
//!
//! With the `linux` feature this crate also builds a **`ci-probe`** binary that
//! discovers and engages an installed CAM: `ci-probe list` / `info` /
//! `descramble <pmt>` / `mmi`, with `--trace` for an annotated link dump.
//!
//! Roadmap: the `host_control` resource and a differential test harness against
//! an external reference.
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

pub use dataplane::{CiDataDevice, MockCiDataDevice, TS_PACKET_LEN};
pub use descrambler::CaDescrambler;
pub use device::{CaDevice, DeviceOp, LinkEvent, MockCaDevice, RecordingCaDevice, SlotInfo};
pub use driver::Driver;
pub use event::{Action, Event, HostRequest, HotPlug, Notification};
#[cfg(all(feature = "linux", target_os = "linux"))]
pub use linux::{LinuxCaDevice, LinuxCiDataDevice};
pub use managed::{CaError, ManagedCa, ManagedService, REQUERY_DEFAULT};
pub use stack::CiStack;

/// Re-export of the wire-codec crate this runtime drives.
pub use dvb_ci;
