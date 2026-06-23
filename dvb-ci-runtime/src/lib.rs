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
//! Roadmap: the `host_control` resource, MMI answering (`menu_answ`/`answ`), and
//! a differential test harness against an external reference.
//!
//! # Example
//!
//! Drive a CAM with the [`MockCaDevice`] — the same `Driver` API works against
//! the real Linux device:
//!
//! ```
//! use std::time::Duration;
//! use dvb_ci_runtime::{Driver, MockCaDevice, Notification};
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
//! // Host-facing events (CamReady, ApplicationInfo, CaInfo, Mmi, …) surface here.
//! for note in driver.take_notifications() {
//!     match note {
//!         Notification::CamReady => { /* now safe to send a ca_pmt */ }
//!         other => { let _ = other; }
//!     }
//! }
//! # Ok(())
//! # }
//! ```

// The portable core is unsafe-free; only the optional Linux device leaf
// (`#[allow(unsafe_code)]` in `linux`) uses ioctls, so this is `deny`, not
// `forbid`.
#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod dataplane;
pub mod device;
pub mod driver;
pub mod event;
pub mod resource;
pub mod session;
pub mod stack;
pub mod transport;

#[cfg(all(feature = "linux", target_os = "linux"))]
pub mod linux;

pub use dataplane::{CiDataDevice, MockCiDataDevice, TS_PACKET_LEN};
pub use device::{CaDevice, DeviceOp, MockCaDevice, SlotInfo};
pub use driver::Driver;
pub use event::{Action, Event, HostRequest, Notification};
#[cfg(all(feature = "linux", target_os = "linux"))]
pub use linux::{LinuxCaDevice, LinuxCiDataDevice};
pub use stack::CiStack;

/// Re-export of the wire-codec crate this runtime drives.
pub use dvb_ci;
