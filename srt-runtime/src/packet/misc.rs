//! The CIF-less / single-scalar-CIF control packets: Keep-Alive (§3.2.3),
//! Congestion Warning (§3.2.6), Shutdown (§3.2.7), ACKACK (§3.2.8), Message
//! Drop Request (§3.2.9), and Peer Error (§3.2.10).

use super::{Error, Result, be32, put_be32};

/// Peer error code for a file-system error — the only value
/// `draft-sharabayko-srt-01` §3.2.10 currently defines.
pub const PEER_ERROR_FILE_SYSTEM: u32 = 4000;

/// Keep-Alive control packet (§3.2.3, Figure 12). No CIF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct KeepAlivePacket {
    /// Timestamp (§3).
    pub timestamp: u32,
    /// Destination Socket ID (§3).
    pub dest_socket_id: u32,
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
    pub timestamp: u32,
    /// Destination Socket ID (§3).
    pub dest_socket_id: u32,
    /// `true` when the CIF carries libsrt's 4-byte zero pad; `false` for the
    /// empty CIF of draft-sharabayko-srt-01.
    pub libsrt_pad: bool,
}

/// Shutdown control packet (§3.2.7, Figure 16). No CIF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ShutdownPacket {
    /// Timestamp (§3).
    pub timestamp: u32,
    /// Destination Socket ID (§3).
    pub dest_socket_id: u32,
    /// `true` when the CIF carries libsrt's 4-byte zero pad; `false` for the
    /// empty CIF of draft-sharabayko-srt-01.
    pub libsrt_pad: bool,
}

/// ACKACK control packet (§3.2.8, Figure 17). No CIF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AckAckPacket {
    /// Acknowledgement Number of the Full ACK being acknowledged.
    pub ack_number: u32,
    /// Timestamp (§3).
    pub timestamp: u32,
    /// Destination Socket ID (§3).
    pub dest_socket_id: u32,
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
    pub message_number: u32,
    /// Timestamp (§3).
    pub timestamp: u32,
    /// Destination Socket ID (§3).
    pub dest_socket_id: u32,
    /// First Packet Sequence Number of the range to drop.
    pub first_seq: u32,
    /// Last Packet Sequence Number of the range to drop.
    pub last_seq: u32,
}

impl DropReqPacket {
    pub(crate) fn parse_cif(
        message_number: u32,
        timestamp: u32,
        dest_socket_id: u32,
        cif: &[u8],
    ) -> Result<Self> {
        if cif.len() != 8 {
            return Err(Error::BufferTooShort {
                need: 8,
                have: cif.len(),
                what: "drop request CIF",
            });
        }
        Ok(DropReqPacket {
            message_number,
            timestamp,
            dest_socket_id,
            first_seq: be32(cif, 0),
            last_seq: be32(cif, 4),
        })
    }

    pub(crate) fn cif_len(&self) -> usize {
        8
    }

    pub(crate) fn write_cif(&self, buf: &mut [u8]) {
        put_be32(buf, 0, self.first_seq);
        put_be32(buf, 4, self.last_seq);
    }
}

/// Peer Error control packet (§3.2.10, Figure 19). No CIF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PeerErrorPacket {
    /// Peer error code (see [`PEER_ERROR_FILE_SYSTEM`]).
    pub error_code: u32,
    /// Timestamp (§3).
    pub timestamp: u32,
    /// Destination Socket ID (§3).
    pub dest_socket_id: u32,
    /// `true` when the CIF carries libsrt's 4-byte zero pad; `false` for the
    /// empty CIF of draft-sharabayko-srt-01.
    pub libsrt_pad: bool,
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::super::control::ControlPacket;
    use super::*;

    #[test]
    fn drop_req_round_trips() {
        let d = DropReqPacket {
            message_number: 7,
            timestamp: 100,
            dest_socket_id: 200,
            first_seq: 10,
            last_seq: 20,
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

    #[test]
    fn keepalive_congestion_shutdown_ackack_peererror_round_trip() {
        let cases: Vec<ControlPacket> = alloc::vec![
            ControlPacket::KeepAlive(KeepAlivePacket {
                timestamp: 1,
                dest_socket_id: 2,
                libsrt_pad: true,
            }),
            ControlPacket::CongestionWarning(CongestionWarningPacket {
                timestamp: 3,
                dest_socket_id: 4,
                libsrt_pad: true,
            }),
            ControlPacket::Shutdown(ShutdownPacket {
                timestamp: 5,
                dest_socket_id: 6,
                libsrt_pad: true,
            }),
            ControlPacket::AckAck(AckAckPacket {
                ack_number: 9,
                timestamp: 7,
                dest_socket_id: 8,
                libsrt_pad: true,
            }),
            ControlPacket::PeerError(PeerErrorPacket {
                error_code: PEER_ERROR_FILE_SYSTEM,
                timestamp: 11,
                dest_socket_id: 12,
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

    /// A peer (e.g. real libsrt — see `tests/libsrt_fixtures.rs`) may also
    /// send these types with the pure-spec empty CIF instead of the pad.
    /// Both 16-byte (no-pad) and 20-byte (with-pad) wire shapes parse correctly.
    #[test]
    fn keepalive_congestion_shutdown_ackack_peererror_accept_empty_cif() {
        let cases: Vec<ControlPacket> = alloc::vec![
            ControlPacket::KeepAlive(KeepAlivePacket {
                timestamp: 1,
                dest_socket_id: 2,
                libsrt_pad: false,
            }),
            ControlPacket::CongestionWarning(CongestionWarningPacket {
                timestamp: 3,
                dest_socket_id: 4,
                libsrt_pad: false,
            }),
            ControlPacket::Shutdown(ShutdownPacket {
                timestamp: 5,
                dest_socket_id: 6,
                libsrt_pad: false,
            }),
            ControlPacket::AckAck(AckAckPacket {
                ack_number: 9,
                timestamp: 7,
                dest_socket_id: 8,
                libsrt_pad: false,
            }),
            ControlPacket::PeerError(PeerErrorPacket {
                error_code: PEER_ERROR_FILE_SYSTEM,
                timestamp: 11,
                dest_socket_id: 12,
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
            timestamp: 0x1234,
            dest_socket_id: 0x5678,
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
            timestamp: 0x1234,
            dest_socket_id: 0x5678,
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
            Err(Error::ReservedFieldNotZero { .. })
        ));
    }
}
