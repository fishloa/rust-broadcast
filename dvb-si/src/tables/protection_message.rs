//! Protection message sections — ETSI TS 102 809 v1.3.1 §9 (table_id 0x7B).
//!
//! Protection messages are MPEG-2 private sections (ISO/IEC 13818-1) used by
//! the application-signalling authentication scheme. They share one table_id
//! (0x7B) and are discriminated by the 16-bit `table_id_extension`
//! ([`ProtectionMessageBody`], §9.3.4 Table 41, PDF p. 66):
//!
//! - `0x0000..=0x00FF` — **authentication message** (§9.4.3 Table 42, PDF
//!   pp. 70-71). The extension *is* the `authentication_group_id`. Carries a
//!   loop of section hashes plus a detached signature.
//! - `0x0100` — **certificate collection message** (§9.5.4.9 Table 51, PDF
//!   p. 91). The extension is the fixed `trust_message_id` 0x0100. Carries a
//!   count-prefixed list of DER-encoded DVBCertificates.
//! - `0x0101..=0xFFFF` — reserved for future use; preserved as a raw body so
//!   parse → serialize stays byte-exact ([`ProtectionMessageBody::Raw`]).
//!
//! Mirrors the SAT precedent (`sat.rs`): a typed common section header plus a
//! discriminated typed body. Variable-length inner loops (hash entries,
//! certificate slices) are exposed as borrowed slices.
//!
//! Per crate contract this parser does NOT verify CRC_32 (use
//! `Section::validate_crc`). Reserved bits are ignored on parse and emitted as
//! 1s on serialize, except spec-mandated zero fields which are emitted 0.

use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for all protection messages (TS 102 809 §9.3.4; coded per EN 300 468 §5.1.3).
pub const TABLE_ID: u8 = 0x7B;
/// Protection messages have no well-known PID — they are carried on the PID(s)
/// of the protectable elementary stream, signalled via the protection message
/// descriptor (§9.3.3). Mirrors the `dsmcc.rs` "no fixed PID" convention.
pub const PID: u16 = 0x0000;

/// `table_id_extension` value (inclusive) of the first authentication message (§9.3.4 Table 41).
pub const AUTH_EXTENSION_FIRST: u16 = 0x0000;
/// `table_id_extension` value (inclusive) of the last authentication message (§9.3.4 Table 41).
pub const AUTH_EXTENSION_LAST: u16 = 0x00FF;
/// `table_id_extension` (`trust_message_id`) of the certificate collection message (§9.3.4 Table 41).
pub const CERTIFICATE_COLLECTION_EXTENSION: u16 = 0x0100;

/// table_id(1) + section_length hi/lo(2) + extension(2) + version/cni(1)
/// + section_number(1) + last_section_number(1) = 8-byte common header.
const HEADER_LEN: usize = 8;
/// `section_length` counts from just after the field (byte 3) to end of section.
const SECTION_LENGTH_PREFIX: usize = 3;
/// CRC_32 trailer.
const CRC_LEN: usize = 4;

/// Authentication-message fixed body bytes after the common header, before the
/// hash loop: section_hash_algorithm_identifier(1) + section_hash_length(1)
/// + signature_algorithm_identifier(1) + reserved(4)|section_hashes_loop_length(12)(2).
const AUTH_FIXED_PREFIX: usize = 5;

/// One entry in the authentication message section-hash loop (§9.4.3 Table 42).
///
/// Each entry pairs a reference (locating the payload section the hash covers)
/// with the truncated hash itself. `reference` length is the 4-bit
/// `reference_length`; `hash` length is the section-wide `section_hash_length`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SectionHashEntry<'a> {
    /// 4-bit `reference_type` (§9.4.3 Table 45): 1 = same ES, 2 = component_tag ES.
    pub reference_type: u8,
    /// `reference_byte` field — its semantics depend on `reference_type`.
    pub reference: &'a [u8],
    /// The (possibly truncated) section hash, `section_hash_length` bytes.
    pub hash: &'a [u8],
}

