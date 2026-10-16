#![doc = include_str!("../README.md")]

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

mod error;
mod header;

pub use error::*;
pub use header::*;
