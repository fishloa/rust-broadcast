#![doc = include_str!("../README.md")]
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
extern crate alloc;

pub mod error;
pub mod mux;
pub mod owned;
pub mod pid;
pub mod pusi;
pub mod resync;
pub mod section;
pub mod ts;
pub mod ts_dup;

pub use error::{Error, Result};
pub use owned::OwnedTsPacket;
pub use ts::{
    AdaptationField, AdaptationFieldControl, AdaptationFieldExtension, ContinuityCounter, Ltw,
    MAX_DATAGRAM_PAYLOAD, Pcr, ScramblingControl, SeamlessSplice, TS_BYTES_PER_DATAGRAM,
    TS_PACKET_SIZE, TS_PACKETS_PER_DATAGRAM, TS_SYNC_BYTE, TsHeader, TsPacket,
};
