//! SMPTE ST 2022-6:2012 — SDI-over-IP transport (HBRMT)
//!
//! ST 2022-6 defines the **High Bit Rate Media Transport (HBRMT)** RTP payload
//! format for carrying uncompressed SDI signals (SD/HD/3G) over IP networks.
//! The entire serial digital interface payload — video, embedded audio, VANC,
//! HANC — is encapsulated as a single RTP stream.
//!
//! ST 2022-7 **seamless protection switching** (hitless failover across two
//! redundant network paths) is **not** implemented by this crate — the
//! hitless merge is not implemented anywhere in this workspace yet.
//! `media-plane::byte_merge` is where it is expected to land — that module's
//! docs record `Hitless2022_7` as deliberately absent, not stubbed, and
//! `MergePolicy` has no such variant (issue #752). This crate only parses the
//! `VSID` field that a ST 2022-7 merge needs (§6.4), and does not depend on
//! `media-plane`.
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
