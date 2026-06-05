//! MPE datagram_section — ETSI EN 301 192 v1.7.1 §7.1 (PDF pp. 17-19).
//!
//! The Multiprotocol Encapsulation (MPE) `datagram_section` carries an IP
//! datagram (optionally LLC/SNAP-encapsulated) over a DVB transport stream,
//! tagged with the destination MAC address. Its `table_id` is `0x3E` — the
//! DSM-CC "sections with private data" value (ISO/IEC 13818-6). This is the
//! *typed* view of exactly what [`crate::tables::dsmcc::DsmccSection`] carries
//! raw: a `0x3E` section reaching `dsmcc.rs` is the same wire bytes this module
//! decodes into MAC address + scrambling control + payload fields.
//!
//! Like DSM-CC, MPE has no well-known PID — the elementary PID is signalled by
//! the PMT (via a `data_broadcast_descriptor`, EN 301 192 §7.2.1) — so [`PID`]
//! is `0x0000`, following the `dsmcc.rs` precedent.
//!
//! Per the crate contract this parser does NOT verify the trailing CRC/checksum
//! integrity; [`dvb_common`]'s section machinery owns CRC validation. Reserved
//! bits are ignored on parse and emitted as `1`s on serialize.
//!
//! ## Trailer (SSI-dependent)
//!
//! EN 301 192 Table 3 makes the 4-byte trailer conditional on
//! `section_syntax_indicator` (SSI):
//! - SSI == 1 → `CRC_32` (computed over the whole section).
//! - SSI == 0 → `checksum` per ISO/IEC 13818-6.
//!
//! The ISO/IEC 13818-6 private-section *checksum* algorithm is not
//! implementable without that spec, so for `SSI == 0` we preserve the four
//! parsed trailer bytes verbatim in [`MpeDatagramSection::checksum`] and
//! re-emit them byte-for-byte. For `SSI == 1` the trailer is recomputed as
//! CRC_32 on serialize and `checksum` is ignored.

use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for an MPE `datagram_section` — the DSM-CC private-data value
/// (ISO/IEC 13818-6); see [`crate::tables::dsmcc`] for the raw view.
pub const TABLE_ID: u8 = 0x3E;

/// MPE has no well-known PID — the elementary PID comes from the PMT.
pub const PID: u16 = 0x0000;

/// Bytes 0-2: table_id(1) + SSI/private/reserved/section_length(2).
const HEADER_LEN: usize = 3;

/// Bytes 3-11: MAC_6(1) + MAC_5(1) + flags(1) + section_number(1)
/// + last_section_number(1) + MAC_4(1) + MAC_3(1) + MAC_2(1) + MAC_1(1).
const EXTENSION_LEN: usize = 9;

/// Bytes occupied by the trailing CRC_32 / checksum field.
const CRC_LEN: usize = 4;

/// Minimum total encoded length: header + extension + trailer (empty payload).
const MIN_SECTION_LEN: usize = HEADER_LEN + EXTENSION_LEN + CRC_LEN;

