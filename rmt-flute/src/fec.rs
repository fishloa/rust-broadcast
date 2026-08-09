//! FEC Building Block — Block Partitioning Algorithm (RFC 5052 §9.1).
//!
//! RFC 5052 §9 ("FEC Schemes and CDPs SHOULD use these algorithms in
//! preference to scheme- or protocol-specific algorithms, where appropriate")
//! defines exactly one concrete *algorithm*: how to split an `L`-octet
//! transport object into `N` source blocks of as-equal-as-possible length,
//! given a maximum source block length `B` (symbols) and an encoding symbol
//! length `E` (octets). [`SourceBlockPartition`] is that algorithm — see
//! `docs/fec.md` §5/§9 for the full transcription and for why this module
//! stops there rather than reproducing any FEC-scheme-specific FEC Payload ID
//! (`docs/fec.md` §3/§8) or Scheme-specific FEC OTI layout (§2.3/§8). Those
//! stay opaque byte slices the caller supplies — exactly like
//! [`crate::AlcPacket::fec_payload_id`] and [`crate::FecPayloadId128`] already
//! do — because their bit layout is FEC-scheme dependent, not defined by
//! ALC/FLUTE/NORM or by this crate.
//!
//! This is deliberately the *only* thing this module does: it operates
//! entirely on symbol **counts**, never on FEC-scheme-specific bytes, so it is
//! equally usable by a Compact-No-Code, Raptor, RaptorQ, or any future FEC
//! scheme's consumer — `dvb-mabr` and `atsc3-route` both need exactly this
//! shape (issue #944).

use crate::error::{Error, Result};

/// The source-block structure of a transport object, per RFC 5052 §9.1's
/// Block Partitioning Algorithm.
///
/// Given a transport object of `transfer_length` octets (`L`), an
/// `encoding_symbol_length` (`E`) and a `max_source_block_length` (`B`), RFC
/// 5052 splits the object into `num_blocks` (`N`) source blocks: the first
/// `larger_blocks` (`I`) blocks each have `larger_block_len` (`A_large`)
/// source symbols, and the remaining `num_blocks - larger_blocks` blocks each
/// have `smaller_block_len` (`A_small`) source symbols. Every source symbol is
/// `E` octets, **except** the very last source symbol of the very last source
/// block ([`Self::last_symbol_len`]).
///
/// All three inputs are RFC 5052 Common FEC OTI elements (§6.2.4) — this type
/// does not read wire bytes; the caller supplies them from wherever the CDP
/// carried them (ALC's `EXT_FTI`, a FLUTE FDT attribute, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SourceBlockPartition {
    /// The transport object length in octets (`L`, Transfer-Length).
    pub transfer_length: u64,
    /// The encoding symbol length in octets (`E`, Encoding-Symbol-Length).
    pub encoding_symbol_length: u32,
    /// Total source symbols in the object (`T = ceil(L / E)`).
    pub source_symbols: u64,
    /// Number of source blocks (`N = ceil(T / B)`).
    pub num_blocks: u64,
    /// Source symbols in each of the first `larger_blocks` blocks
    /// (`A_large = ceil(T / N)`).
    pub larger_block_len: u64,
    /// Source symbols in each of the remaining blocks
    /// (`A_small = floor(T / N)`).
    pub smaller_block_len: u64,
    /// Number of "larger" blocks (`I = T - A_small * N`).
    pub larger_blocks: u64,
}

