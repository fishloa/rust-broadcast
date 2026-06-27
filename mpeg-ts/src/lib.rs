//! MPEG-2 Transport Stream framing — ITU-T H.222.0 §2.4 (= ISO/IEC 13818-1).
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;

pub mod error;
pub mod pid;
pub mod ts;
pub mod section;
pub mod resync;
pub mod mux;
pub mod packet_buf;

pub use error::{Error, Result};