/// MPE `datagram_section` (ETSI EN 301 192 §7.1).
///
/// The 48-bit destination MAC is scattered across the section by the wire
/// format (Figure 1, PDF p. 18): `MAC_address_1` (the most-significant byte)
/// lands last, `MAC_address_6` (the least-significant byte) lands first:
///
/// ```text
/// section byte:   3        4        8        9        10       11
/// MAC field:      MAC_6    MAC_5    MAC_4    MAC_3    MAC_2    MAC_1
/// MAC byte:       LSB ...                                 ... MSB
/// ```
///
/// We reassemble it into [`MpeDatagramSection::mac_address`] in network order
/// (`MAC_1..MAC_6`, most-significant first), so `mac_address[0]` is `MAC_1`
/// and `mac_address[5]` is `MAC_6`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MpeDatagramSection<'a> {
    /// `section_syntax_indicator` bit. When `true` the trailer is a computed
    /// `CRC_32`; when `false` it is an ISO/IEC 13818-6 checksum preserved
    /// verbatim in [`Self::checksum`].
    pub section_syntax_indicator: bool,

    /// `private_indicator` bit (byte 1, bit 6).
    pub private_indicator: bool,

    /// Destination MAC address in network order, `MAC_1` (MSB) first through
    /// `MAC_6` (LSB) last. See the struct docs for the wire scatter.
    pub mac_address: [u8; 6],

    /// 2-bit `payload_scrambling_control` (EN 301 192 Table 4). `0` =
    /// unscrambled; `1`/`2`/`3` = service-defined.
    pub payload_scrambling_control: u8,

    /// 2-bit `address_scrambling_control` (EN 301 192 Table 5). `0` =
    /// unscrambled; `1`/`2`/`3` = service-defined.
    pub address_scrambling_control: u8,

    /// `LLC_SNAP_flag`. When `true`, [`Self::payload`] is an LLC/SNAP-
    /// encapsulated datagram; when `false`, a bare IP datagram. We keep the
    /// payload raw either way (LLC/SNAP and IP framing are out of scope).
    pub llc_snap_flag: bool,

    /// `current_next_indicator` bit (the spec mandates `1`).
    pub current_next_indicator: bool,

    /// Section index within the fragmented datagram.
    pub section_number: u8,

    /// Final section index of the fragmented datagram.
    pub last_section_number: u8,

    /// Raw payload: LLC/SNAP bytes when [`Self::llc_snap_flag`] is set, else
    /// IP datagram bytes — plus any trailing `stuffing_byte`s — kept as one
    /// borrowed slice running from byte 12 to the 4-byte trailer. We do not
    /// parse LLC/SNAP or IP, nor split out stuffing (EN 301 192 §7.1).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub payload: &'a [u8],

    /// Verbatim trailer bytes when `section_syntax_indicator == false` (an
    /// ISO/IEC 13818-6 checksum we cannot recompute). Ignored when SSI is
    /// `true`, where the trailer is a computed `CRC_32`.
    pub checksum: [u8; 4],
}

impl<'a> Parse<'a> for MpeDatagramSection<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MIN_SECTION_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_SECTION_LEN,
                have: bytes.len(),
                what: "MpeDatagramSection",
            });
        }

        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "MpeDatagramSection",
                expected: &[TABLE_ID],
            });
        }

        // Byte 1: SSI(1) | private(1) | reserved(2) | section_length[11:8].
        let section_syntax_indicator = (bytes[1] & 0x80) != 0;
        let private_indicator = (bytes[1] & 0x40) != 0;
        let section_length = (((bytes[1] & 0x0F) as usize) << 8) | bytes[2] as usize;
        let total = HEADER_LEN + section_length;
        if bytes.len() < total {
            return Err(Error::SectionLengthOverflow {
                declared: section_length,
                available: bytes.len() - HEADER_LEN,
            });
        }
        // The declared section must at least span the extension + trailer.
        if section_length < EXTENSION_LEN + CRC_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: section_length,
                available: bytes.len() - HEADER_LEN,
            });
        }

        // MAC scatter: byte 3 = MAC_6 (LSB), byte 4 = MAC_5,
        // bytes 8-11 = MAC_4, MAC_3, MAC_2, MAC_1 (MSB). Reassemble MSB-first.
        let mac_6 = bytes[3];
        let mac_5 = bytes[4];

        // Byte 5: reserved(2) | payload_sc(2) | address_sc(2) | LLC_SNAP(1) | cni(1).
        let payload_scrambling_control = (bytes[5] >> 4) & 0x03;
        let address_scrambling_control = (bytes[5] >> 2) & 0x03;
        let llc_snap_flag = (bytes[5] & 0x02) != 0;
        let current_next_indicator = (bytes[5] & 0x01) != 0;

        let section_number = bytes[6];
        let last_section_number = bytes[7];

        let mac_4 = bytes[8];
        let mac_3 = bytes[9];
        let mac_2 = bytes[10];
        let mac_1 = bytes[11];
        let mac_address = [mac_1, mac_2, mac_3, mac_4, mac_5, mac_6];

        let payload_start = HEADER_LEN + EXTENSION_LEN;
        let trailer_start = total - CRC_LEN;
        let payload = &bytes[payload_start..trailer_start];
        let checksum = [
            bytes[trailer_start],
            bytes[trailer_start + 1],
            bytes[trailer_start + 2],
            bytes[trailer_start + 3],
        ];

        Ok(MpeDatagramSection {
            section_syntax_indicator,
            private_indicator,
            mac_address,
            payload_scrambling_control,
            address_scrambling_control,
            llc_snap_flag,
            current_next_indicator,
            section_number,
            last_section_number,
            payload,
            checksum,
        })
    }
}

