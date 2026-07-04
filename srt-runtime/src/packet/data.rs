//! SRT data packet — `draft-sharabayko-srt-01` §3.1, Figure 3.
//!
//! ```text
//! word0  0|                Packet Sequence Number (31)                 |
//! word1  PP|O|KK|R|                Message Number (26)                 |
//! word2                          Timestamp (32)
//! word3                    Destination Socket ID (32)
//! rest                              Data
//! ```

use super::{Error, Result, SEQ_NUMBER_MASK, SRT_HEADER_LEN, be32, put_be32};

/// Bit width of the Message Number field (§3.1).
const MESSAGE_NUMBER_BITS: u32 = 26;
/// Mask for the 26-bit Message Number field (§3.1).
const MESSAGE_NUMBER_MASK: u32 = (1 << MESSAGE_NUMBER_BITS) - 1;

/// `PP` (Packet Position Flag) wire values (§3.1).
const PP_MIDDLE: u8 = 0b00;
const PP_LAST: u8 = 0b01;
const PP_FIRST: u8 = 0b10;
const PP_SOLO: u8 = 0b11;

/// `KK` (Key-based Encryption Flag) wire values (§3.1).
const KK_NOT_ENCRYPTED: u8 = 0b00;
const KK_EVEN: u8 = 0b01;
const KK_ODD: u8 = 0b10;
/// `11b` — reserved; only meaningful on control packets (Key Material `KK`).
#[cfg(test)]
const KK_CONTROL_ONLY: u8 = 0b11;

/// `PP`: Packet Position Flag — position of the data packet in its message
/// (`draft-sharabayko-srt-01` §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum PacketPosition {
    /// `10b`: first packet of the message.
    First,
    /// `00b`: a packet in the middle of the message.
    Middle,
    /// `01b`: last packet of the message.
    Last,
    /// `11b`: the whole message fits in a single data packet.
    Solo,
}

impl PacketPosition {
    /// Decode the 2-bit `PP` field.
    pub fn from_bits(v: u8) -> Self {
        match v & 0b11 {
            PP_FIRST => PacketPosition::First,
            PP_LAST => PacketPosition::Last,
            PP_SOLO => PacketPosition::Solo,
            _ => PacketPosition::Middle, // PP_MIDDLE, and unreachable otherwise (v & 0b11 <= 3)
        }
    }

    /// The 2-bit wire value.
    pub fn to_bits(self) -> u8 {
        match self {
            PacketPosition::First => PP_FIRST,
            PacketPosition::Middle => PP_MIDDLE,
            PacketPosition::Last => PP_LAST,
            PacketPosition::Solo => PP_SOLO,
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            PacketPosition::First => "first",
            PacketPosition::Middle => "middle",
            PacketPosition::Last => "last",
            PacketPosition::Solo => "solo",
        }
    }
}

broadcast_common::impl_spec_display!(PacketPosition);

/// `KK`: Key-based Encryption Flag — whether/how the `Data` field is
/// encrypted (`draft-sharabayko-srt-01` §3.1, §6). `11b` is reserved for
/// control packets (Key Material's own `KK` field, §3.2.2) and should not
/// appear on a data packet, but is decoded rather than rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EncryptionKeyField {
    /// `00b`: the payload is not encrypted.
    NotEncrypted,
    /// `01b`: encrypted with the even key.
    Even,
    /// `10b`: encrypted with the odd key.
    Odd,
    /// `11b`: reserved — only meaningful on control packets.
    Reserved(u8),
}

impl EncryptionKeyField {
    /// Decode the 2-bit `KK` field.
    pub fn from_bits(v: u8) -> Self {
        match v & 0b11 {
            KK_NOT_ENCRYPTED => EncryptionKeyField::NotEncrypted,
            KK_EVEN => EncryptionKeyField::Even,
            KK_ODD => EncryptionKeyField::Odd,
            other => EncryptionKeyField::Reserved(other),
        }
    }

