//! MPEG-1/2 Program Stream parsing — ISO/IEC 13818-1 (Rec. ITU-T H.222.0) §2.5.
//!
//! The Program Stream (`.mpg` / `.vob`) framing that wraps PES packets: the
//! `pack_header` (42-bit SCR + program_mux_rate), the optional `system_header`
//! (rate/audio/video bounds + per-stream P-STD buffer bounds), and the
//! `program_stream_map` (PSM). PES payloads are parsed via the `dvb-pes` crate.
//!
//! Depends only on `dvb-common` + `dvb-pes` and is `#![no_std]` (+ `alloc`).
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

extern crate alloc;

// Implemented by story #262 (delegated): pack_header / system_header /
// program_stream_map + the pack walker, with the symmetric Serialize.
