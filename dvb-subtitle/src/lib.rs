//! DVB subtitling (bitmap) segment parser — ETSI EN 300 743 V1.6.1.
//!
//! Parses the subtitling segments carried in a DVB subtitle PES data field:
//! display-definition, page-composition, region-composition, CLUT-definition,
//! object-data (incl. 2/4/8-bit pixel-data sub-blocks), disparity-signalling,
//! alternative-CLUT and end-of-display-set segments. Feed it a reassembled PES
//! payload (e.g. from `dvb-pes`); it depends only on `dvb-common` and is
//! `#![no_std]` (+ `alloc`).
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

extern crate alloc;

// Implemented by story #257 (delegated): the segment modules + dispatch + tests.
