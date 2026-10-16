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

pub use error::{Error, Result};
pub use owned::OwnedTsPacket;
pub use ts::{
    AdaptationField, AdaptationFieldControl, AdaptationFieldExtension, Ltw, Pcr, ScramblingControl,
    SeamlessSplice, TsHeader, TsPacket,
};
