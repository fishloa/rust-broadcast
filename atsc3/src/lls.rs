//! LLS Binary Envelope — ATSC A/331:2025-06 §6.2, Table 6.1.
//!
//! Every `LLS_table()` instance is a 4-byte common header followed by the
//! table body. Per §6.2's closing bullet, each table body
//! (SLT/RRT/SystemTime/AEAT/OnscreenMessageNotification/UserDefined) is
//! individually gzip-compressed (RFC 1952) XML — `SignedMultiTable` itself is
//! not modeled by this crate yet (see [`crate::lls_table_id::LlsTableId`]
//! doc).
//!
//! The header parse/serialize is `no_std` — the compressed body is kept as an
//! opaque borrowed byte slice. Gzip decompression ([`LlsEnvelope::decompress`])
//! needs `std` (`flate2`) and is only available under the `std` feature.

use crate::error::{Error, Result};
use crate::lls_table_id::LlsTableId;
use broadcast_common::{Parse, Serialize};

/// Length of the common `LLS_table()` header (`LLS_table_id` +
/// `LLS_group_id` + `group_count_minus1` + `LLS_table_version`), in bytes.
const HEADER_LEN: usize = 4;

/// `LLS_table()` common envelope (A/331 §6.2 Table 6.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LlsEnvelope<'a> {
    /// `LLS_table_id` — identifies the table body's type.
    pub table_id: LlsTableId,
    /// `LLS_group_id` — associates this `LLS_table()` instance with a group
    /// of tables sharing the same ID; scope is the broadcast stream.
    pub group_id: u8,
    /// `group_count_minus1` — one less than the total number of distinct
    /// `LLS_group_id` values present in this PLP's ALP packet stream. See
    /// [`LlsEnvelope::group_count`] for the actual count.
    pub group_count_minus1: u8,
    /// `LLS_table_version` — increments by 1 (mod 256, wrapping `0xFF` ->
    /// `0x00`) whenever the identified table's data changes.
    pub table_version: u8,
    /// The table body: individually gzip-compressed (RFC 1952) XML.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub payload: &'a [u8],
}

impl LlsEnvelope<'_> {
    /// `group_count_minus1 + 1` — the actual number of distinct
    /// `LLS_group_id` values present.
    #[must_use]
    pub fn group_count(&self) -> u16 {
        u16::from(self.group_count_minus1) + 1
    }
}

impl<'a> Parse<'a> for LlsEnvelope<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "LlsEnvelope header",
            });
        }
        Ok(Self {
            table_id: LlsTableId::from_u8(bytes[0]),
            group_id: bytes[1],
            group_count_minus1: bytes[2],
            table_version: bytes[3],
            payload: &bytes[HEADER_LEN..],
        })
    }
}

impl Serialize for LlsEnvelope<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.payload.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = self.table_id.to_u8();
        buf[1] = self.group_id;
        buf[2] = self.group_count_minus1;
        buf[3] = self.table_version;
        buf[HEADER_LEN..len].copy_from_slice(self.payload);
        Ok(len)
    }
}

