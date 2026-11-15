//! NAK (Negative Acknowledgement / Loss Report) control packet —
//! `draft-sharabayko-srt-01` §3.2.5, Figure 14, and the loss-list coding of
//! Appendix A.
//!
//! The CIF is a sequence of 31-bit sequence-number entries: a single lost
//! packet (top bit clear), or a range `[a, b]` encoded as two consecutive
//! entries — `a` with its top bit set, `b` with its top bit clear
//! (Appendix A, Figures 21/22).
//!
//! [`NakPacket::entries`] is the typed loss list: a hand-built NAK is checked
//! the moment it is constructed (every endpoint is a [`SeqNo`] that already
//! fits the 31-bit field), rather than only when it is serialised. Parse
//! decodes the wire bytes into that list once, at the edge.

use alloc::vec::Vec;

use super::number::SeqNo;
use super::{Error, F_BIT, Result, be32, put_be32};

/// One decoded entry of a NAK loss list (Appendix A). Data-carrying ADT — not
/// a spec/field label, so it is exempt from the `name()`/`Display`
/// convention (see `workspace-guards/tests/guards/label.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum LossListEntry {
    /// A single lost packet sequence number (Figure 21).
    Single(SeqNo),
    /// An inclusive range of lost packet sequence numbers `[first, last]`
    /// (Figure 22).
    Range(SeqNo, SeqNo),
}

/// NAK control packet (§3.2.5, Figure 14).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NakPacket {
    /// Timestamp (§3).
    pub timestamp: u32,
    /// Destination Socket ID (§3).
    pub dest_socket_id: u32,
    /// The decoded loss-list entries (Appendix A), serialized on write.
    pub entries: Vec<LossListEntry>,
}

impl NakPacket {
    /// Parse the loss-list Control Information Field (Appendix A).
    ///
    /// # Errors
    /// [`Error::InvalidLossList`] when the bytes are not a sequence of whole
    /// entries, when a range start is not followed by its end, or when that
    /// end has the range marker bit set.
    pub fn parse_cif(timestamp: u32, dest_socket_id: u32, cif: &[u8]) -> Result<Self> {
        let mut entries = Vec::with_capacity(cif.len() / 4);
        let mut rest = cif;
        while !rest.is_empty() {
            if rest.len() < 4 {
                return Err(Error::InvalidLossList {
                    reason: "trailing bytes are not a whole 4-byte entry",
                });
            }
            let w0 = be32(rest, 0);
            rest = &rest[4..];
            if w0 & F_BIT != 0 {
                let first = SeqNo::from_wire_masked(w0);
                if rest.len() < 4 {
                    return Err(Error::InvalidLossList {
                        reason: "range entry missing its end sequence number",
                    });
                }
                let w1 = be32(rest, 0);
                rest = &rest[4..];
                if w1 & F_BIT != 0 {
                    return Err(Error::InvalidLossList {
                        reason: "range end entry had its top bit set",
                    });
                }
                entries.push(LossListEntry::Range(first, SeqNo::from_wire_masked(w1)));
            } else {
                entries.push(LossListEntry::Single(SeqNo::from_wire_masked(w0)));
            }
        }
        Ok(NakPacket {
            timestamp,
            dest_socket_id,
            entries,
        })
    }

    /// Number of CIF bytes the entries encode to: one 32-bit word for a
    /// single, two for a range (Appendix A).
    pub(crate) fn cif_len(&self) -> usize {
        self.entries
            .iter()
            .map(|e| match e {
                LossListEntry::Single(_) => 4,
                LossListEntry::Range(..) => 8,
            })
            .sum()
    }

    pub(crate) fn write_cif(&self, buf: &mut [u8]) {
        let mut off = 0;
        for entry in &self.entries {
            match *entry {
                LossListEntry::Single(seq) => {
                    put_be32(buf, off, seq.value());
                    off += 4;
                }
                LossListEntry::Range(first, last) => {
                    put_be32(buf, off, first.value() | F_BIT);
                    put_be32(buf, off + 4, last.value());
                    off += 8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::SRT_HEADER_LEN;
    use super::super::control::{CONTROL_TYPE_NAK, ControlPacket};
    use super::*;

    fn sn(v: u32) -> SeqNo {
        SeqNo::try_new(v).expect("fits 31 bits")
    }

    #[test]
    fn single_and_range_entries_round_trip_hand_computed_bytes() {
        let entries = alloc::vec![
            LossListEntry::Single(sn(5)),
            LossListEntry::Range(sn(10), sn(20)),
            LossListEntry::Single(sn(30)),
        ];
        let pkt = ControlPacket::Nak(NakPacket {
            timestamp: 1,
            dest_socket_id: 2,
            entries: entries.clone(),
        });
        let mut buf = alloc::vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut buf).unwrap();
        let cif = &buf[super::super::SRT_HEADER_LEN..];
        assert_eq!(cif.len(), 16);
        assert_eq!(&cif[0..4], &5u32.to_be_bytes());
        assert_eq!(&cif[4..8], &(10u32 | F_BIT).to_be_bytes());
        assert_eq!(&cif[8..12], &20u32.to_be_bytes());
        assert_eq!(&cif[12..16], &30u32.to_be_bytes());

        let parsed = ControlPacket::parse(&buf).unwrap();
        assert_eq!(parsed, pkt);
        if let ControlPacket::Nak(n) = parsed {
            assert_eq!(n.entries, entries);
        } else {
            panic!("expected NAK");
        }
    }

    #[test]
    fn malformed_range_end_top_bit_errs_without_panic() {
        // Range start (top bit set) followed by another range-start-looking
        // word (top bit also set) — invalid per Appendix A.
        let mut raw = Vec::new();
        raw.extend_from_slice(&(1u32 | F_BIT).to_be_bytes());
        raw.extend_from_slice(&(2u32 | F_BIT).to_be_bytes());
        let err = NakPacket::parse_cif(0, 0, &raw)
            .expect_err("a range end with the top bit set is malformed");
        assert!(matches!(err, Error::InvalidLossList { .. }));
    }

    #[test]
    fn a_truncated_loss_list_is_rejected_on_parse() {
        // Three bytes are not a whole 4-byte entry.
        let err = NakPacket::parse_cif(0, 0, &[0, 0, 0])
            .expect_err("trailing bytes are not a whole entry");
        assert!(matches!(err, Error::InvalidLossList { .. }));
    }

    /// The malformed list is rejected by the packet dispatcher, not merely by
    /// a loss-list walker a caller may never run: the old borrowed-list shape
    /// parsed this control packet successfully and only failed when iterated.
    #[test]
    fn the_dispatcher_rejects_a_malformed_loss_list() {
        let mut raw = alloc::vec![0u8; SRT_HEADER_LEN];
        put_be32(&mut raw, 0, u32::from(CONTROL_TYPE_NAK) << 16 | F_BIT);
        raw.extend_from_slice(&(1u32 | F_BIT).to_be_bytes());
        raw.extend_from_slice(&(2u32 | F_BIT).to_be_bytes());
        assert!(matches!(
            ControlPacket::parse(&raw),
            Err(Error::InvalidLossList { .. })
        ));
    }

    #[test]
    fn a_range_missing_its_end_is_rejected_on_parse() {
        let raw = (1u32 | F_BIT).to_be_bytes();
        let err = NakPacket::parse_cif(0, 0, &raw)
            .expect_err("a range start without its end is malformed");
        assert!(matches!(err, Error::InvalidLossList { .. }));
    }
}
