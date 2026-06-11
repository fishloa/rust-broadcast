//! Shared primitives for the dvb_si / dvb_t2mi / dvb_bbframe family.
//!
//! See individual modules for documentation: the [`Parse`] / [`Serialize`]
//! traits every wire type implements, the MPEG-2 [`crc32_mpeg2`] CRC, and the
//! [`bcd`] / [`time`] codecs.
//!
//! # Quick start
//! ```
//! use dvb_common::{bcd, crc32_mpeg2};
//!
//! // Binary-coded decimal (as used in MJD/BCD time fields):
//! assert_eq!(bcd::from_bcd_byte(0x42), Some(42));
//! assert_eq!(bcd::to_bcd_byte(42), Some(0x42));
//!
//! // MPEG-2 CRC-32 over a section body (deterministic):
//! let crc = crc32_mpeg2::compute(&[0xDE, 0xAD, 0xBE, 0xEF]);
//! assert_eq!(crc, crc32_mpeg2::compute(&[0xDE, 0xAD, 0xBE, 0xEF]));
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod bcd;
pub mod crc32_mpeg2;
pub mod time;
pub mod traits;

pub use traits::{Parse, Serialize};
