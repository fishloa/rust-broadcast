//! ETSI DVB-S2 / S2X / T2 BBFrame parser + builder.
//!
//! Supports both Normal Mode (NM) and High Efficiency Mode (HEM)
//! per EN 302 755 v1.4.1 §5.1.7.
//!
//! Entry points:
//! - [`header::Bbheader`] — the 10-byte BBHEADER with parse + serialize.
//! - [`packet::up_iter`] — user packet extraction from the data field.
//! - [`crc::crc8`] — CRC-8 encoder (EN 302 307-1 §5.1.4 / EN 302 755 Annex F).
//! - [`issy`] — ISSY field parser (EN 302 755 Annex C).

#![warn(missing_docs)]

pub mod crc;
pub mod error;
pub mod header;
pub mod issy;
pub mod packet;

pub use error::{Error, Result};