/// Discriminated protection-message body, selected by `table_id_extension`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ProtectionMessageBody<'a> {
    /// Authentication message (extension `0x0000..=0x00FF`; §9.4.3 Table 42).
    AuthenticationMessage {
        /// `section_hash_algorithm_identifier` (§9.4.3 Table 43): 0 = SHA-256, 1 = SHA-512.
        section_hash_algorithm_identifier: u8,
        /// `section_hash_length` — bytes per hash in each loop entry.
        section_hash_length: u8,
        /// `signature_algorithm_identifier` (§9.4.3 Table 44).
        signature_algorithm_identifier: u8,
        /// Section-hash loop entries in wire order.
        hashes: Vec<SectionHashEntry<'a>>,
        /// `extension_byte` payload (length-prefixed by `extension_bytes_length`).
        extension_bytes: &'a [u8],
        /// `signature_key_identifier_byte` payload (length-prefixed).
        signature_key_identifier: &'a [u8],
        /// Detached signature — runs from after the key identifier to the CRC.
        signature: &'a [u8],
    },
    /// Certificate collection message (extension `0x0100`; §9.5.4.9 Table 51).
    CertificateCollection {
        /// DER-encoded DVBCertificate byte runs, one slice per `certificate_length` loop entry.
        certificates: Vec<&'a [u8]>,
    },
    /// Reserved extension (`0x0101..=0xFFFF`) — body preserved verbatim.
    Raw(&'a [u8]),
}

/// Protection message section (TS 102 809 §9; Tables 42 / 51).
///
/// Typed fields cover the common section header; [`ProtectionMessageSection::body`]
/// carries the typed, discriminated body.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProtectionMessageSection<'a> {
    /// `table_id_extension` — `authentication_group_id` for authentication
    /// messages, `trust_message_id` (0x0100) for certificate collections.
    pub table_id_extension: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit (spec mandates 1).
    pub current_next_indicator: bool,
    /// section_number.
    pub section_number: u8,
    /// last_section_number.
    pub last_section_number: u8,
    /// Discriminated body, selected by `table_id_extension`.
    pub body: ProtectionMessageBody<'a>,
}

impl<'a> Parse<'a> for ProtectionMessageSection<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = HEADER_LEN + CRC_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "ProtectionMessageSection",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "ProtectionMessageSection",
                expected: &[TABLE_ID],
            });
        }
        let section_length = (((bytes[1] & 0x0F) as usize) << 8) | bytes[2] as usize;
        let total = SECTION_LENGTH_PREFIX + section_length;
        if bytes.len() < total || total < HEADER_LEN + CRC_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: section_length,
                available: bytes.len().saturating_sub(SECTION_LENGTH_PREFIX),
            });
        }

        let table_id_extension = u16::from_be_bytes([bytes[3], bytes[4]]);
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = bytes[5] & 0x01 != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        let body_bytes = &bytes[HEADER_LEN..total - CRC_LEN];
        let body = match table_id_extension {
            AUTH_EXTENSION_FIRST..=AUTH_EXTENSION_LAST => parse_authentication_message(body_bytes)?,
            CERTIFICATE_COLLECTION_EXTENSION => parse_certificate_collection(body_bytes)?,
            _ => ProtectionMessageBody::Raw(body_bytes),
        };

        Ok(ProtectionMessageSection {
            table_id_extension,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            body,
        })
    }
}