#[cfg(feature = "std")]
impl LlsEnvelope<'_> {
    /// Gzip-decompress (RFC 1952) `payload` into the XML bytes it carries.
    ///
    /// # Errors
    /// [`Error::Decompress`] if `payload` is not valid gzip data.
    pub fn decompress(&self) -> Result<alloc::vec::Vec<u8>> {
        use std::io::Read;

        let mut decoder = flate2::read::GzDecoder::new(self.payload);
        let mut out = alloc::vec::Vec::new();
        decoder
            .read_to_end(&mut out)
            .map_err(|e| Error::Decompress {
                what: "LlsEnvelope payload",
                reason: alloc::string::ToString::to_string(&e),
            })?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: [u8; 4] = [0x01, 0x02, 0x03, 0x04];

    /// A real gzip (RFC 1952) stream, decompressing to a spec-valid minimal
    /// SLT XML document (`<SLT bsid="1"><Service serviceId="1"
    /// serviceCategory="1"/></SLT>` — A/331 §6.3 Table 6.2 shape) —
    /// spec-valid data replacing the previous `b"payload-bytes"`, which was
    /// neither valid gzip nor valid XML (issue #926). Precomputed with
    /// Python's `gzip` module (`mtime=0`); a genuine real-broadcast capture
    /// fixture is separately committed under `fixtures/atsc3/` and covered
    /// by `tests/fixture_slt.rs` — these inline unit tests only need
    /// spec-shaped bytes, not a second copy of the real capture.
    const GZIPPED_SLT_XML: [u8; 67] = [
        0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0xff, 0xb3, 0x09, 0xf6, 0x09, 0x51,
        0x48, 0x2a, 0xce, 0x4c, 0xb1, 0x55, 0x32, 0x54, 0xb2, 0xb3, 0x09, 0x4e, 0x2d, 0x2a, 0xcb,
        0x4c, 0x4e, 0x55, 0x28, 0x86, 0xd0, 0x9e, 0x60, 0x61, 0x18, 0xcf, 0x39, 0xb1, 0x24, 0x35,
        0x3d, 0xbf, 0xa8, 0x12, 0x24, 0xa6, 0x6f, 0x67, 0xa3, 0x0f, 0xd4, 0x6a, 0x07, 0x00, 0xcc,
        0x27, 0xb5, 0x38, 0x40, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn parse_header_fields() {
        let mut bytes = HEADER.to_vec();
        bytes.extend_from_slice(&GZIPPED_SLT_XML);
        let env = LlsEnvelope::parse(&bytes).unwrap();
        assert_eq!(env.table_id, LlsTableId::Slt);
        assert_eq!(env.group_id, 0x02);
        assert_eq!(env.group_count_minus1, 0x03);
        assert_eq!(env.group_count(), 0x04);
        assert_eq!(env.table_version, 0x04);
        assert_eq!(env.payload, &GZIPPED_SLT_XML[..]);
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(matches!(
            LlsEnvelope::parse(&HEADER[..2]),
            Err(Error::BufferTooShort {
                need: 4,
                have: 2,
                ..
            })
        ));
    }

    #[test]
    fn round_trip() {
        let mut bytes = HEADER.to_vec();
        bytes.extend_from_slice(&GZIPPED_SLT_XML);
        let env = LlsEnvelope::parse(&bytes).unwrap();

        let mut out = alloc::vec![0u8; env.serialized_len()];
        let written = env.serialize_into(&mut out).unwrap();
        assert_eq!(written, bytes.len());
        assert_eq!(out, bytes);

        let reparsed = LlsEnvelope::parse(&out).unwrap();
        assert_eq!(reparsed, env);
    }

    #[test]
    fn serialize_rejects_short_output_buffer() {
        let env = LlsEnvelope {
            table_id: LlsTableId::Slt,
            group_id: 0,
            group_count_minus1: 0,
            table_version: 0,
            payload: b"xx",
        };
        let mut out = [0u8; 2];
        assert!(matches!(
            env.serialize_into(&mut out),
            Err(Error::OutputBufferTooSmall { need: 6, have: 2 })
        ));
    }

    #[cfg(feature = "std")]
    #[test]
    fn decompress_gzip_payload() {
        use std::io::Write;

        let xml = b"<SLT bsid=\"1\"><Service serviceId=\"1\" serviceCategory=\"1\"/></SLT>";
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(xml).unwrap();
        let gzipped = encoder.finish().unwrap();

        let mut bytes = HEADER.to_vec();
        bytes.extend_from_slice(&gzipped);

        let env = LlsEnvelope::parse(&bytes).unwrap();
        let decompressed = env.decompress().unwrap();
        assert_eq!(decompressed, xml);
    }

    #[cfg(feature = "std")]
    #[test]
    fn decompress_rejects_non_gzip_payload() {
        let mut bytes = HEADER.to_vec();
        bytes.extend_from_slice(b"not gzip data at all");
        let env = LlsEnvelope::parse(&bytes).unwrap();
        assert!(matches!(env.decompress(), Err(Error::Decompress { .. })));
    }
}