impl Serialize for MpeDatagramSection<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + EXTENSION_LEN + self.payload.len() + CRC_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }

        // 2-bit scrambling fields must fit; reject over-range values rather
        // than silently truncating (mirrors cit/sdt guarding derived fields).
        if self.payload_scrambling_control > 0x03 {
            return Err(Error::ReservedBitsViolation {
                field: "payload_scrambling_control",
                reason: "value exceeds 2-bit field",
            });
        }
        if self.address_scrambling_control > 0x03 {
            return Err(Error::ReservedBitsViolation {
                field: "address_scrambling_control",
                reason: "value exceeds 2-bit field",
            });
        }

        let section_length = (len - HEADER_LEN) as u16;

        buf[0] = TABLE_ID;
        // Byte 1: SSI(1) | private(1) | reserved(2)=11 | section_length[11:8].
        buf[1] = (u8::from(self.section_syntax_indicator) << 7)
            | (u8::from(self.private_indicator) << 6)
            | 0x30 // reserved bits set to 1
            | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;

        // MAC scatter: byte 3 = MAC_6 (mac_address[5]), byte 4 = MAC_5.
        buf[3] = self.mac_address[5];
        buf[4] = self.mac_address[4];

        // Byte 5: reserved(2)=11 | payload_sc(2) | address_sc(2) | LLC_SNAP(1) | cni(1).
        buf[5] = 0xC0
            | ((self.payload_scrambling_control & 0x03) << 4)
            | ((self.address_scrambling_control & 0x03) << 2)
            | (u8::from(self.llc_snap_flag) << 1)
            | u8::from(self.current_next_indicator);

        buf[6] = self.section_number;
        buf[7] = self.last_section_number;

        // bytes 8-11 = MAC_4, MAC_3, MAC_2, MAC_1.
        buf[8] = self.mac_address[3];
        buf[9] = self.mac_address[2];
        buf[10] = self.mac_address[1];
        buf[11] = self.mac_address[0];

        let payload_start = HEADER_LEN + EXTENSION_LEN;
        let trailer_start = payload_start + self.payload.len();
        buf[payload_start..trailer_start].copy_from_slice(self.payload);

        if self.section_syntax_indicator {
            // SSI=1 → recompute CRC_32 over the whole section up to the trailer.
            let crc = dvb_common::crc32_mpeg2::compute(&buf[..trailer_start]);
            buf[trailer_start..len].copy_from_slice(&crc.to_be_bytes());
        } else {
            // SSI=0 → ISO/IEC 13818-6 checksum we cannot recompute; re-emit
            // the preserved trailer bytes verbatim.
            buf[trailer_start..len].copy_from_slice(&self.checksum);
        }

        Ok(len)
    }
}

impl<'a> Table<'a> for MpeDatagramSection<'a> {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for MpeDatagramSection<'a> {
    /// `0x3E` is included in `DsmccSection`'s range `[(0x3A, 0x3F)]` and is
    /// NOT auto-dispatched to this type by the default dispatcher. Use
    /// `AnyTable::parse_as::<MpeDatagramSection>` or
    /// `MpeDatagramSection::parse` to obtain the typed MPE view.
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "MPE_DATAGRAM_SECTION";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a syntactically valid MPE datagram_section.
    ///
    /// `mac_address` is in network order (MAC_1 first). The 4-byte trailer is
    /// written from `trailer` verbatim (callers pass a computed CRC or an
    /// arbitrary checksum), matching what the serializer would emit for the
    /// `ssi == false` path.
    #[allow(clippy::too_many_arguments)]
    fn build_mpe(
        ssi: bool,
        private_indicator: bool,
        mac_address: [u8; 6],
        payload_sc: u8,
        address_sc: u8,
        llc_snap: bool,
        section_number: u8,
        last_section_number: u8,
        payload: &[u8],
        trailer: [u8; 4],
    ) -> Vec<u8> {
        let section_length = (EXTENSION_LEN + payload.len() + CRC_LEN) as u16;
        let flags = 0xC0
            | ((payload_sc & 0x03) << 4)
            | ((address_sc & 0x03) << 2)
            | (u8::from(llc_snap) << 1)
            | 0x01; // cni = 1
        let mut v = vec![
            TABLE_ID,
            (u8::from(ssi) << 7)
                | (u8::from(private_indicator) << 6)
                | 0x30
                | ((section_length >> 8) as u8 & 0x0F),
            (section_length & 0xFF) as u8,
            mac_address[5], // MAC_6
            mac_address[4], // MAC_5
            flags,
            section_number,
            last_section_number,
            mac_address[3], // MAC_4
            mac_address[2], // MAC_3
            mac_address[1], // MAC_2
            mac_address[0], // MAC_1
        ];
        v.extend_from_slice(payload);
        v.extend_from_slice(&trailer);
        v
    }