/// Parse the authentication-message body (§9.4.3 Table 42, PDF pp. 70-71).
fn parse_authentication_message(body: &[u8]) -> Result<ProtectionMessageBody<'_>> {
    if body.len() < AUTH_FIXED_PREFIX {
        return Err(Error::BufferTooShort {
            need: AUTH_FIXED_PREFIX,
            have: body.len(),
            what: "ProtectionMessageSection::AuthenticationMessage",
        });
    }
    let section_hash_algorithm_identifier = body[0];
    let section_hash_length = body[1];
    let signature_algorithm_identifier = body[2];
    // bytes[3] high nibble = reserved; section_hashes_loop_length is 12 bits.
    let section_hashes_loop_length = (((body[3] & 0x0F) as usize) << 8) | body[4] as usize;

    let loop_start = AUTH_FIXED_PREFIX;
    let loop_end = loop_start + section_hashes_loop_length;
    if loop_end > body.len() {
        return Err(Error::SectionLengthOverflow {
            declared: section_hashes_loop_length,
            available: body.len() - loop_start,
        });
    }

    let hash_len = section_hash_length as usize;
    let mut hashes = Vec::new();
    let mut pos = loop_start;
    while pos < loop_end {
        // reference_type(4) | reference_length(4)
        let lead = body[pos];
        let reference_type = lead >> 4;
        let reference_length = (lead & 0x0F) as usize;
        let ref_start = pos + 1;
        let ref_end = ref_start + reference_length;
        let hash_end = ref_end + hash_len;
        if hash_end > loop_end {
            return Err(Error::SectionLengthOverflow {
                declared: reference_length + hash_len,
                available: loop_end - ref_start,
            });
        }
        hashes.push(SectionHashEntry {
            reference_type,
            reference: &body[ref_start..ref_end],
            hash: &body[ref_end..hash_end],
        });
        pos = hash_end;
    }

    // extension_bytes_length(8) + extension bytes (§9.4.3 Table 42 tail).
    if loop_end >= body.len() {
        return Err(Error::BufferTooShort {
            need: loop_end + 1,
            have: body.len(),
            what: "ProtectionMessageSection::extension_bytes_length",
        });
    }
    let extension_bytes_length = body[loop_end] as usize;
    let ext_start = loop_end + 1;
    let ext_end = ext_start + extension_bytes_length;
    if ext_end > body.len() {
        return Err(Error::SectionLengthOverflow {
            declared: extension_bytes_length,
            available: body.len() - ext_start,
        });
    }

    // signature_key_identifier_length(8) + key id bytes (Table 43 spillover, PDF p. 71).
    if ext_end >= body.len() {
        return Err(Error::BufferTooShort {
            need: ext_end + 1,
            have: body.len(),
            what: "ProtectionMessageSection::signature_key_identifier_length",
        });
    }
    let key_id_length = body[ext_end] as usize;
    let key_start = ext_end + 1;
    let key_end = key_start + key_id_length;
    if key_end > body.len() {
        return Err(Error::SectionLengthOverflow {
            declared: key_id_length,
            available: body.len() - key_start,
        });
    }

    // signature_byte loop runs to the end of the body (i.e. up to the CRC_32).
    let signature = &body[key_end..];

    Ok(ProtectionMessageBody::AuthenticationMessage {
        section_hash_algorithm_identifier,
        section_hash_length,
        signature_algorithm_identifier,
        hashes,
        extension_bytes: &body[ext_start..ext_end],
        signature_key_identifier: &body[key_start..key_end],
        signature,
    })
}

/// Parse the certificate-collection body (§9.5.4.9 Table 51, PDF p. 91).
fn parse_certificate_collection(body: &[u8]) -> Result<ProtectionMessageBody<'_>> {
    if body.is_empty() {
        return Err(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "ProtectionMessageSection::CertificateCollection",
        });
    }
    // byte 0: reserved(4) | certificate_count(4)
    let certificate_count = (body[0] & 0x0F) as usize;
    let mut certificates = Vec::with_capacity(certificate_count);
    let mut pos = 1;
    for _ in 0..certificate_count {
        if pos + 2 > body.len() {
            return Err(Error::BufferTooShort {
                need: pos + 2,
                have: body.len(),
                what: "ProtectionMessageSection::certificate_length",
            });
        }
        // reserved(4) | certificate_length(12)
        let certificate_length = (((body[pos] & 0x0F) as usize) << 8) | body[pos + 1] as usize;
        let cert_start = pos + 2;
        let cert_end = cert_start + certificate_length;
        if cert_end > body.len() {
            return Err(Error::SectionLengthOverflow {
                declared: certificate_length,
                available: body.len() - cert_start,
            });
        }
        certificates.push(&body[cert_start..cert_end]);
        pos = cert_end;
    }
    Ok(ProtectionMessageBody::CertificateCollection { certificates })
}

