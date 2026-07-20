//! MPEG-2 Transport Stream framing — ITU-T H.222.0 / ISO/IEC 13818-1.
//!
//! Parses and serializes the MPEG-TS packet layer: 188-byte TS packets,
//! adaptation fields, PCR, PSI section reassembly and packetisation, byte-stream
//! resynchronisation, and a scheduler-mux for SI tables.
//!
//! # Primary types
//!
//! | Module | Type | Description |
//! |---|---|---|
//! | [`ts`] | [`ts::TsPacket`] | Zero-copy borrowed view of a 188-byte TS packet (ITU-T H.222.0 §2.4.3.2) |
//! | [`owned`] | [`owned::OwnedTsPacket`] | Owned 188-byte TS packet — for queuing, cloning, and in-place mutation |
//! | [`ts`] | [`ts::SectionReassembler`] | Per-PID PSI section assembly from TS payloads (§2.4.4) |
//! | [`mux`] | [`mux::SectionPacketiser`] | Packetise PSI sections back into TS packets with correct CC and padding |
//! | [`mux`] | [`mux::SiMux`] | Rate-scheduled SI table mux: upsert per-PID section sets, poll for TS packets |
//! | [`resync`] | [`resync::TsResync`] | Byte-stream resynchroniser — recovers 188/204-byte packet alignment from raw bytes |
//! | [`pid`] | [`pid::Pid`] | Typed 13-bit PID newtype with well-known constants |
//! | [`section`] | [`section::Section`] | Generic PSI/SI section header parser (table_id, section_length, version, CRC) |
//!
//! # `no_std` + embedded
//!
//! `mpeg-ts` is `#![no_std]` + `alloc`. It runs on bare-metal targets with a
//! heap (e.g. `thumbv7em-none-eabi`). Enable the `std` feature for
//! `std::error::Error` impls.
//!
//! # Feature flags
//!
//! | Feature | Default | Description |
//! |---|---|---|
//! | `std` | yes | `std::error::Error` impls; disable for embedded |
//! | `serde` | yes | `serde::Serialize` for packet/section types |
//!
//! # Spec
//!
//! - **ITU-T H.222.0** (= ISO/IEC 13818-1): §2.4.3.2 (TS packet), §2.4.3.3
//!   (adaptation field), §2.4.3.4 (PCR), §2.4.4 (PSI section).
//!   All wire layouts are cited by clause in the module-level doc comments.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;

pub mod error;
pub mod mux;
pub mod owned;
pub mod pid;
pub mod pusi;
pub mod resync;
pub mod section;
pub mod ts;

pub use error::{Error, Result};
pub use owned::OwnedTsPacket;
pub use ts::{
    AdaptationField, AdaptationFieldControl, AdaptationFieldExtension, Ltw, Pcr, ScramblingControl,
    SeamlessSplice, TsHeader, TsPacket,
};
