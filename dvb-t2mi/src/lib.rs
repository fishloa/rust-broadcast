//! ETSI TS 102 773 v1.4.1 DVB-T2 Modulator Interface (T2-MI) parser + builder.
//!
//! Entry points:
//! - [`traits::Parse`] / [`traits::Serialize`] — the two symmetric contracts
//!   every payload type implements.
//! - [`packet`] — T2-MI packet header and type parsing.
//! - [`payload`] — BBFrame, L1, FEF, timestamp, and addressing payload types.
//! - [`crc`] — CRC-32 per Annex A.
//!
//! # Example
//!
//! ```
//! use dvb_t2mi::packet::Header;
//! use dvb_common::Parse;
//! let buf = [0x00u8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
//! let hdr = Header::parse(&buf[..]).unwrap();
//! assert_eq!(hdr.payload_len_bits, 0);
//! ```

#![warn(missing_docs)]

pub mod crc;
pub mod error;
pub mod packet;
pub mod payload;

#[cfg(feature = "ts")]
pub mod ts;

pub use error::{Error, Result};