impl SourceBlockPartition {
    /// Apply RFC 5052 §9.1's Block Partitioning Algorithm.
    ///
    /// `transfer_length` is the transport object length in octets (`L`),
    /// `encoding_symbol_length` is `E`, and `max_source_block_length` is `B`.
    ///
    /// A zero-length object (`transfer_length == 0`) yields zero source
    /// symbols and zero source blocks — RFC 5052's ceiling division treats
    /// `ceil(0/E)` as `0`, not `1`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidField`] if `encoding_symbol_length` or
    /// `max_source_block_length` is `0` (both are divisors in the algorithm;
    /// RFC 5052 gives no meaning to either being zero).
    pub fn new(
        transfer_length: u64,
        encoding_symbol_length: u32,
        max_source_block_length: u32,
    ) -> Result<Self> {
        if encoding_symbol_length == 0 {
            return Err(Error::InvalidField {
                what: "Encoding-Symbol-Length",
                reason: "must be non-zero",
            });
        }
        if max_source_block_length == 0 {
            return Err(Error::InvalidField {
                what: "Maximum-Source-Block-Length",
                reason: "must be non-zero",
            });
        }
        let e = encoding_symbol_length as u64;
        let b = max_source_block_length as u64;

        // First step (§9.1.1): T = ceil(L/E); N = ceil(T/B). `div_ceil` is
        // division-based (not addition-based), so it cannot overflow here.
        let source_symbols = transfer_length.div_ceil(e);
        let num_blocks = if source_symbols == 0 {
            0
        } else {
            source_symbols.div_ceil(b)
        };

        // Second step (§9.1.2): A_large, A_small, I.
        let (larger_block_len, smaller_block_len, larger_blocks) = if num_blocks == 0 {
            (0, 0, 0)
        } else {
            let a_large = source_symbols.div_ceil(num_blocks);
            let a_small = source_symbols / num_blocks;
            let i = source_symbols - a_small * num_blocks;
            (a_large, a_small, i)
        };

        Ok(SourceBlockPartition {
            transfer_length,
            encoding_symbol_length,
            source_symbols,
            num_blocks,
            larger_block_len,
            smaller_block_len,
            larger_blocks,
        })
    }

    /// Number of source symbols in source block `index` (0-based), or `None`
    /// if `index >= num_blocks`.
    ///
    /// The first `larger_blocks` blocks (indices `0..larger_blocks`) each have
    /// `larger_block_len` symbols; the remaining blocks have
    /// `smaller_block_len`.
    pub fn block_len(&self, index: u64) -> Option<u64> {
        if index < self.larger_blocks {
            Some(self.larger_block_len)
        } else if index < self.num_blocks {
            Some(self.smaller_block_len)
        } else {
            None
        }
    }

