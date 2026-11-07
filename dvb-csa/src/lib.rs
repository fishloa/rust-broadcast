#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

#[cfg(feature = "bitsliced")]
#[cfg_attr(docsrs, doc(cfg(feature = "bitsliced")))]
pub mod bitsliced;
mod block;
pub mod csa;
pub mod error;
pub mod key;
mod stream;
mod tables;
pub mod ts;
mod zeroize;

pub use csa::{descramble, scramble};
pub use error::Error;
pub use key::ControlWord;
