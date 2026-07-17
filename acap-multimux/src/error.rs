//! Error type for acap-multimux.
use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AcapError {
    /// A VDO access unit could not be converted to an IR sample.
    #[error("convert: {0}")]
    Convert(String),
    /// A transmux error while building codec config / samples.
    #[error("transmux: {0}")]
    Transmux(#[from] transmux::Error),
    /// A multimux error (pipeline/origin).
    #[error("multimux: {0}")]
    Multimux(#[from] multimux::MultimuxError),
    /// A VDO stream/buffer error (device builds only — `vdo` is an optional,
    /// `device`-feature-gated dependency; see `crate::vdo_source`).
    #[cfg(feature = "device")]
    #[error("vdo: {0}")]
    Vdo(#[from] vdo::Error),
}

pub type Result<T> = core::result::Result<T, AcapError>;
