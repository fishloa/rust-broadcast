#![doc = include_str!("../README.md")]
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

#[cfg(test)]
mod audit_k_tests;

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
