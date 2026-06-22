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
//! # Status
//!
//! Foundation: the [`CaDevice`] abstraction + mock. The TPDU/SPDU/resource state
//! machines and the Linux device implementation land incrementally.

// The portable core is unsafe-free; only the optional Linux device leaf
// (`#[allow(unsafe_code)]` in `linux`) uses ioctls, so this is `deny`, not
// `forbid`.
#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod device;
pub mod driver;
pub mod event;
pub mod resource;
pub mod session;
pub mod stack;
pub mod transport;

#[cfg(all(feature = "linux", target_os = "linux"))]
pub mod linux;

pub use device::{CaDevice, DeviceOp, MockCaDevice, SlotInfo};
pub use driver::Driver;
pub use event::{Action, Event, HostRequest, Notification};
#[cfg(all(feature = "linux", target_os = "linux"))]
pub use linux::LinuxCaDevice;
pub use stack::CiStack;

/// Re-export of the wire-codec crate this runtime drives.
pub use dvb_ci;
