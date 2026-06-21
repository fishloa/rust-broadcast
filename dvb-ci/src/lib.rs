//! DVB Common Interface (EN 50221) — the host↔CICAM wire protocol.
//!
//! Parses + builds the EN 50221 protocol objects: resource APDUs (Resource
//! Manager, Application Information, CA support incl. `ca_pmt`/`ca_pmt_reply`,
//! Date-Time, Host Control, the full MMI set — low-level/display/keypad/subtitle/
//! download and high-level menu/list/enq — and Low-Speed Communications), the
//! session-layer SPDUs and transport-layer TPDUs, plus a `CA_PMT` builder that
//! turns a `dvb-si` PMT into the object handed to a CICAM.
//!
//! Every wire structure implements [`dvb_common::Parse`] / [`dvb_common::Serialize`]
//! symmetrically (parse → serialize is byte-identical), with all length fields
//! computed from content. Spec citations live in each module doc; the
//! render-verified transcription is in `docs/en_50221/`.
//!
//! Scope: the wire/protocol layer only. The physical PC-Card transport and CI+
//! crypto (the CC resource) are out of scope. `#![no_std]` (+ `alloc`).
//!
//! # Layers
//!
//! - [`tag`] / [`length`] — the 3-byte `apdu_tag` and the ASN.1-style
//!   `length_field` shared by all PDUs.
//! - [`resource`] — the 4-octet `resource_identifier()`.
//! - [`objects`] — application-layer APDU objects, dispatched by [`AnyApdu`].
//! - [`spdu`] — session-layer SPDUs (open/create/close session, session number).
//! - [`tpdu`] — transport-layer framing (C_TPDU/R_TPDU + connection mgmt).
//! - [`builder`] — the `CA_PMT` projection from a `dvb-si` PMT.
//!
//! All Table 58 `apdu_tag`s are now typed; an unallocated/private tag is produced
//! by [`AnyApdu::parse`] as [`AnyApdu::Unknown`] (lossless round-trip). The CI+
//! crypto (CC resource) and the PC-Card hardware transport remain out of scope.
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "Two runnable examples ship with this crate (`cargo run -p dvb-ci --example <name>`).\n"]
#![doc = "\n## `build_ca_pmt`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_ca_pmt.rs")]
#![doc = "```\n\n## `parse_apdu`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_apdu.rs")]
#![doc = "```"]

extern crate alloc;

pub mod any;
pub mod builder;
pub mod ci_ext;
pub mod error;
pub mod length;
pub mod objects;
pub mod resource;
pub mod spdu;
pub mod tag;
pub mod tpdu;
pub mod traits;

pub use any::AnyApdu;
pub use error::{Error, Result};
pub use resource::ResourceId;
pub use tag::ApduTag;
pub use traits::ApduDef;
