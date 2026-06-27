//! MPEG-2 Transport Stream framing — ITU-T H.222.0 §2.4 (= ISO/IEC 13818-1).
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;

pub mod error;
pub mod mux;
pub mod packet_buf;
pub mod pid;
pub mod resync;
pub mod section;
pub mod ts;

pub use error::{Error, Result};
