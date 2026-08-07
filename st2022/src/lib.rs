//! SMPTE ST 2022-6:2012 / ST 2022-7:2019 — SDI-over-IP transport
//!
//! ST 2022-6 defines the **High Bit Rate Media Transport (HBRMT)** RTP payload
//! format for carrying uncompressed SDI signals (SD/HD/3G) over IP networks.
//! The entire serial digital interface payload — video, embedded audio, VANC,
//! HANC — is encapsulated as a single RTP stream.
//!
//! ST 2022-7 adds **seamless protection switching** across two redundant network
//! paths, allowing hitless failover with no visible glitch.
//!
//! # Wire structures
//!
//! - [`PayloadHeader`] — the 4/8/12+-byte HBRMT header preceding the media
//!   payload in each RTP datagram (§6.4).
//! - [`VideoSourceFormat`] — the MAP/FRAME/FRATE/SAMPLE fields describing the
//!   SDI signal structure.
//! - [`ClockFrequency`], [`FecUsage`], [`TimestampRef`], [`Scrambling`],
//!   [`VideoSourceId`] — typed field enums.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

extern crate alloc;

mod error;
mod header;

pub use error::*;
pub use header::*;
