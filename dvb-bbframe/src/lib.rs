//! ETSI DVB-S2 / S2X / T2 BBFrame parser + builder.
//!
//! Supports both Normal Mode (NM) and High Efficiency Mode (HEM)
//! per EN 302 755 v1.4.1 §5.1.7.
//!
//! Entry points:
//! - [`header::Bbheader`] — the 10-byte BBHEADER with parse + serialize.
//! - [`packet::up_iter`] — user packet extraction from the data field.
//! - [`pump::BbframePump`] — per-PLP BBFrame→inner-TS pump (orchestrates header
//!   parse + carry-over extraction).
//! - [`crc::crc8`] — CRC-8 encoder (EN 302 307-1 §5.1.4 / EN 302 755 Annex F).
//! - [`issy`] — ISSY field parser (EN 302 755 Annex C).
//!
//! # Quick start
//! ```
//! use dvb_bbframe::header::{Bbheader, Matype, Mode, TsGs, BBHEADER_LEN};
//!
//! let hdr = Bbheader {
//!     matype: Matype { ts_gs: TsGs::Ts, sis: true, ccm: true, issyi: false, npd: false, ext: 0, isi: 0 },
//!     upl: 1504, sync: 0x47, dfl: 1504, syncd: 0, mode: Mode::Normal, issy_in_header: None,
//! };
//! let bytes = hdr.serialize();              // 10-byte BBHEADER
//! assert_eq!(bytes.len(), BBHEADER_LEN);
//! assert_eq!(Bbheader::parse(&bytes).unwrap(), hdr); // byte-identical round-trip
//! ```
//! For recovering the inner TS carried in a T2-MI stream, see
//! `dvb_t2mi::inner_ts::InnerTsRecovery`, which drives this header + the
//! [`packet`] extractor for you.
//!
//! # Generic Stream (GSE) handoff
//!
//! When `Bbheader::parse` yields `matype.ts_gs == TsGs::Gse`, the data field
//! carries GSE (Generic Stream Encapsulation) packets (EN 302 307-1 / EN 302 755).
//! GSE parsing is out of scope for this crate — hand the data field to the
//! third-party `dvb-gse` crate:
//!
//! ```ignore
//! use dvb_bbframe::header::{Bbheader, TsGs, BBHEADER_LEN};
//!
//! let hdr = Bbheader::parse(df_bytes).unwrap();
//! let data_field = &df_bytes[BBHEADER_LEN..];
//! match hdr.matype.ts_gs {
//!     TsGs::Ts => {
//!         /* TS user packets: dvb_bbframe::packet::up_iter(data_field, &hdr) */
//!     }
//!     TsGs::Gse => {
//!         /* GSE packets: hand `data_field` to the `dvb-gse` crate */
//!     }
//!     other => {
//!         /* GFPS / GCS — generic continuous or packetized */
//!     }
//! }
//! ```
//!
//! # RFU policy
//!
//! BBFrame `reserved_future_use` bits are **emitted as 1** and
//! `reserved_zero_future_use` bits as **0**, following the DVB convention.
//! Parsers accept any value (no rejection on non-zero RFU) for forward
//! compatibility.

#![warn(missing_docs)]

pub mod crc;
pub mod error;
pub mod header;
pub mod issy;
pub mod packet;
pub mod pump;

pub use error::{Error, Result};