    /// The 2-bit wire value.
    pub fn to_bits(self) -> u8 {
        match self {
            EncryptionKeyField::NotEncrypted => KK_NOT_ENCRYPTED,
            EncryptionKeyField::Even => KK_EVEN,
            EncryptionKeyField::Odd => KK_ODD,
            EncryptionKeyField::Reserved(v) => v & 0b11,
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            EncryptionKeyField::NotEncrypted => "not encrypted",
            EncryptionKeyField::Even => "even key",
            EncryptionKeyField::Odd => "odd key",
            EncryptionKeyField::Reserved(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(EncryptionKeyField, Reserved);

/// An SRT data packet (`draft-sharabayko-srt-01` §3.1, Figure 3).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataPacket<'a> {
    /// Packet Sequence Number — 31 bits.
    pub seq_number: u32,
    /// `PP`: Packet Position Flag.
    pub position: PacketPosition,
    /// `O`: Order Flag — deliver in order (`true`) or not (`false`).
    pub in_order: bool,
    /// `KK`: Key-based Encryption Flag.
    pub key_flag: EncryptionKeyField,
    /// `R`: Retransmitted Packet Flag.
    pub retransmitted: bool,
    /// Message Number — 26 bits.
    pub message_number: u32,
    /// Timestamp, in microseconds relative to connection establishment (§3).
    pub timestamp: u32,
    /// Destination Socket ID (§3).
    pub dest_socket_id: u32,
    /// The payload. Length is whatever remains of the UDP datagram.
    pub data: &'a [u8],
}

const KK_SHIFT: u32 = 27;
const PP_SHIFT: u32 = 30;
const O_SHIFT: u32 = 29;
const R_SHIFT: u32 = 26;

impl<'a> DataPacket<'a> {
    /// Parse a data packet from `bytes` (the full SRT packet, header + data).
    ///
    /// # Errors
    /// [`Error::BufferTooShort`] if shorter than the 16-byte header;
    /// [`Error::WrongPacketKind`] if the `F` bit is set (this is a control
    /// packet).
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < SRT_HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: SRT_HEADER_LEN,
                have: bytes.len(),
                what: "SRT data packet header",
            });
        }
        let word0 = be32(bytes, 0);
        if word0 & super::F_BIT != 0 {
            return Err(Error::WrongPacketKind {
                expected: "data packet (F=0)",
            });
        }
        let word1 = be32(bytes, 4);
        let seq_number = word0 & SEQ_NUMBER_MASK;
        let position = PacketPosition::from_bits((word1 >> PP_SHIFT) as u8);
        let in_order = (word1 >> O_SHIFT) & 1 != 0;
        let key_flag = EncryptionKeyField::from_bits((word1 >> KK_SHIFT) as u8);
        let retransmitted = (word1 >> R_SHIFT) & 1 != 0;
        let message_number = word1 & MESSAGE_NUMBER_MASK;
        let timestamp = be32(bytes, 8);
        let dest_socket_id = be32(bytes, 12);
        let data = &bytes[SRT_HEADER_LEN..];
        Ok(DataPacket {
            seq_number,
            position,
            in_order,
            key_flag,
            retransmitted,
            message_number,
            timestamp,
            dest_socket_id,
            data,
        })
    }

    /// Number of bytes [`Self::serialize_into`] will write.
    pub fn serialized_len(&self) -> usize {
        SRT_HEADER_LEN + self.data.len()
    }

    /// Serialize this data packet into `buf`.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        if self.seq_number > SEQ_NUMBER_MASK {
            return Err(Error::FieldTooWide {
                what: "Packet Sequence Number",
                value: u64::from(self.seq_number),
                bits: 31,
            });
        }
        if self.message_number > MESSAGE_NUMBER_MASK {
            return Err(Error::FieldTooWide {
                what: "Message Number",
                value: u64::from(self.message_number),
                bits: MESSAGE_NUMBER_BITS,
            });
        }
        let word0 = self.seq_number; // F bit (bit 31) stays clear.
        let word1 = (u32::from(self.position.to_bits()) << PP_SHIFT)
            | (u32::from(self.in_order) << O_SHIFT)
            | (u32::from(self.key_flag.to_bits()) << KK_SHIFT)
            | (u32::from(self.retransmitted) << R_SHIFT)
            | self.message_number;
        put_be32(buf, 0, word0);
        put_be32(buf, 4, word1);
        put_be32(buf, 8, self.timestamp);
        put_be32(buf, 12, self.dest_socket_id);
        buf[SRT_HEADER_LEN..len].copy_from_slice(self.data);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DataPacket<'static> {
        DataPacket {
            seq_number: 0x0123_4567,
            position: PacketPosition::Solo,
            in_order: true,
            key_flag: EncryptionKeyField::Even,
            retransmitted: false,
            message_number: 0x0155_5555, // fits 26 bits
            timestamp: 0xAABB_CCDD,
            dest_socket_id: 0x1122_3344,
            data: &[0x47, 0x00, 0x01, 0x02],
        }
    }

    #[test]
    fn round_trip_bytes_are_hand_computed() {
        let pkt = sample();
        let mut buf = [0u8; 20];
        let n = pkt.serialize_into(&mut buf).unwrap();
        assert_eq!(n, 20);

        // word0: F=0, seq=0x01234567 -> top bit clear already.
        assert_eq!(&buf[0..4], &0x0123_4567u32.to_be_bytes());
        // word1: PP=11 (Solo), O=1, KK=01 (Even), R=0 (omitted: 0<<26 is a
        // no-op), msg=0x01555555.
        let expected_word1 = (0b11u32 << 30) | (1u32 << 29) | (0b01u32 << 27) | 0x0155_5555;
        assert_eq!(&buf[4..8], &expected_word1.to_be_bytes());
        assert_eq!(&buf[8..12], &0xAABB_CCDDu32.to_be_bytes());
        assert_eq!(&buf[12..16], &0x1122_3344u32.to_be_bytes());
        assert_eq!(&buf[16..20], &[0x47, 0x00, 0x01, 0x02]);

        let parsed = DataPacket::parse(&buf).unwrap();
        assert_eq!(parsed, pkt);
    }

    #[test]
    fn mutate_field_changes_bytes() {
        let mut pkt = sample();
        let mut buf1 = [0u8; 20];
        pkt.serialize_into(&mut buf1).unwrap();
        pkt.retransmitted = true;
        let mut buf2 = [0u8; 20];
        pkt.serialize_into(&mut buf2).unwrap();
        assert_ne!(buf1, buf2);
    }

    #[test]
    fn rejects_control_packet_bytes() {
        let mut buf = [0u8; 16];
        buf[0] = 0x80; // F=1
        assert_eq!(
            DataPacket::parse(&buf).unwrap_err(),
            Error::WrongPacketKind {
                expected: "data packet (F=0)"
            }
        );
    }

    #[test]
    fn all_packet_positions_round_trip() {
        for p in [
            PacketPosition::First,
            PacketPosition::Middle,
            PacketPosition::Last,
            PacketPosition::Solo,
        ] {
            assert_eq!(PacketPosition::from_bits(p.to_bits()), p);
        }
    }

    #[test]
    fn all_key_flags_round_trip() {
        for k in [
            EncryptionKeyField::NotEncrypted,
            EncryptionKeyField::Even,
            EncryptionKeyField::Odd,
        ] {
            assert_eq!(EncryptionKeyField::from_bits(k.to_bits()), k);
        }
        assert_eq!(
            EncryptionKeyField::from_bits(KK_CONTROL_ONLY),
            EncryptionKeyField::Reserved(KK_CONTROL_ONLY)
        );
    }

    #[test]
    fn overwide_seq_number_errs() {
        let mut pkt = sample();
        pkt.seq_number = 0x8000_0000;
        let mut buf = [0u8; 20];
        assert!(matches!(
            pkt.serialize_into(&mut buf),
            Err(Error::FieldTooWide { .. })
        ));
    }
}