    /// Length in octets of the very last source symbol of the very last
    /// source block (RFC 5052 §9.1: `L - floor((L-1)/E)*E`) — the object's
    /// actual trailing remainder, since `L` is not generally an exact
    /// multiple of `E`. `None` if the object has zero source symbols (there
    /// is no "last symbol" to speak of).
    pub fn last_symbol_len(&self) -> Option<u32> {
        if self.source_symbols == 0 {
            return None;
        }
        let l = self.transfer_length;
        let e = self.encoding_symbol_length as u64;
        let len = l - ((l - 1) / e) * e;
        Some(len as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[test]
    fn rejects_zero_divisors() {
        assert!(matches!(
            SourceBlockPartition::new(100, 0, 10),
            Err(Error::InvalidField {
                what: "Encoding-Symbol-Length",
                ..
            })
        ));
        assert!(matches!(
            SourceBlockPartition::new(100, 10, 0),
            Err(Error::InvalidField {
                what: "Maximum-Source-Block-Length",
                ..
            })
        ));
    }

    #[test]
    fn zero_length_object_has_no_symbols_or_blocks() {
        // L=0: T = ceil(0/E) = 0, N = ceil(0/B) = 0 (RFC 5052 §9.1.1).
        let p = SourceBlockPartition::new(0, 1000, 10).unwrap();
        assert_eq!(p.source_symbols, 0);
        assert_eq!(p.num_blocks, 0);
        assert_eq!(p.larger_block_len, 0);
        assert_eq!(p.smaller_block_len, 0);
        assert_eq!(p.larger_blocks, 0);
        assert_eq!(p.block_len(0), None);
        assert_eq!(p.last_symbol_len(), None);
    }

    // Exact-multiple worked example, hand-derived from RFC 5052 §9.1's own
    // formulas (the RFC gives no numeric worked example itself — verified
    // against the IETF-published text of RFC 5052 §9.1):
    //   L=10000, E=1000, B=3
    //   T = ceil(10000/1000) = 10
    //   N = ceil(10/3) = 4
    //   A_large = ceil(10/4) = 3, A_small = floor(10/4) = 2, I = 10-2*4 = 2
    // so blocks 0,1 have 3 symbols; blocks 2,3 have 2 symbols (3+3+2+2=10).
    #[test]
    fn exact_multiple_worked_example() {
        let p = SourceBlockPartition::new(10_000, 1000, 3).unwrap();
        assert_eq!(p.source_symbols, 10);
        assert_eq!(p.num_blocks, 4);
        assert_eq!(p.larger_block_len, 3);
        assert_eq!(p.smaller_block_len, 2);
        assert_eq!(p.larger_blocks, 2);

        let lens: Vec<u64> = (0..p.num_blocks).map(|i| p.block_len(i).unwrap()).collect();
        assert_eq!(lens, [3, 3, 2, 2]);
        assert_eq!(lens.iter().sum::<u64>(), p.source_symbols);
        assert_eq!(p.block_len(p.num_blocks), None);

        // L is an exact multiple of E, so the trailing remainder is a full
        // symbol.
        assert_eq!(p.last_symbol_len(), Some(1000));
    }

    // Non-exact-multiple worked example (trailing remainder exercised):
    //   L=10005, E=1000, B=3
    //   T = ceil(10005/1000) = 11
    //   N = ceil(11/3) = 4
    //   A_large = ceil(11/4) = 3, A_small = floor(11/4) = 2, I = 11-2*4 = 3
    // so blocks 0,1,2 have 3 symbols; block 3 has 2 symbols (3+3+3+2=11).
    #[test]
    fn non_exact_multiple_worked_example() {
        let p = SourceBlockPartition::new(10_005, 1000, 3).unwrap();
        assert_eq!(p.source_symbols, 11);
        assert_eq!(p.num_blocks, 4);
        assert_eq!(p.larger_block_len, 3);
        assert_eq!(p.smaller_block_len, 2);
        assert_eq!(p.larger_blocks, 3);

        let lens: Vec<u64> = (0..p.num_blocks).map(|i| p.block_len(i).unwrap()).collect();
        assert_eq!(lens, [3, 3, 3, 2]);
        assert_eq!(lens.iter().sum::<u64>(), p.source_symbols);

        // 10005 = 10*1000 + 5, so the trailing remainder is 5 octets.
        assert_eq!(p.last_symbol_len(), Some(5));
    }

    // B larger than T collapses to a single source block (N=1), and A_large
    // == A_small == T in that case (I = T - T*1 = 0).
    #[test]
    fn max_block_length_exceeding_total_symbols_yields_one_block() {
        let p = SourceBlockPartition::new(100, 10, 1000).unwrap();
        assert_eq!(p.source_symbols, 10);
        assert_eq!(p.num_blocks, 1);
        assert_eq!(p.larger_block_len, 10);
        assert_eq!(p.smaller_block_len, 10);
        assert_eq!(p.larger_blocks, 0);
        assert_eq!(p.block_len(0), Some(10));
        assert_eq!(p.block_len(1), None);
    }

    // A single-symbol object smaller than E: T=1 regardless of how small L is.
    #[test]
    fn sub_symbol_object_rounds_up_to_one_symbol() {
        let p = SourceBlockPartition::new(5, 1000, 10).unwrap();
        assert_eq!(p.source_symbols, 1);
        assert_eq!(p.num_blocks, 1);
        // The one and only symbol IS the last symbol: full L, not full E.
        assert_eq!(p.last_symbol_len(), Some(5));
    }

    #[test]
    fn mutating_transfer_length_changes_partition() {
        let a = SourceBlockPartition::new(10_000, 1000, 3).unwrap();
        let b = SourceBlockPartition::new(10_005, 1000, 3).unwrap();
        assert_ne!(a.source_symbols, b.source_symbols);
        assert_ne!(a.larger_blocks, b.larger_blocks);
        assert_ne!(a.last_symbol_len(), b.last_symbol_len());
    }
}
