//! ETSI EN 300 468 v1.19.1 DVB Service Information parser and builder.
//!
//! Entry points:
//! - [`Parse`](dvb_common::Parse) / [`Serialize`](dvb_common::Serialize) — the two
//!   symmetric contracts every table and descriptor implements.
//! - [`tables`] — PAT, PMT, CAT, TSDT, NIT, BAT, SDT, EIT, TDT, TOT, RST, DIT, SIT,
//!   ST, SAT, AIT, DSM-CC section, UNT, INT, RCT, CIT, RNT.
//! - [`descriptors`] — every DVB descriptor (tags 0x40..0x7F) plus MPEG-2 descriptors.
//! - [`pid::well_known`] — reserved DVB/MPEG-2 PIDs.
//! - [`table_id::TableId`] — typed table_id enum.
//! - [`descriptor_tag::DescriptorTag`] — typed descriptor_tag enum.
//!
//! See the crate README and `docs/` for the structured spec reference.

#![warn(missing_docs)]

pub mod descriptor_tag;
pub mod descriptors;
pub mod error;
pub mod pid;
pub mod section;
pub mod table_id;
pub mod tables;
pub mod text;
pub mod traits;

#[cfg(feature = "ts")]
pub mod ts;

pub use error::{Error, Result};
pub use table_id::TableId;
pub use descriptor_tag::DescriptorTag;
