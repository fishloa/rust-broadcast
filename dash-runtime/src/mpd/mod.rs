//! MPD model and parser (ISO/IEC 23009-1 clause 5.3, Annex B).

pub mod descriptors;
pub mod inherit;
pub mod model;
mod parse;
pub mod urls;
pub(crate) mod xml;

pub use model::*;
