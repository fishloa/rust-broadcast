//! ACK (Acknowledgment) control packet — `draft-sharabayko-srt-01` §3.2.4,
//! Figure 13.
//!
//! Three variants share the same header shape and differ only in how much of
//! the CIF is present, distinguished by CIF length on the wire:
//!
//! - **Full** (28-byte CIF): all seven fields, sent every 10 ms.
//! - **Small** (16-byte CIF): fields up to and including Available Buffer
//!   Size.
//! - **Light** (4-byte CIF): only Last Acknowledged Packet Sequence Number.

use super::{Error, Result, be32, put_be32};

/// CIF length, in bytes, of a Full ACK (§3.2.4).
pub const ACK_CIF_LEN_FULL: usize = 28;
/// CIF length, in bytes, of a Small ACK (§3.2.4).
pub const ACK_CIF_LEN_SMALL: usize = 16;
/// CIF length, in bytes, of a Light ACK (§3.2.4).
pub const ACK_CIF_LEN_LIGHT: usize = 4;

/// The ACK Control Information Field — its shape (Full/Small/Light) is
/// selected by which fields are present on the wire (§3.2.4). Data-carrying
/// ADT: see [`AckPacket`] for the label convention rationale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AckCif {
    /// Full ACK — all seven CIF fields.
    Full {
        /// Last Acknowledged Packet Sequence Number.
        last_ack_seq: u32,
        /// RTT estimate, in microseconds.
        rtt_us: u32,
        /// RTT variance, in microseconds.
        rtt_var_us: u32,
        /// Available receiver buffer size, in packets.
        avail_buf_size: u32,
        /// Packets receiving rate, in packets per second.
        pkt_recv_rate: u32,
        /// Estimated link capacity, in packets per second.
        est_link_capacity: u32,
        /// Estimated receiving rate, in bytes per second.
        recv_rate_bps: u32,
    },
    /// Small ACK — fields up to and including Available Buffer Size.
    Small {
        /// Last Acknowledged Packet Sequence Number.
        last_ack_seq: u32,
        /// RTT estimate, in microseconds.
        rtt_us: u32,
        /// RTT variance, in microseconds.
        rtt_var_us: u32,
        /// Available receiver buffer size, in packets.
        avail_buf_size: u32,
    },
    /// Light ACK — only Last Acknowledged Packet Sequence Number.
    Light {
        /// Last Acknowledged Packet Sequence Number.
        last_ack_seq: u32,
    },
}

/// ACK control packet (§3.2.4, Figure 13). `ack_number` occupies the header
/// `Type-specific Information` word ("the sequential number of the full
/// acknowledgment packet starting from 1"); per §3.2.4 it "should be set to
/// 0" for Small/Light ACKs, but is stored and round-tripped verbatim
/// regardless of variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AckPacket {
    /// Acknowledgement Number.
    pub ack_number: u32,
    /// Timestamp (§3).
    pub timestamp: u32,
    /// Destination Socket ID (§3).
    pub dest_socket_id: u32,
    /// The CIF, shaped per variant.
    pub cif: AckCif,
}

impl AckPacket {
    pub(crate) fn parse_cif(
        ack_number: u32,
        timestamp: u32,
        dest_socket_id: u32,
        cif: &[u8],
    ) -> Result<Self> {
        let parsed = match cif.len() {
            ACK_CIF_LEN_FULL => AckCif::Full {
                last_ack_seq: be32(cif, 0),
                rtt_us: be32(cif, 4),
                rtt_var_us: be32(cif, 8),
                avail_buf_size: be32(cif, 12),
                pkt_recv_rate: be32(cif, 16),
                est_link_capacity: be32(cif, 20),
                recv_rate_bps: be32(cif, 24),
            },
            ACK_CIF_LEN_SMALL => AckCif::Small {
                last_ack_seq: be32(cif, 0),
                rtt_us: be32(cif, 4),
                rtt_var_us: be32(cif, 8),
                avail_buf_size: be32(cif, 12),
            },
            ACK_CIF_LEN_LIGHT => AckCif::Light {
                last_ack_seq: be32(cif, 0),
            },
            other => return Err(Error::InvalidAckLength { len: other }),
        };
        Ok(AckPacket {
            ack_number,
            timestamp,
            dest_socket_id,
            cif: parsed,
        })
    }

