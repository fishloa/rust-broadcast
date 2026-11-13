//! Segmenting a program into the parts and segments a [`Trunk`](crate::trunk::Trunk)
//! serves: the separate-rendition followers that cut where the leader cut.

pub mod audio;
pub mod program;
pub mod subtitle;

pub use program::{
    ProgramSegmenter, SegmenterConfig, SegmenterEvent, SiblingTrunks, TsHlsIdrFallback,
    TsHlsIdrStatus,
};
