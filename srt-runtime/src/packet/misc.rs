//! The CIF-less / single-scalar-CIF control packets: Keep-Alive (§3.2.3),
//! Congestion Warning (§3.2.6), Shutdown (§3.2.7), ACKACK (§3.2.8), Message
//! Drop Request (§3.2.9), and Peer Error (§3.2.10).

use super::number::{AckNumber, MESSAGE_NUMBER_MASK, MessageNumber, SeqNo, SocketId, SrtTimestamp};
use super::{Error, Result, be32, put_be32};

/// Length of a Drop Request CIF and the offset of its Last Packet Sequence
/// Number (§3.2.5, Figure 15).
const DROP_REQ_CIF_LEN: usize = 8;
const DROP_REQ_LAST_SEQ_OFFSET: usize = 4;
use broadcast_core::error::BufferTooShort;

/// Peer error code for a file-system error — the only value
/// `draft-sharabayko-srt-01` §3.2.10 currently defines.
pub const PEER_ERROR_FILE_SYSTEM: u32 = 4000;

/// Keep-Alive control packet (§3.2.3, Figure 12). No CIF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct KeepAlivePacket {
    /// Timestamp (§3).
    pub timestamp: SrtTimestamp,
    /// Destination Socket ID (§3).
    pub dest_socket_id: SocketId,
    /// `true` when the CIF carries libsrt's 4-byte zero pad; `false` for the
    /// empty CIF of draft-sharabayko-srt-01.
    pub libsrt_pad: bool,
}

/// Congestion Warning control packet (§3.2.6, Figure 15). Reserved for future
/// use; no CIF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CongestionWarningPacket {
    /// Timestamp (§3).
    pub timestamp: SrtTimestamp,
    /// Destination Socket ID (§3).
    pub dest_socket_id: SocketId,
    /// `true` when the CIF carries libsrt's 4-byte zero pad; `false` for the
    /// empty CIF of draft-sharabayko-srt-01.
    pub libsrt_pad: bool,
}

/// Shutdown control packet (§3.2.7, Figure 16). No CIF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ShutdownPacket {
    /// Timestamp (§3).
    pub timestamp: SrtTimestamp,
    /// Destination Socket ID (§3).
    pub dest_socket_id: SocketId,
    /// `true` when the CIF carries libsrt's 4-byte zero pad; `false` for the
    /// empty CIF of draft-sharabayko-srt-01.
    pub libsrt_pad: bool,
}

/// ACKACK control packet (§3.2.8, Figure 17). No CIF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AckAckPacket {
    /// Acknowledgement Number of the Full ACK being acknowledged (§3.2.8,
    /// Figure 17).
    pub ack_number: AckNumber,
    /// Timestamp (§3).
    pub timestamp: SrtTimestamp,
    /// Destination Socket ID (§3).
    pub dest_socket_id: SocketId,
    /// `true` when the CIF carries libsrt's 4-byte zero pad; `false` for the
    /// empty CIF of draft-sharabayko-srt-01.
    pub libsrt_pad: bool,
}

/// Message Drop Request control packet (§3.2.9, Figure 18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DropReqPacket {
    /// The message number requested to be dropped (`0` if the sender no
    /// longer has the packets and cannot restore it).
    ///
    /// §3.2.9 (Figure 18) calls this field 32 bits wide, but it names a
    /// message, so it is bounded here by the 26-bit Message Number field of
    /// §3.1 it refers to — the only values the sender can ever produce.
    pub message_number: MessageNumber,
    /// Timestamp (§3).
    pub timestamp: SrtTimestamp,
    /// Destination Socket ID (§3).
    pub dest_socket_id: SocketId,
    /// First Packet Sequence Number of the range to drop.
    pub first_seq: SeqNo,
    /// Last Packet Sequence Number of the range to drop.
    pub last_seq: SeqNo,
}

impl DropReqPacket {
    pub(crate) fn parse_cif(
        message_number: u32,
        timestamp: SrtTimestamp,
        dest_socket_id: SocketId,
        cif: &[u8],
    ) -> Result<Self> {
        if cif.len() != DROP_REQ_CIF_LEN {
            return Err(Error::BufferTooShort(BufferTooShort {
                need: DROP_REQ_CIF_LEN,
                have: cif.len(),
                what: "drop request CIF",
            }));
        }
        let message_number = MessageNumber::try_new(message_number).map_err(|_| {
            Error::FieldOverflow(broadcast_core::error::FieldOverflow {
                field: "Message Number",
                value: u64::from(message_number),
                max: u64::from(MESSAGE_NUMBER_MASK),
            })
        })?;
        Ok(DropReqPacket {
            message_number,
            timestamp,
            dest_socket_id,
            first_seq: SeqNo::from_wire_masked(be32(cif, 0)),
            last_seq: SeqNo::from_wire_masked(be32(cif, DROP_REQ_LAST_SEQ_OFFSET)),
        })
    }