impl ProtectionMessageBody<'_> {
    /// Serialized length of the body (excluding common header and CRC).
    fn body_len(&self) -> usize {
        match self {
            ProtectionMessageBody::AuthenticationMessage {
                hashes,
                extension_bytes,
                signature_key_identifier,
                signature,
                ..
            } => {
                let loop_bytes: usize = hashes
                    .iter()
                    .map(|h| 1 + h.reference.len() + h.hash.len())
                    .sum();
                AUTH_FIXED_PREFIX
                    + loop_bytes
                    + 1
                    + extension_bytes.len()
                    + 1
                    + signature_key_identifier.len()
                    + signature.len()
            }
            ProtectionMessageBody::CertificateCollection { certificates } => {
                1 + certificates.iter().map(|c| 2 + c.len()).sum::<usize>()
            }
            ProtectionMessageBody::Raw(raw) => raw.len(),
        }
    }

    /// Write the body into `buf`, returning the number of bytes written.
    /// Over-range length/count fields error rather than silently truncating
    /// (the crate's strict serialize idiom — cf. `mpe.rs` scrambling guards).
    fn write_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            ProtectionMessageBody::AuthenticationMessage {
                section_hash_algorithm_identifier,
                section_hash_length,
                signature_algorithm_identifier,
                hashes,
                extension_bytes,
                signature_key_identifier,
                signature,
            } => {
                buf[0] = *section_hash_algorithm_identifier;
                buf[1] = *section_hash_length;
                buf[2] = *signature_algorithm_identifier;
                let loop_bytes: usize = hashes
                    .iter()
                    .map(|h| 1 + h.reference.len() + h.hash.len())
                    .sum();
                if loop_bytes > 0x0FFF {
                    return Err(Error::SectionLengthOverflow {
                        declared: loop_bytes,
                        available: 0x0FFF,
                    });
                }
                if extension_bytes.len() > u8::MAX as usize {
                    return Err(Error::SectionLengthOverflow {
                        declared: extension_bytes.len(),
                        available: u8::MAX as usize,
                    });
                }
                if signature_key_identifier.len() > u8::MAX as usize {
                    return Err(Error::SectionLengthOverflow {
                        declared: signature_key_identifier.len(),
                        available: u8::MAX as usize,
                    });
                }
                // reserved(4) emitted 1s | section_hashes_loop_length(12).
                buf[3] = 0xF0 | ((loop_bytes >> 8) as u8 & 0x0F);
                buf[4] = (loop_bytes & 0xFF) as u8;
                let mut pos = AUTH_FIXED_PREFIX;
                for h in hashes {
                    // reference_length is a 4-bit field.
                    if h.reference.len() > 0x0F {
                        return Err(Error::SectionLengthOverflow {
                            declared: h.reference.len(),
                            available: 0x0F,
                        });
                    }
                    buf[pos] = (h.reference_type << 4) | (h.reference.len() as u8 & 0x0F);
                    pos += 1;
                    buf[pos..pos + h.reference.len()].copy_from_slice(h.reference);
                    pos += h.reference.len();
                    buf[pos..pos + h.hash.len()].copy_from_slice(h.hash);
                    pos += h.hash.len();
                }
                buf[pos] = extension_bytes.len() as u8;
                pos += 1;
                buf[pos..pos + extension_bytes.len()].copy_from_slice(extension_bytes);
                pos += extension_bytes.len();
                buf[pos] = signature_key_identifier.len() as u8;
                pos += 1;
                buf[pos..pos + signature_key_identifier.len()]
                    .copy_from_slice(signature_key_identifier);
                pos += signature_key_identifier.len();
                buf[pos..pos + signature.len()].copy_from_slice(signature);
                pos += signature.len();
                Ok(pos)
            }
            ProtectionMessageBody::CertificateCollection { certificates } => {
                // certificate_count is a 4-bit field.
                if certificates.len() > 0x0F {
                    return Err(Error::SectionLengthOverflow {
                        declared: certificates.len(),
                        available: 0x0F,
                    });
                }
                // reserved(4) emitted 1s | certificate_count(4).
                buf[0] = 0xF0 | (certificates.len() as u8 & 0x0F);
                let mut pos = 1;
                for c in certificates {
                    // certificate_length is a 12-bit field.
                    if c.len() > 0x0FFF {
                        return Err(Error::SectionLengthOverflow {
                            declared: c.len(),
                            available: 0x0FFF,
                        });
                    }
                    // reserved(4) emitted 1s | certificate_length(12).
                    buf[pos] = 0xF0 | ((c.len() >> 8) as u8 & 0x0F);
                    buf[pos + 1] = (c.len() & 0xFF) as u8;
                    pos += 2;
                    buf[pos..pos + c.len()].copy_from_slice(c);
                    pos += c.len();
                }
                Ok(pos)
            }
            ProtectionMessageBody::Raw(raw) => {
                buf[..raw.len()].copy_from_slice(raw);
                Ok(raw.len())
            }
        }
    }
}

