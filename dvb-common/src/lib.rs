//! Shared primitives for the dvb_si / dvb_t2mi / dvb_bbframe family.
//!
//! See individual modules for documentation.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod bcd;
pub mod crc32_mpeg2;
pub mod time;
pub mod traits;

pub use traits::{Parse, Serialize};