    pub(crate) fn cif_len(&self) -> usize {
        match self.cif {
            AckCif::Full { .. } => ACK_CIF_LEN_FULL,
            AckCif::Small { .. } => ACK_CIF_LEN_SMALL,
            AckCif::Light { .. } => ACK_CIF_LEN_LIGHT,
        }
    }

    pub(crate) fn write_cif(&self, buf: &mut [u8]) -> Result<()> {
        match self.cif {
            AckCif::Full {
                last_ack_seq,
                rtt_us,
                rtt_var_us,
                avail_buf_size,
                pkt_recv_rate,
                est_link_capacity,
                recv_rate_bps,
            } => {
                put_be32(buf, 0, last_ack_seq);
                put_be32(buf, 4, rtt_us);
                put_be32(buf, 8, rtt_var_us);
                put_be32(buf, 12, avail_buf_size);
                put_be32(buf, 16, pkt_recv_rate);
                put_be32(buf, 20, est_link_capacity);
                put_be32(buf, 24, recv_rate_bps);
            }
            AckCif::Small {
                last_ack_seq,
                rtt_us,
                rtt_var_us,
                avail_buf_size,
            } => {
                put_be32(buf, 0, last_ack_seq);
                put_be32(buf, 4, rtt_us);
                put_be32(buf, 8, rtt_var_us);
                put_be32(buf, 12, avail_buf_size);
            }
            AckCif::Light { last_ack_seq } => {
                put_be32(buf, 0, last_ack_seq);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::control::ControlPacket;
    use super::*;

    #[test]
    fn full_ack_round_trips_hand_computed_bytes() {
        let pkt = ControlPacket::Ack(AckPacket {
            ack_number: 42,
            timestamp: 1000,
            dest_socket_id: 2000,
            cif: AckCif::Full {
                last_ack_seq: 1,
                rtt_us: 2,
                rtt_var_us: 3,
                avail_buf_size: 4,
                pkt_recv_rate: 5,
                est_link_capacity: 6,
                recv_rate_bps: 7,
            },
        });
        let mut buf = [0u8; 16 + 28];
        let n = pkt.serialize_into(&mut buf).unwrap();
        assert_eq!(n, 44);
        assert_eq!(buf[0], 0x80); // F=1
        assert_eq!(&buf[0..4], &(0x8002_0000u32).to_be_bytes()); // control type=2
        assert_eq!(&buf[4..8], &42u32.to_be_bytes());
        assert_eq!(&buf[16..20], &1u32.to_be_bytes());
        assert_eq!(&buf[40..44], &7u32.to_be_bytes());
        assert_eq!(ControlPacket::parse(&buf).unwrap(), pkt);
    }

    #[test]
    fn small_and_light_ack_round_trip() {
        for cif in [
            AckCif::Small {
                last_ack_seq: 10,
                rtt_us: 20,
                rtt_var_us: 30,
                avail_buf_size: 40,
            },
            AckCif::Light { last_ack_seq: 99 },
        ] {
            let pkt = ControlPacket::Ack(AckPacket {
                ack_number: 0,
                timestamp: 1,
                dest_socket_id: 2,
                cif,
            });
            let mut buf = alloc::vec![0u8; pkt.serialized_len()];
            pkt.serialize_into(&mut buf).unwrap();
            assert_eq!(ControlPacket::parse(&buf).unwrap(), pkt);
        }
    }

    #[test]
    fn invalid_ack_cif_length_errs() {
        let mut buf = [0u8; 16 + 5];
        buf[0] = 0x80;
        buf[1] = 0x02; // ACK
        assert_eq!(
            ControlPacket::parse(&buf).unwrap_err(),
            Error::InvalidAckLength { len: 5 }
        );
    }

    #[test]
    fn mutate_field_changes_bytes() {
        let mut pkt = AckPacket {
            ack_number: 1,
            timestamp: 2,
            dest_socket_id: 3,
            cif: AckCif::Light { last_ack_seq: 4 },
        };
        let mut buf1 = [0u8; 20];
        ControlPacket::Ack(pkt).serialize_into(&mut buf1).unwrap();
        pkt.ack_number = 99;
        let mut buf2 = [0u8; 20];
        ControlPacket::Ack(pkt).serialize_into(&mut buf2).unwrap();
        assert_ne!(buf1, buf2);
    }
}
