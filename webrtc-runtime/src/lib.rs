#![doc = include_str!("../README.md")]

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

extern crate alloc;

pub mod error;
mod http_util;
pub mod ice;
#[cfg(feature = "media")]
#[cfg_attr(docsrs, doc(cfg(feature = "media")))]
pub mod media;
pub mod whep;
pub mod whip;

pub use error::Error;