    pub(crate) fn cif_len(&self) -> usize {
        DROP_REQ_CIF_LEN
    }

    pub(crate) fn write_cif(&self, buf: &mut [u8]) -> Result<()> {
        put_be32(buf, 0, self.first_seq.value());
        put_be32(buf, DROP_REQ_LAST_SEQ_OFFSET, self.last_seq.value());
        Ok(())
    }
}

/// Peer Error control packet (§3.2.10, Figure 19). No CIF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PeerErrorPacket {
    /// Peer error code (see [`PEER_ERROR_FILE_SYSTEM`]).
    pub error_code: u32,
    /// Timestamp (§3).
    pub timestamp: SrtTimestamp,
    /// Destination Socket ID (§3).
    pub dest_socket_id: SocketId,
    /// `true` when the CIF carries libsrt's 4-byte zero pad; `false` for the
    /// empty CIF of draft-sharabayko-srt-01.
    pub libsrt_pad: bool,
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::super::control::ControlPacket;
    use super::super::data::MESSAGE_NUMBER_MASK as MESSAGE_NUMBER_MAX;
    use super::*;

    fn msg(n: u32) -> MessageNumber {
        MessageNumber::try_new(n).expect("fits 26 bits")
    }

    fn sn(n: u32) -> SeqNo {
        SeqNo::try_new(n).expect("fits 31 bits")
    }

    #[test]
    fn drop_req_round_trips() {
        let d = DropReqPacket {
            message_number: msg(7),
            timestamp: SrtTimestamp::from_wire(100),
            dest_socket_id: SocketId::from_wire(200),
            first_seq: sn(10),
            last_seq: sn(20),
        };
        let pkt = ControlPacket::DropReq(d);
        let mut buf = [0u8; 24];
        let n = pkt.serialize_into(&mut buf).unwrap();
        assert_eq!(n, 24);
        assert_eq!(&buf[4..8], &7u32.to_be_bytes()); // message number in word1
        assert_eq!(&buf[16..20], &10u32.to_be_bytes());
        assert_eq!(&buf[20..24], &20u32.to_be_bytes());
        let parsed = ControlPacket::parse(&buf).unwrap();
        assert_eq!(parsed, pkt);
    }

    /// §3.2.9's fields are 32-bit words on the wire, but the two sequence
    /// numbers share the data packet's 31-bit space (§3.1) and the Message
    /// Number is the 26-bit one: a peer's top-bit-set values are accepted
    /// (masked for the sequence numbers, since the typed `SeqNo` cannot hold
    /// them) and serialize back to the same masked bytes — never a
    /// serialize-time refusal of a packet parse accepted.
    #[test]
    fn drop_req_wide_wire_numbers_still_serialize() {
        let mut buf = alloc::vec![0u8; 24];
        ControlPacket::DropReq(DropReqPacket {
            message_number: msg(7),
            timestamp: SrtTimestamp::from_wire(100),
            dest_socket_id: SocketId::from_wire(200),
            first_seq: sn(10),
            last_seq: sn(20),
        })
        .serialize_into(&mut buf)
        .unwrap();
        buf[16..20].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
        buf[20..24].copy_from_slice(&0xFFFF_FFFEu32.to_be_bytes());
        let parsed = ControlPacket::parse(&buf).unwrap();
        let ControlPacket::DropReq(d) = parsed else {
            panic!("a drop request");
        };
        assert_eq!(d.first_seq, SeqNo::MAX);
        assert_eq!(d.last_seq, sn(0x7FFF_FFFE));
        let mut out = alloc::vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut out).unwrap();
        assert_eq!(&out[0..16], &buf[0..16]);
        assert_eq!(&out[16..20], &SeqNo::MAX.value().to_be_bytes());
        assert_eq!(&out[20..24], &0x7FFF_FFFEu32.to_be_bytes());
        assert_eq!(ControlPacket::parse(&out).unwrap(), parsed);
    }

    /// A DROPREQ whose Message Number word is wider than the 26-bit Message
    /// Number it names is rejected at parse, so `parse` never yields a packet
    /// `serialize` would refuse (#2766's bound now sits at the parse edge).
    #[test]
    fn drop_req_rejects_an_over_wide_message_number_at_parse() {
        let mut buf = alloc::vec![0u8; 24];
        ControlPacket::DropReq(DropReqPacket {
            message_number: msg(MESSAGE_NUMBER_MAX),
            timestamp: SrtTimestamp::from_wire(100),
            dest_socket_id: SocketId::from_wire(200),
            first_seq: sn(10),
            last_seq: sn(20),
        })
        .serialize_into(&mut buf)
        .unwrap();
        assert!(ControlPacket::parse(&buf).is_ok());

        for wide in [MESSAGE_NUMBER_MAX + 1, 0x8E8E_8E8E, u32::MAX] {
            buf[4..8].copy_from_slice(&wide.to_be_bytes());
            assert_eq!(
                ControlPacket::parse(&buf).unwrap_err(),
                Error::FieldOverflow(broadcast_core::error::FieldOverflow {
                    field: "Message Number",
                    value: u64::from(wide),
                    max: u64::from(MESSAGE_NUMBER_MAX),
                })
            );
        }
    }

    /// An ACKACK echoes a Full ACK's Acknowledgement Number verbatim, and
    /// that field is a whole 32-bit word (§3.2.8, Figure 17) — a value above
    /// the 31-bit sequence space round-trips.
    #[test]
    fn ackack_carries_every_32_bit_ack_number() {
        for ack_number in [0, 1, 0x8000_0000, 0x8E8E_8E8E, u32::MAX] {
            let pkt = ControlPacket::AckAck(AckAckPacket {
                ack_number: AckNumber::from_wire(ack_number),
                timestamp: SrtTimestamp::from_wire(7),
                dest_socket_id: SocketId::from_wire(8),
                libsrt_pad: false,
            });
            let mut buf = alloc::vec![0u8; pkt.serialized_len()];
            pkt.serialize_into(&mut buf).unwrap();
            assert_eq!(&buf[4..8], &ack_number.to_be_bytes());
            assert_eq!(ControlPacket::parse(&buf).unwrap(), pkt);
        }
    }

    #[test]
    fn keepalive_congestion_shutdown_ackack_peererror_round_trip() {
        let cases: Vec<ControlPacket> = alloc::vec![
            ControlPacket::KeepAlive(KeepAlivePacket {
                timestamp: SrtTimestamp::from_wire(1),
                dest_socket_id: SocketId::from_wire(2),
                libsrt_pad: true,
            }),
            ControlPacket::CongestionWarning(CongestionWarningPacket {
                timestamp: SrtTimestamp::from_wire(3),
                dest_socket_id: SocketId::from_wire(4),
                libsrt_pad: true,
            }),
            ControlPacket::Shutdown(ShutdownPacket {
                timestamp: SrtTimestamp::from_wire(5),
                dest_socket_id: SocketId::from_wire(6),
                libsrt_pad: true,
            }),
            ControlPacket::AckAck(AckAckPacket {
                ack_number: AckNumber::from_wire(9),
                timestamp: SrtTimestamp::from_wire(7),
                dest_socket_id: SocketId::from_wire(8),
                libsrt_pad: true,
            }),
            ControlPacket::PeerError(PeerErrorPacket {
                error_code: PEER_ERROR_FILE_SYSTEM,
                timestamp: SrtTimestamp::from_wire(11),
                dest_socket_id: SocketId::from_wire(12),
                libsrt_pad: true,
            }),
        ];
        for pkt in cases {
            // 20 bytes when libsrt_pad is true: serializes with the 4-byte
            // zero pad (see `control::LIBSRT_CIF_PAD_LEN`).
            let mut buf = [0u8; 20];
            let n = pkt.serialize_into(&mut buf).unwrap();
            assert_eq!(n, 20);
            assert_eq!(&buf[16..20], &[0, 0, 0, 0]);
            let parsed = ControlPacket::parse(&buf).unwrap();
            assert_eq!(parsed, pkt);
        }
    }

    /// A peer (e.g. real libsrt — see `tests/it/libsrt_fixtures.rs`) may also
    /// send these types with the pure-spec empty CIF instead of the pad.
    /// Both 16-byte (no-pad) and 20-byte (with-pad) wire shapes parse correctly.
    #[test]
    fn keepalive_congestion_shutdown_ackack_peererror_accept_empty_cif() {
        let cases: Vec<ControlPacket> = alloc::vec![
            ControlPacket::KeepAlive(KeepAlivePacket {
                timestamp: SrtTimestamp::from_wire(1),
                dest_socket_id: SocketId::from_wire(2),
                libsrt_pad: false,
            }),
            ControlPacket::CongestionWarning(CongestionWarningPacket {
                timestamp: SrtTimestamp::from_wire(3),
                dest_socket_id: SocketId::from_wire(4),
                libsrt_pad: false,
            }),
            ControlPacket::Shutdown(ShutdownPacket {
                timestamp: SrtTimestamp::from_wire(5),
                dest_socket_id: SocketId::from_wire(6),
                libsrt_pad: false,
            }),
            ControlPacket::AckAck(AckAckPacket {
                ack_number: AckNumber::from_wire(9),
                timestamp: SrtTimestamp::from_wire(7),
                dest_socket_id: SocketId::from_wire(8),
                libsrt_pad: false,
            }),
            ControlPacket::PeerError(PeerErrorPacket {
                error_code: PEER_ERROR_FILE_SYSTEM,
                timestamp: SrtTimestamp::from_wire(11),
                dest_socket_id: SocketId::from_wire(12),
                libsrt_pad: false,
            }),
        ];
        for pkt in cases {
            // Serialize with libsrt_pad: false -> 16 bytes
            let mut buf = [0u8; 16];
            let n = pkt.serialized_len();
            assert_eq!(n, 16, "16-byte packets with libsrt_pad: false");
            pkt.serialize_into(&mut buf).unwrap();

            // Parse the 16-byte form back
            let parsed = ControlPacket::parse(&buf).unwrap();
            assert_eq!(parsed, pkt);
        }
    }

    /// A 16-byte (pure-spec empty CIF) KEEPALIVE packet parses and
    /// round-trips byte-identically.
    #[test]
    fn sixteen_byte_keepalive_round_trip_byte_identical() {
        let pkt = ControlPacket::KeepAlive(KeepAlivePacket {
            timestamp: SrtTimestamp::from_wire(0x1234),
            dest_socket_id: SocketId::from_wire(0x5678),
            libsrt_pad: false,
        });
        let mut buf = [0u8; 16];
        let n = pkt.serialize_into(&mut buf).unwrap();
        assert_eq!(n, 16, "16-byte packet with libsrt_pad: false");

        let parsed = ControlPacket::parse(&buf).unwrap();
        assert_eq!(parsed, pkt);

        let mut reserialized = [0u8; 16];
        let n = parsed.serialize_into(&mut reserialized).unwrap();
        assert_eq!(n, 16);
        assert_eq!(&reserialized[..], &buf[..], "round-trip is byte-identical");
    }

    /// A 20-byte (libsrt's 4-byte zero pad) KEEPALIVE packet parses and
    /// round-trips byte-identically.
    #[test]
    fn twenty_byte_keepalive_round_trip_byte_identical() {
        let pkt = ControlPacket::KeepAlive(KeepAlivePacket {
            timestamp: SrtTimestamp::from_wire(0x1234),
            dest_socket_id: SocketId::from_wire(0x5678),
            libsrt_pad: true,
        });
        let mut buf = [0u8; 20];
        let n = pkt.serialize_into(&mut buf).unwrap();
        assert_eq!(n, 20, "20-byte packet with libsrt_pad: true");
        assert_eq!(&buf[16..20], &[0, 0, 0, 0], "CIF is 4-byte zero pad");

        let parsed = ControlPacket::parse(&buf).unwrap();
        assert_eq!(parsed, pkt);

        let mut reserialized = [0u8; 20];
        let n = parsed.serialize_into(&mut reserialized).unwrap();
        assert_eq!(n, 20);
        assert_eq!(&reserialized[..], &buf[..], "round-trip is byte-identical");
    }

    #[test]
    fn keepalive_rejects_trailing_bytes() {
        // Neither 0 (pure spec) nor 4 (libsrt's pad, §control::LIBSRT_CIF_PAD_LEN):
        // 1 stray byte must still be rejected.
        let mut buf = [0u8; 17];
        buf[0] = 0x80; // F=1
        buf[1] = 0x01; // control type = 1 (KEEPALIVE)
        assert!(matches!(
            ControlPacket::parse(&buf),
            Err(Error::UnexpectedTrailingBytes { .. })
        ));
    }

    #[test]
    fn keepalive_rejects_nonzero_pad() {
        let mut buf = [0u8; 20];
        buf[0] = 0x80; // F=1
        buf[1] = 0x01; // control type = 1 (KEEPALIVE)
        buf[19] = 0x01; // non-zero byte in the 4-byte pad
        assert!(matches!(
            ControlPacket::parse(&buf),
            Err(Error::ReservedBits(_))
        ));
    }
}