    #[test]
    fn parse_happy_path() {
        let mac = [0x01, 0x00, 0x5E, 0x12, 0x34, 0x56];
        let payload = [0xDE, 0xAD, 0xBE, 0xEF];
        let bytes = build_mpe(
            false,
            true,
            mac,
            0b10,
            0b01,
            true,
            2,
            3,
            &payload,
            [0xAA, 0xBB, 0xCC, 0xDD],
        );
        let sec = MpeDatagramSection::parse(&bytes).unwrap();
        assert!(!sec.section_syntax_indicator);
        assert!(sec.private_indicator);
        assert_eq!(sec.mac_address, mac);
        assert_eq!(sec.payload_scrambling_control, 0b10);
        assert_eq!(sec.address_scrambling_control, 0b01);
        assert!(sec.llc_snap_flag);
        assert!(sec.current_next_indicator);
        assert_eq!(sec.section_number, 2);
        assert_eq!(sec.last_section_number, 3);
        assert_eq!(sec.payload, &payload);
        assert_eq!(sec.checksum, [0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn mac_scatter_decoded_in_network_order() {
        // Distinct bytes per MAC position so a wrong scatter is obvious.
        let mac = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
        let bytes = build_mpe(true, false, mac, 0, 0, false, 0, 0, &[], [0, 0, 0, 0]);
        // Verify the on-wire scatter directly:
        assert_eq!(bytes[3], 0x66, "byte 3 = MAC_6 (LSB)");
        assert_eq!(bytes[4], 0x55, "byte 4 = MAC_5");
        assert_eq!(bytes[8], 0x44, "byte 8 = MAC_4");
        assert_eq!(bytes[9], 0x33, "byte 9 = MAC_3");
        assert_eq!(bytes[10], 0x22, "byte 10 = MAC_2");
        assert_eq!(bytes[11], 0x11, "byte 11 = MAC_1 (MSB)");
        let sec = MpeDatagramSection::parse(&bytes).unwrap();
        assert_eq!(sec.mac_address, mac);
    }

    #[test]
    fn parse_empty_payload() {
        let bytes = build_mpe(
            true,
            false,
            [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
            0,
            0,
            false,
            0,
            0,
            &[],
            [0, 0, 0, 0],
        );
        let sec = MpeDatagramSection::parse(&bytes).unwrap();
        assert!(sec.payload.is_empty());
        assert_eq!(sec.mac_address, [0xFF; 6]);
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let mut bytes = build_mpe(
            true,
            false,
            [0; 6],
            0,
            0,
            false,
            0,
            0,
            &[0x01],
            [0, 0, 0, 0],
        );
        bytes[0] = 0x3F; // valid DSM-CC range value, but not the MPE table_id
        assert!(matches!(
            MpeDatagramSection::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x3F, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = MpeDatagramSection::parse(&[TABLE_ID, 0x00]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_section_length_overflow() {
        let mut bytes = build_mpe(
            true,
            false,
            [0; 6],
            0,
            0,
            false,
            0,
            0,
            &[0xAA],
            [0, 0, 0, 0],
        );
        // Inflate declared section_length well past the actual buffer.
        let fake_sl: u16 = (bytes.len() as u16) + 100 - HEADER_LEN as u16;
        bytes[1] = (bytes[1] & 0xF0) | ((fake_sl >> 8) as u8 & 0x0F);
        bytes[2] = (fake_sl & 0xFF) as u8;
        assert!(matches!(
            MpeDatagramSection::parse(&bytes).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn round_trip_identity_ssi_set_crc() {
        // SSI=1: serialize recomputes CRC_32. Build with a matching CRC so the
        // parsed `checksum` field also matches (it is ignored when SSI=1, but
        // we set it correctly to assert full struct equality).
        let mac = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF];
        let payload = [0x45, 0x00, 0x00, 0x1C, 0x00, 0x01];
        let original = MpeDatagramSection {
            section_syntax_indicator: true,
            private_indicator: false,
            mac_address: mac,
            payload_scrambling_control: 0,
            address_scrambling_control: 0,
            llc_snap_flag: false,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            payload: &payload,
            checksum: [0, 0, 0, 0],
        };
        let mut buf = vec![0u8; original.serialized_len()];
        original.serialize_into(&mut buf).unwrap();
        let parsed = MpeDatagramSection::parse(&buf).unwrap();
        // Everything but the (ignored-on-SSI=1) checksum must match.
        assert!(parsed.section_syntax_indicator);
        assert_eq!(parsed.mac_address, mac);
        assert_eq!(parsed.payload, &payload);
        // Re-serialize the parsed value: bytes must be byte-identical.
        let mut buf2 = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2);
    }

    #[test]
    fn round_trip_identity_ssi_clear_checksum_preserved() {
        // SSI=0: the trailer is an opaque checksum preserved verbatim.
        let mac = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
        let payload = [0x11, 0x22, 0x33];
        let trailer = [0x12, 0x34, 0x56, 0x78];
        let bytes = build_mpe(false, true, mac, 0b11, 0b10, true, 1, 5, &payload, trailer);
        let parsed = MpeDatagramSection::parse(&bytes).unwrap();
        assert_eq!(parsed.checksum, trailer);
        let mut buf = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut buf).unwrap();
        // Full byte-for-byte identity, including the preserved checksum.
        assert_eq!(buf, bytes);
        assert_eq!(MpeDatagramSection::parse(&buf).unwrap(), parsed);
    }

    #[test]
    fn serialize_rejects_output_buffer_too_small() {
        let sec = MpeDatagramSection {
            section_syntax_indicator: true,
            private_indicator: false,
            mac_address: [0; 6],
            payload_scrambling_control: 0,
            address_scrambling_control: 0,
            llc_snap_flag: false,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            payload: &[],
            checksum: [0; 4],
        };
        let mut buf = [0u8; 2];
        assert!(matches!(
            sec.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[test]
    fn serialize_rejects_over_range_scrambling_control() {
        let sec = MpeDatagramSection {
            section_syntax_indicator: true,
            private_indicator: false,
            mac_address: [0; 6],
            payload_scrambling_control: 0x04, // > 2-bit field
            address_scrambling_control: 0,
            llc_snap_flag: false,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            payload: &[],
            checksum: [0; 4],
        };
        let mut buf = vec![0u8; sec.serialized_len()];
        assert!(matches!(
            sec.serialize_into(&mut buf).unwrap_err(),
            Error::ReservedBitsViolation {
                field: "payload_scrambling_control",
                ..
            }
        ));
    }

    #[test]
    fn table_trait_constants() {
        assert_eq!(<MpeDatagramSection as Table>::TABLE_ID, 0x3E);
        assert_eq!(<MpeDatagramSection as Table>::PID, 0x0000);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_json_round_trip() {
        let payload = [0xAB, 0xCD];
        let bytes = build_mpe(
            false,
            true,
            [0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F],
            0b01,
            0b11,
            true,
            3,
            7,
            &payload,
            [0xDE, 0xAD, 0xBE, 0xEF],
        );
        let sec = MpeDatagramSection::parse(&bytes).unwrap();
        let j = serde_json::to_string(&sec).unwrap();

        // The borrowed `payload: &[u8]` field cannot be JSON-deserialized
        // zero-copy (serde_json renders it as a number sequence, not a
        // borrowed byte array — the same constraint that affects every
        // borrowed-slice table in the crate). Unlike cat.rs, whose fields are
        // all owned and so round-trip via `from_str::<Self>`, we exercise the
        // serde derive through the WIRE form: a re-parse of the same bytes
        // must serialize to byte-identical JSON. This pins the Serialize impl.
        let reparsed = MpeDatagramSection::parse(&bytes).unwrap();
        assert_eq!(serde_json::to_string(&reparsed).unwrap(), j);

        // And confirm the JSON carries the decoded fields: network-order MAC,
        // both 2-bit scrambling controls, and the preserved checksum trailer.
        assert!(j.contains("\"mac_address\":[10,11,12,13,14,15]"));
        assert!(j.contains("\"payload_scrambling_control\":1"));
        assert!(j.contains("\"address_scrambling_control\":3"));
        assert!(j.contains("\"checksum\":[222,173,190,239]"));
    }
}
