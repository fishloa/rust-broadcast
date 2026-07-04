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
}

/// Shutdown control packet (§3.2.7, Figure 16). No CIF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ShutdownPacket {
    /// Timestamp (§3).
    pub timestamp: u32,
    /// Destination Socket ID (§3).
    pub dest_socket_id: u32,
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
                dest_socket_id: 2
            }),
            ControlPacket::CongestionWarning(CongestionWarningPacket {
                timestamp: 3,
                dest_socket_id: 4
            }),
            ControlPacket::Shutdown(ShutdownPacket {
                timestamp: 5,
                dest_socket_id: 6
            }),
            ControlPacket::AckAck(AckAckPacket {
                ack_number: 9,
                timestamp: 7,
                dest_socket_id: 8
            }),
            ControlPacket::PeerError(PeerErrorPacket {
                error_code: PEER_ERROR_FILE_SYSTEM,
                timestamp: 11,
                dest_socket_id: 12
            }),
        ];
        for pkt in cases {
            let mut buf = [0u8; 16];
            let n = pkt.serialize_into(&mut buf).unwrap();
            assert_eq!(n, 16);
            let parsed = ControlPacket::parse(&buf).unwrap();
            assert_eq!(parsed, pkt);
        }
    }

    #[test]
    fn keepalive_rejects_trailing_bytes() {
        let mut buf = [0u8; 17];
        buf[0] = 0x80; // F=1
        buf[1] = 0x01; // control type = 1 (KEEPALIVE)
        assert!(matches!(
            ControlPacket::parse(&buf),
            Err(Error::UnexpectedTrailingBytes { .. })
        ));
    }
}
