//! DVB SimulCrypt — head-end CA message framing (ETSI TS 103 197 V1.5.1).
//!
//! The DVB SimulCrypt head-end carries control/response messages between its
//! conditional-access components over TCP. This crate is a **codec for those
//! messages** — it does not open sockets. It implements the generic message
//! structure and the two CA-bearing interfaces:
//!
//! - [`SimulcryptMessage`] — the generic `generic_message` (TS 103 197 §4.4.1,
//!   Table 1b): a 5-byte header (`protocol_version` + `message_type` +
//!   `message_length`, big-endian) followed by an ordered list of TLV
//!   [`Parameter`]s (`parameter_type` + `parameter_length` + value).
//!   `message_length` and every `parameter_length` are **recomputed on
//!   serialize** from the typed fields — there is no raw passthrough.
//! - **ECMG ⇔ SCS** (clause 5): [`EcmgScsMessageType`] (channel/stream setup,
//!   test, status, close, error, plus `CW_provision` `0x0201` and
//!   `ECM_response` `0x0202`) and the Table 5 [`EcmgScsParameterType`] registry
//!   (`Super_CAS_id` `0x0001` … `ECM_id` `0x0019`, `error_status` `0x7000`,
//!   `error_information` `0x7001`) + [`EcmgErrorStatus`] (Table 6).
//! - **EMMG/PDG ⇔ MUX** (clause 6): [`EmmgMuxMessageType`] (channel/stream
//!   messages, `stream_BW_request`/`allocation`, `data_provision` `0x0211`) and
//!   the Table 7 [`EmmgMuxParameterType`] registry + [`EmmgErrorStatus`]
//!   (Table 8), plus the [`DataType`] (§6.2.3) and [`SectionTspktFlag`] value
//!   tables.
//!
//! # Interface scoping
//!
//! The 16-bit `message_type`/`parameter_type` spaces are **interface-scoped**:
//! the same value means different things on different interfaces, and the
//! interface is not on the wire — it is fixed by which TCP connection the
//! message arrived on (analogous to a resource scope). So
//! [`SimulcryptMessage::parse_on`] takes an [`Interface`] hint and decodes the
//! raw values into the matching interface-tagged [`MessageType`] /
//! [`ParameterType`] enums.
//!
//! # Signalling only — no crypto
//!
//! The control words in `CP_CW_combination`/`CW_encryption`, the ECMs in
//! `ECM_datagram`, and the EMM/private data in `datagram` are carried as
//! **opaque borrowed bytes**. This crate frames and parses them; it never
//! decrypts or interprets them. The non-implemented interfaces (C(P)SIG⇔(P)SIG,
//! EIS⇔SCS, (P)SIG⇔MUX, ACG⇔EIS, SIMCOMP⇔MUXCONFIG) share the same framing but
//! are not modelled.
//!
//! `#![no_std]` + `alloc`; depends only on `dvb-common`.
//!
//! # Examples
//!
//! Build an ECMG⇔SCS `channel_setup` from typed fields and round-trip it:
//!
//! ```
//! use dvb_simulcrypt::{
//!     EcmgScsMessageType, EcmgScsParameterType, Interface, MessageType, Parameter,
//!     ParameterType, SimulcryptMessage,
//! };
//! use dvb_common::traits::{Parse, Serialize};
//!
//! let ecm_channel_id = [0x00, 0x2A]; // 0x002A
//! let super_cas_id = [0x00, 0x01, 0x00, 0x02]; // CA_system_id | subsystem_id
//! let msg = SimulcryptMessage::new(
//!     Interface::EcmgScs.protocol_version(),
//!     MessageType::EcmgScs(EcmgScsMessageType::ChannelSetup),
//!     vec![
//!         Parameter::new(
//!             ParameterType::EcmgScs(EcmgScsParameterType::EcmChannelId),
//!             &ecm_channel_id,
//!         ),
//!         Parameter::new(
//!             ParameterType::EcmgScs(EcmgScsParameterType::SuperCasId),
//!             &super_cas_id,
//!         ),
//!     ],
//! );
//!
//! let mut buf = vec![0u8; msg.serialized_len()];
//! msg.serialize_into(&mut buf).unwrap();
//! assert_eq!(SimulcryptMessage::parse_on(Interface::EcmgScs, &buf).unwrap(), msg);
//! ```
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p dvb-simulcrypt --example <name>`.\n"]
#![doc = "\n### `build_channel_setup`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_channel_setup.rs")]
#![doc = "```\n\n### `parse_cw_provision`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_cw_provision.rs")]
#![doc = "```"]

extern crate alloc;

mod error;
mod message;
mod registry;

pub use error::{Error, Result};
pub use message::{Parameter, SimulcryptMessage, HEADER_LEN, PARAMETER_HEADER_LEN};
pub use registry::{
    DataType, EcmgErrorStatus, EcmgScsMessageType, EcmgScsParameterType, EmmgErrorStatus,
    EmmgMuxMessageType, EmmgMuxParameterType, Interface, MessageType, ParameterType,
    SectionTspktFlag,
};
