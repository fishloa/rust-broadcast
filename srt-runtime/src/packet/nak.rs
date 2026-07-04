//! NAK (Negative Acknowledgement / Loss Report) control packet —
//! `draft-sharabayko-srt-01` §3.2.5, Figure 14, and the loss-list coding of
//! Appendix A.
//!
//! The CIF is a sequence of 31-bit sequence-number entries: a single lost
//! packet (top bit clear), or a range `[a, b]` encoded as two consecutive
//! entries — `a` with its top bit set, `b` with its top bit clear
//! (Appendix A, Figures 21/22).
//!
//! [`NakPacket::raw_loss_list`] is kept as a borrowed byte slice (the same
//! lazy-loop convention `dvb-si` uses for descriptor loops) rather than
//! eagerly decoded into an owned list — walk it with
//! [`NakPacket::entries`].

use alloc::vec::Vec;

use super::{Error, F_BIT, Result, SEQ_NUMBER_MASK, be32};

/// One decoded entry of a NAK loss list (Appendix A). Data-carrying ADT — not
/// a spec/field label, so it is exempt from the `name()`/`Display`
/// convention (see `tests/label_coverage.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum LossListEntry {
    /// A single lost packet sequence number (Figure 21).
    Single(u32),
    /// An inclusive range of lost packet sequence numbers `[first, last]`
    /// (Figure 22).
    Range(u32, u32),
}

/// NAK control packet (§3.2.5, Figure 14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NakPacket<'a> {
    /// Timestamp (§3).
    pub timestamp: u32,
    /// Destination Socket ID (§3).
    pub dest_socket_id: u32,
    /// The raw loss-list CIF bytes. Walk with [`Self::entries`].
    pub raw_loss_list: &'a [u8],
}

impl<'a> NakPacket<'a> {
    pub(crate) fn parse_cif(timestamp: u32, dest_socket_id: u32, cif: &'a [u8]) -> Self {
        NakPacket {
            timestamp,
            dest_socket_id,
            raw_loss_list: cif,
        }
    }

    pub(crate) fn cif_len(&self) -> usize {
        self.raw_loss_list.len()
    }

    pub(crate) fn write_cif(&self, buf: &mut [u8]) {
        buf.copy_from_slice(self.raw_loss_list);
    }

    /// Iterate the decoded loss-list entries (Appendix A). Yields an
    /// [`Error`] (never panics) on a malformed entry, then stops.
    pub fn entries(&self) -> LossListIter<'a> {
        LossListIter {
            rest: self.raw_loss_list,
        }
    }
}

/// Iterator over a NAK loss list's decoded entries. See [`NakPacket::entries`].
#[derive(Debug, Clone)]
pub struct LossListIter<'a> {
    rest: &'a [u8],
}

impl Iterator for LossListIter<'_> {
    type Item = Result<LossListEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() {
            return None;
        }
        if self.rest.len() < 4 {
            self.rest = &[];
            return Some(Err(Error::InvalidLossList {
                reason: "trailing bytes are not a whole 4-byte entry",
            }));
        }
        let w0 = be32(self.rest, 0);
        self.rest = &self.rest[4..];
        if w0 & F_BIT != 0 {
            let first = w0 & SEQ_NUMBER_MASK;
            if self.rest.len() < 4 {
                self.rest = &[];
                return Some(Err(Error::InvalidLossList {
                    reason: "range entry missing its end sequence number",
                }));
            }
            let w1 = be32(self.rest, 0);
            self.rest = &self.rest[4..];
            if w1 & F_BIT != 0 {
                return Some(Err(Error::InvalidLossList {
                    reason: "range end entry had its top bit set",
                }));
            }
            Some(Ok(LossListEntry::Range(first, w1 & SEQ_NUMBER_MASK)))
        } else {
            Some(Ok(LossListEntry::Single(w0 & SEQ_NUMBER_MASK)))
        }
    }
}

/// Build the raw loss-list bytes for a NAK CIF from decoded entries
/// (Appendix A). Inverse of [`NakPacket::entries`].
///
/// # Errors
/// [`Error::FieldTooWide`] if a sequence number does not fit in 31 bits.
pub fn build_loss_list(entries: &[LossListEntry]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(entries.len() * 4);
    let check = |seq: u32| -> Result<()> {
        if seq > SEQ_NUMBER_MASK {
            return Err(Error::FieldTooWide {
                what: "loss list sequence number",
                value: u64::from(seq),
                bits: 31,
            });
        }
        Ok(())
    };
    for entry in entries {
        match *entry {
            LossListEntry::Single(seq) => {
                check(seq)?;
                out.extend_from_slice(&seq.to_be_bytes());
            }
            LossListEntry::Range(first, last) => {
                check(first)?;
                check(last)?;
                out.extend_from_slice(&(first | F_BIT).to_be_bytes());
                out.extend_from_slice(&last.to_be_bytes());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::control::ControlPacket;
    use super::*;

    #[test]
    fn single_and_range_entries_round_trip_hand_computed_bytes() {
        let entries = [
            LossListEntry::Single(5),
            LossListEntry::Range(10, 20),
            LossListEntry::Single(30),
        ];
        let raw = build_loss_list(&entries).unwrap();
        assert_eq!(raw.len(), 16);
        assert_eq!(&raw[0..4], &5u32.to_be_bytes());
        assert_eq!(&raw[4..8], &(10u32 | 0x8000_0000).to_be_bytes());
        assert_eq!(&raw[8..12], &20u32.to_be_bytes());
        assert_eq!(&raw[12..16], &30u32.to_be_bytes());

        let pkt = ControlPacket::Nak(NakPacket {
            timestamp: 1,
            dest_socket_id: 2,
            raw_loss_list: &raw,
        });
        let mut buf = alloc::vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut buf).unwrap();
        let parsed = ControlPacket::parse(&buf).unwrap();
        assert_eq!(parsed, pkt);

        if let ControlPacket::Nak(n) = parsed {
            let decoded: Vec<LossListEntry> = n.entries().map(|e| e.unwrap()).collect();
            assert_eq!(&decoded, &entries);
        } else {
            panic!("expected NAK");
        }
    }

    #[test]
    fn malformed_range_end_top_bit_errs_without_panic() {
        // Range start (top bit set) followed by another range-start-looking
        // word (top bit also set) — invalid per Appendix A.
        let mut raw = Vec::new();
        raw.extend_from_slice(&(1u32 | 0x8000_0000).to_be_bytes());
        raw.extend_from_slice(&(2u32 | 0x8000_0000).to_be_bytes());
        let n = NakPacket {
            timestamp: 0,
            dest_socket_id: 0,
            raw_loss_list: &raw,
        };
        let mut it = n.entries();
        assert!(matches!(
            it.next(),
            Some(Err(Error::InvalidLossList { .. }))
        ));
        assert!(it.next().is_none());
    }

    #[test]
    fn overwide_sequence_number_errs() {
        assert!(matches!(
            build_loss_list(&[LossListEntry::Single(0x8000_0000)]),
            Err(Error::FieldTooWide { .. })
        ));
    }
}
