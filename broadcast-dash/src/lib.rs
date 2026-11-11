#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod descriptors;
pub mod duration;
pub mod error;
pub mod inherit;
pub mod limits;
pub mod model;
mod parse;
pub mod template;
pub mod utc_timing;
pub mod xs_datetime;

pub use error::{Limit, MpdError, Unsupported};
pub use model::*;
