//! acap-multimux — an Axis ACAP app that captures the camera's hardware-encoded
//! H.264/H.265 stream via VDO and serves LL-HLS on the camera, reusing the
//! `multimux` library. The pure `convert` module (VDO access unit -> IR sample)
//! is host-testable; the `vdo_source` module + the binary are `device`-gated
//! and build only inside the Axis ACAP Native SDK.

pub mod convert;
pub mod error;

#[cfg(feature = "device")]
pub mod vdo_source;

pub use error::{AcapError, Result};
