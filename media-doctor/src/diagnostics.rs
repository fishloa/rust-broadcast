//! Built-in diagnostics.
//!
//! Each module implements one [`Diagnostic`](crate::Diagnostic).

pub mod cc_anomaly;
pub(crate) mod codec_common;
pub mod codec_signalling;
pub mod fps_cadence;
pub mod interlace;
pub mod param_sets;
pub mod pat_pmt_version;
pub mod pcr_check;
pub mod pts_check;
pub mod scte35_check;
pub mod sync_byte;
