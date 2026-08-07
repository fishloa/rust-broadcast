//! ATSC 3.0 signalling — A/321 bootstrap + A/331 ROUTE/DASH and MMT
//!
//! Implements the signalling layer of the ATSC 3.0 (NextGen TV) broadcast
//! system:
//!
//! - **A/321** — System Discovery and Signalling: bootstrap signalling that
//!   lets a receiver discover available services and their delivery parameters.
//! - **A/331** — Signalling, Delivery, Synchronization, and Error Protection:
//!   ROUTE/DASH delivery (LCT-based object carriage over ALC/FLUTE) and MMT
//!   signalling (MMTP-based delivery), plus the Service List Table (SLT) and
//!   Service Layer Signalling (SLS).

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

extern crate alloc;

mod error;

pub use error::*;
