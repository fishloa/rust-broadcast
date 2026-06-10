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
//!
//! # RFU policy
//!
//! BBFrame `reserved_future_use` bits are **emitted as 1** and
//! `reserved_zero_future_use` bits as **0**, following the DVB convention.
//! Parsers accept any value (no rejection on non-zero RFU) for forward
//! compatibility.

#![warn(missing_docs)]

pub mod crc;
pub mod error;
pub mod header;
pub mod issy;
pub mod packet;

pub use error::{Error, Result};
