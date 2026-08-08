//! ATSC 3.0 (NextGen TV) Low-Level Signalling — A/331:2025-06 §6.2/§6.3
//!
//! Implements the two pieces of the ATSC 3.0 (NextGen TV) signalling stack
//! that are shipped today:
//!
//! - **LLS binary envelope** (A/331 §6.2) — the 4-byte common `LLS_table()`
//!   header plus its gzip-compressed table body.
//! - **Service List Table (SLT)** (A/331 §6.3) — XML parse of the
//!   rapid-channel-scan bootstrap table carried inside the envelope (a
//!   subset of fields — see [`crate::slt`] doc for exactly which).
//!
//! A/321 bootstrap discovery, A/331 ROUTE/DASH delivery, A/331 MMT
//! signalling, and Service Layer Signalling (SLS) are **not** implemented —
//! see the crate README's "Planned" section.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

extern crate alloc;

mod error;
pub mod lls;
pub mod lls_table_id;
pub mod slt;

pub use error::*;
pub use lls::LlsEnvelope;
pub use lls_table_id::LlsTableId;
pub use slt::{BroadcastSvcSignaling, ServiceCategory, SlsProtocol, Slt, SltService};
