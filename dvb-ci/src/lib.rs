//! DVB Common Interface (EN 50221) — the host↔CICAM wire protocol, as typed Rust.
//!
//! Parses and builds the EN 50221 objects across the transport, session and
//! application layers, the DVB CI extensions (ETSI TS 101 699), the CI Plus
//! extension APDUs (ETSI TS 103 205), and a `CA_PMT` builder that turns a
//! `dvb-si` PMT into the object handed to a CICAM.
//!
//! ```text
//! dvb-si PmtSection --> builder::build_ca_pmt --> ca_pmt object (9F 80 32) --> CICAM
//! ```
//!
//! # What it does / what it does not
//!
//! **Does:** lossless bytes to typed objects and back for every TPDU, SPDU and
//! APDU; builds `ca_pmt` from a PMT and filters it to the CAM's `CA_system_id`s.
//! Sans-IO.
//!
//! **Does not:** talk to hardware (see `dvb-ci-runtime`); implement CI Plus
//! cryptography (Content Control authentication / key derivation / SAC
//! encryption — the APDU framing and SAC datatypes are typed, nothing is
//! decrypted); implement CI+ over USB (ETSI TS 103 605) or the PC-Card physical
//! layer; descramble anything.
//!
//! # Standards supported
//!
//! | Standard | Edition | Scope | Status |
//! |---|---|---|---|
//! | CENELEC EN 50221 — Common Interface Specification for CA and other DVB Decoder Applications | February 1997 | transport, session, Resource Manager, Application Information, CA Support (`ca_info`/`ca_pmt`/`ca_pmt_reply`), Host Control, Date-Time, MMI (low-level + high-level), Low-Speed Communications; all Table 58 tags | Full |
//! | ETSI TS 101 699 — Extensions to the Common Interface | V1.1.1 (1999-11) | Resource Manager v2, Application Information v2, Power Manager, Event Manager, Copy Protection, StreamInput, ServiceGateway, BroadcastServiceGateway, Status Query, Application MMI, Download, CA Pipeline | Full (printed APDU syntax) |
//! | ETSI TS 103 205 — Extensions to the CI Plus Specification | V1.4.1 (2019-05) | Multi-stream, Content Control APDUs + SAC datatypes, Multi-stream Host Control, Sample Decryption, CICAM Player, Auxiliary File System, extended CA Support, LSC v4, URI v3, Sample-Mode descriptors | Partial: `FileRequest`/`FileAcknowledge` bodies and `tune_broadcast_req`/`tune_reply`/`ask_release(_reply)` defer to CI Plus V1.3 and are not encoded; DRM payloads opaque; no cryptography |
//!
//! # Features
//!
//! | Feature | Default | Effect |
//! |---|---|---|
//! | `std` | yes | Links `std`; off gives `#![no_std]` + `alloc` |
//! | `serde` | no | `serde::Serialize` on the public types |
//!
//! # Quick start
//!
//! ```rust
//! use broadcast_common::Serialize;
//! use dvb_ci::AnyApdu;
//! use dvb_ci::length::LengthForm;
//! use dvb_ci::objects::ca_info::CaInfo;
//!
//! // A ca_info() APDU advertising two CA_system_ids.
//! let info = CaInfo {
//!     length_form: LengthForm::Minimal,
//!     ca_system_ids: vec![0x0500, 0x0B00],
//! };
//! let wire = info.try_to_bytes().unwrap(); // 9F 80 31 04 05 00 0B 00
//! assert_eq!(wire, [0x9F, 0x80, 0x31, 0x04, 0x05, 0x00, 0x0B, 0x00]);
//!
//! // Route any APDU by its 3-byte tag without knowing the type up front.
//! let any = AnyApdu::parse(&wire).unwrap();
//! assert_eq!(any.name(), "CA_INFO");
//! assert_eq!(any.try_to_bytes().unwrap(), wire); // round-trips byte-for-byte
//! ```
//!
//! # What is implemented
//!
//! - [`tag`] / [`length`] — the 3-byte [`ApduTag`] and the ASN.1-style
//!   `length_field`, whose coding form ([`length::LengthForm`]) is preserved.
//! - [`resource`] — the 4-octet [`ResourceId`].
//! - [`objects`] — EN 50221 application objects, dispatched by [`AnyApdu`]
//!   (40 typed `apdu_tag`s plus [`AnyApdu::Unknown`] for lossless fallthrough).
//! - [`spdu`] / [`tpdu`] — session- and transport-layer PDUs.
//! - [`builder`] — [`builder::build_ca_pmt`] and [`builder::build_ca_pmt_for_caids`].
//! - [`ci_ext`] — TS 101 699 resources, dispatched by [`ci_ext::CiExtApdu`] on the
//!   `resource_identifier()` (the extension resources reuse `0x9F80xx` tags).
//! - [`ci_plus`] — TS 103 205 resources, dispatched by [`ci_plus::CiPlusApdu`];
//!   extended CA Support, LSC v4, URI v3 and the Sample-Mode descriptors are
//!   standalone typed structs because TS 103 205 prints no resource_id for them.
//!
//! # Design notes
//!
//! - Every wire type implements [`broadcast_common::Parse`] /
//!   [`broadcast_common::Serialize`]; parse to serialize is byte-identical or the
//!   parse is an error. Reserved bits a peer sent are kept ([`reserved`]).
//! - Every 13-bit PID field is `mpeg_ts::pid::Pid`.
//! - `#![no_std]` + `alloc`; structured errors ([`Error`]); `#[non_exhaustive]`
//!   dispatch enums.
//! - Hardware verification belongs to `dvb-ci-runtime`; this crate was checked
//!   against real hardware only for the non-minimal `length_field` capture.
//!
//! # Related crates
//!
//! Depends on `broadcast-common`, `dvb-si` and `mpeg-ts`; `dvb-ci-runtime` builds
//! the driver runtime on top. MSRV 1.95.0; MIT OR Apache-2.0.
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
pub mod ci_plus;
mod dispatch;
pub mod error;
pub mod length;
pub mod objects;
pub mod reserved;
pub mod resource;
#[cfg(test)]
mod roundtrip_property;
pub mod spdu;
#[cfg(test)]
mod strict_tests;
pub mod tag;
pub mod tpdu;
pub mod traits;

pub use any::AnyApdu;
pub use error::{Error, Result};
pub use resource::ResourceId;
pub use tag::ApduTag;
pub use traits::ApduDef;