impl Serialize for ProtectionMessageSection<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.body.body_len() + CRC_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let section_length = (len - SECTION_LENGTH_PREFIX) as u16;
        buf[0] = TABLE_ID;
        // section_syntax_indicator=1, reserved_future_use=0, reserved=11, section_length hi nibble.
        buf[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3..5].copy_from_slice(&self.table_id_extension.to_be_bytes());
        // reserved(2)=11, version_number(5), current_next_indicator(1).
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;
        let body_written = self.body.write_into(&mut buf[HEADER_LEN..])?;
        let body_end = HEADER_LEN + body_written;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..body_end]);
        buf[body_end..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}

impl<'a> Table<'a> for ProtectionMessageSection<'a> {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for ProtectionMessageSection<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "PROTECTION_MESSAGE";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wrap a body in the 8-byte common header + placeholder CRC.
    fn build_section(extension: u16, version: u8, body: &[u8]) -> Vec<u8> {
        let section_length = (HEADER_LEN - SECTION_LENGTH_PREFIX + body.len() + CRC_LEN) as u16;
        let mut v = vec![
            TABLE_ID,
            0xB0 | ((section_length >> 8) as u8 & 0x0F),
            (section_length & 0xFF) as u8,
            (extension >> 8) as u8,
            (extension & 0xFF) as u8,
            0xC0 | (version << 1) | 0x01,
            0x00,
            0x00,
        ];
        v.extend_from_slice(body);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    /// Build an authentication-message body: one hash entry + ext + key id + signature.
    fn auth_body() -> Vec<u8> {
        let reference = [0x01]; // reference_type 1 => table_id of payload section
        let hash = [0xAA, 0xBB, 0xCC, 0xDD]; // section_hash_length = 4
        let mut hashes_loop = vec![(1u8 << 4) | (reference.len() as u8)]; // reference_type 1, length 1
        hashes_loop.extend_from_slice(&reference);
        hashes_loop.extend_from_slice(&hash);
        let loop_len = hashes_loop.len();

        let mut b = vec![
            0x00,                                  // section_hash_algorithm_identifier = SHA-256
            hash.len() as u8,                      // section_hash_length = 4
            0x01,                                  // signature_algorithm_identifier = ed25519
            0xF0 | ((loop_len >> 8) as u8 & 0x0F), // reserved | loop_length hi
            (loop_len & 0xFF) as u8,               // loop_length lo
        ];
        b.extend_from_slice(&hashes_loop);
        // extension_bytes_length + bytes
        b.push(2);
        b.extend_from_slice(&[0xDE, 0xAD]);
        // signature_key_identifier_length + bytes
        b.push(3);
        b.extend_from_slice(&[0x11, 0x22, 0x33]);
        // signature (runs to CRC)
        b.extend_from_slice(&[0x90, 0x91, 0x92, 0x93, 0x94, 0x95]);
        b
    }

    /// Build a certificate-collection body with two certificate slices.
    fn cert_body() -> Vec<u8> {
        let c0: &[u8] = &[0x30, 0x82, 0x01, 0x02];
        let c1: &[u8] = &[0xAB, 0xCD];
        let mut b = vec![0xF0 | 0x02]; // reserved | certificate_count = 2
        b.push(0xF0 | ((c0.len() >> 8) as u8 & 0x0F));
        b.push((c0.len() & 0xFF) as u8);
        b.extend_from_slice(c0);
        b.push(0xF0 | ((c1.len() >> 8) as u8 & 0x0F));
        b.push((c1.len() & 0xFF) as u8);
        b.extend_from_slice(c1);
        b
    }

    #[test]
    fn parse_authentication_message() {
        let bytes = build_section(0x0042, 5, &auth_body());
        let sec = ProtectionMessageSection::parse(&bytes).unwrap();
        assert_eq!(sec.table_id_extension, 0x0042);
        assert_eq!(sec.version_number, 5);
        assert!(sec.current_next_indicator);
        match sec.body {
            ProtectionMessageBody::AuthenticationMessage {
                section_hash_algorithm_identifier,
                section_hash_length,
                signature_algorithm_identifier,
                hashes,
                extension_bytes,
                signature_key_identifier,
                signature,
            } => {
                assert_eq!(section_hash_algorithm_identifier, 0x00);
                assert_eq!(section_hash_length, 4);
                assert_eq!(signature_algorithm_identifier, 0x01);
                assert_eq!(hashes.len(), 1);
                assert_eq!(hashes[0].reference_type, 1);
                assert_eq!(hashes[0].reference, &[0x01]);
                assert_eq!(hashes[0].hash, &[0xAA, 0xBB, 0xCC, 0xDD]);
                assert_eq!(extension_bytes, &[0xDE, 0xAD]);
                assert_eq!(signature_key_identifier, &[0x11, 0x22, 0x33]);
                assert_eq!(signature, &[0x90, 0x91, 0x92, 0x93, 0x94, 0x95]);
            }
            other => panic!("expected AuthenticationMessage, got {other:?}"),
        }
    }

    #[test]
    fn parse_certificate_collection() {
        let bytes = build_section(CERTIFICATE_COLLECTION_EXTENSION, 0, &cert_body());
        let sec = ProtectionMessageSection::parse(&bytes).unwrap();
        assert_eq!(sec.table_id_extension, 0x0100);
        match sec.body {
            ProtectionMessageBody::CertificateCollection { certificates } => {
                assert_eq!(certificates.len(), 2);
                assert_eq!(certificates[0], &[0x30, 0x82, 0x01, 0x02]);
                assert_eq!(certificates[1], &[0xAB, 0xCD]);
            }
            other => panic!("expected CertificateCollection, got {other:?}"),
        }
    }

    #[test]
    fn reserved_extension_kept_raw() {
        let raw = [0x01, 0x02, 0x03, 0x04];
        let bytes = build_section(0x0200, 0, &raw);
        let sec = ProtectionMessageSection::parse(&bytes).unwrap();
        assert!(matches!(sec.body, ProtectionMessageBody::Raw(b) if b == raw));
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let mut bytes = build_section(0x0000, 0, &auth_body());
        bytes[0] = 0x4D;
        assert!(matches!(
            ProtectionMessageSection::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x4D, .. }
        ));
    }

    #[test]
    fn rejects_short_buffer() {
        assert!(matches!(
            ProtectionMessageSection::parse(&[0x7B, 0xB0]).unwrap_err(),
            Error::BufferTooShort {
                what: "ProtectionMessageSection",
                ..
            }
        ));
    }

    #[test]
    fn auth_loop_overflow_rejected() {
        // Declare a section_hashes_loop_length that overruns the body.
        let mut body = vec![0x00, 0x04, 0x01, 0xF0, 0xFF]; // loop_len = 0xFF, body far shorter
        body.extend_from_slice(&[0x00]); // 1 stray byte
        let bytes = build_section(0x0000, 0, &body);
        assert!(matches!(
            ProtectionMessageSection::parse(&bytes).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn cert_length_overflow_rejected() {
        // certificate_count = 1 but certificate_length overruns the body.
        let body = vec![0xF0 | 0x01, 0x00, 0x10, 0x01]; // length 0x010 = 16, only 1 byte present
        let bytes = build_section(CERTIFICATE_COLLECTION_EXTENSION, 0, &body);
        assert!(matches!(
            ProtectionMessageSection::parse(&bytes).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn round_trip_authentication_message() {
        let bytes = build_section(0x0042, 7, &auth_body());
        let sec = ProtectionMessageSection::parse(&bytes).unwrap();
        let mut buf = vec![0u8; sec.serialized_len()];
        sec.serialize_into(&mut buf).unwrap();
        let re = ProtectionMessageSection::parse(&buf).unwrap();
        assert_eq!(sec, re);
    }

    #[test]
    fn round_trip_certificate_collection() {
        let bytes = build_section(CERTIFICATE_COLLECTION_EXTENSION, 3, &cert_body());
        let sec = ProtectionMessageSection::parse(&bytes).unwrap();
        let mut buf = vec![0u8; sec.serialized_len()];
        sec.serialize_into(&mut buf).unwrap();
        let re = ProtectionMessageSection::parse(&buf).unwrap();
        assert_eq!(sec, re);
    }

    #[test]
    fn round_trip_raw_reserved() {
        let bytes = build_section(0xABCD, 1, &[0xDE, 0xAD, 0xBE, 0xEF]);
        let sec = ProtectionMessageSection::parse(&bytes).unwrap();
        let mut buf = vec![0u8; sec.serialized_len()];
        sec.serialize_into(&mut buf).unwrap();
        let re = ProtectionMessageSection::parse(&buf).unwrap();
        assert_eq!(sec, re);
    }

    #[test]
    fn table_trait_constants() {
        assert_eq!(<ProtectionMessageSection as Table>::TABLE_ID, 0x7B);
        assert_eq!(<ProtectionMessageSection as Table>::PID, 0x0000);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_json_round_trip() {
        let bytes = build_section(0x0042, 5, &auth_body());
        let sec = ProtectionMessageSection::parse(&bytes).unwrap();
        let j = serde_json::to_string(&sec).unwrap();
        // The borrowed `&[u8]` fields (reference/hash/extension/signature)
        // cannot be JSON-deserialized zero-copy (serde_json renders them as
        // number sequences, not borrowed byte arrays) — the crate-wide
        // constraint that affects every borrowed-slice table (cf. mpe.rs).
        // Exercise the derive through the WIRE form: a re-parse must serialize
        // to byte-identical JSON, pinning the Serialize impl.
        let reparsed = ProtectionMessageSection::parse(&bytes).unwrap();
        assert_eq!(serde_json::to_string(&reparsed).unwrap(), j);
        assert!(j.contains("\"signature_algorithm_identifier\":1"));
    }
}
