//! Random Index Pack — SMPTE ST 377-1:2019 §12, Tables 29-30
//! (`docs/st377-1.md`): the optional last KLV item in a file, letting a
//! decoder locate every Partition without a linear scan.

extern crate alloc;

use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::ber::{BerLength, ber_length_size_for, decode_ber_length, encode_ber_length_as};
use crate::error::{Error, Result};
use crate::types::ul_bytes_from_prefix;

const RIP_KEY_PREFIX: [u8; 7] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01];
const RIP_KEY_MID: [u8; 4] = [0x0D, 0x01, 0x02, 0x01];
/// Byte 14 (Set/Pack Kind = Random Index Pack) and byte 15 (RIP version).
const RIP_KEY_TAIL: [u8; 2] = [0x11, 0x01];

/// One `{BodySID, ByteOffset}` pair (Table 30) locating a single Partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PartitionLocation {
    /// Stream ID of the Body in that Partition (0 if none).
    pub body_sid: u32,
    /// Byte offset from the first byte of the Header Partition Pack Key
    /// (byte 0) to the first byte of that Partition's own Partition Pack
    /// Key.
    pub byte_offset: u64,
}

/// The Random Index Pack — SMPTE ST 377-1:2019 §12: one
/// [`PartitionLocation`] per Partition in the file (including Header and
/// Footer), ascending `byte_offset` order, plus a trailing overall-length
/// field (§12.2 Note 2) that lets a decoder seek from EOF directly to this
/// Pack's own Key without a forward scan.
///
/// `PartialEq`/`Eq` compare only `partitions` — `len_size` is a
/// serialization-*form* preference, not part of the Pack's logical value
/// (see [`crate::KlvItem`]'s doc; issue #1047 / audit MX-C1).
#[derive(Debug, Clone, Default)]
pub struct RandomIndexPack {
    /// One entry per Partition in the file, ascending `byte_offset` order.
    pub partitions: Vec<PartitionLocation>,
    /// The on-wire BER length-field width — [`BerLength::Minimal`] for a
    /// freshly built value, or the exact width `parse` found on the wire.
    pub len_size: BerLength,
}

impl PartialEq for RandomIndexPack {
    fn eq(&self, other: &Self) -> bool {
        self.partitions == other.partitions
    }
}

impl Eq for RandomIndexPack {}

impl RandomIndexPack {
    /// Build the 16-byte Random Index Pack Key (Table 29).
    #[must_use]
    pub fn key() -> crate::types::UlBytes {
        let mut key = [0u8; 16];
        key[0..7].copy_from_slice(&RIP_KEY_PREFIX);
        key[7] = 0x01; // registry version
        key[8..12].copy_from_slice(&RIP_KEY_MID);
        key[12] = 0x01; // Structure Kind
        key[13..15].copy_from_slice(&RIP_KEY_TAIL);
        key[15] = 0x00; // reserved
        key
    }

    /// True if `key` is the Random Index Pack Key (Table 29), ignoring
    /// byte 8 (registry version, wildcard).
    #[must_use]
    pub fn is_rip_key(key: &[u8; 16]) -> bool {
        key[0..7] == RIP_KEY_PREFIX
            && key[8..12] == RIP_KEY_MID
            && key[12] == 0x01
            && key[13..15] == RIP_KEY_TAIL
    }

    fn check_key(key: &[u8; 16]) -> Result<()> {
        if Self::is_rip_key(key) {
            Ok(())
        } else {
            Err(Error::KeyPrefixMismatch {
                what: "Random Index Pack (Table 29)",
            })
        }
    }

    /// Parse a Random Index Pack (Key + Length + Value, including the
    /// trailing overall-length field) from `bytes`.
    pub fn parse_prefix(bytes: &[u8]) -> Result<(Self, usize)> {
        if bytes.len() < 16 {
            return Err(Error::BufferTooShort {
                need: 16,
                have: bytes.len(),
                what: "Random Index Pack key",
            });
        }
        let key: [u8; 16] = ul_bytes_from_prefix(bytes);
        Self::check_key(&key)?;

        let (len, len_token_size) = decode_ber_length(&bytes[16..])?;
        let value_start = 16 + len_token_size;
        // #1108/MX-W1: `len` is a 64-bit BER length; a bare `as usize`
        // truncates it on a 32-bit target instead of rejecting an
        // over-range value.
        let len = usize::try_from(len).map_err(|_| Error::BufferTooShort {
            need: usize::MAX,
            have: bytes.len(),
            what: "Random Index Pack value (length exceeds platform usize)",
        })?;
        let value_end = value_start.checked_add(len).ok_or(Error::BufferTooShort {
            need: usize::MAX,
            have: bytes.len(),
            what: "Random Index Pack value (length overflow)",
        })?;
        if bytes.len() < value_end {
            return Err(Error::BufferTooShort {
                need: value_end,
                have: bytes.len(),
                what: "Random Index Pack value",
            });
        }
        let v = &bytes[value_start..value_end];
        // Value = N * {BodySID: u32, ByteOffset: u64} (12 bytes each) +
        // trailing overall Length: u32.
        if v.len() < 4 || !(v.len() - 4).is_multiple_of(12) {
            return Err(Error::InvalidBatchHeader {
                count: 0,
                item_len: 12,
                buffer_len: v.len(),
            });
        }
        let pairs_len = v.len() - 4;
        let mut partitions = Vec::with_capacity(pairs_len / 12);
        for chunk in v[..pairs_len].chunks_exact(12) {
            let body_sid = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            let byte_offset = u64::from_be_bytes([
                chunk[4], chunk[5], chunk[6], chunk[7], chunk[8], chunk[9], chunk[10], chunk[11],
            ]);
            partitions.push(PartitionLocation {
                body_sid,
                byte_offset,
            });
        }
        // Trailing Length field is a redundant seek-optimization value
        // (recomputed on serialize, not stored) — validated against the
        // pack's own actual total length.
        let trailing = u32::from_be_bytes([
            v[pairs_len],
            v[pairs_len + 1],
            v[pairs_len + 2],
            v[pairs_len + 3],
        ]);
        let actual_total = value_end as u64;
        if u64::from(trailing) != actual_total {
            return Err(Error::InvalidPropertyLength {
                tag: 0,
                name: "Random Index Pack trailing Length",
                found: trailing as usize,
                expected: actual_total as usize,
            });
        }

        Ok((
            RandomIndexPack {
                partitions,
                len_size: BerLength::fixed_from_consumed(len_token_size),
            },
            value_end,
        ))
    }
}

impl<'a> Parse<'a> for RandomIndexPack {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (rip, consumed) = Self::parse_prefix(bytes)?;
        if consumed != bytes.len() {
            return Err(Error::BufferTooShort {
                need: consumed,
                have: bytes.len(),
                what: "Random Index Pack (trailing bytes after exact-fit parse)",
            });
        }
        Ok(rip)
    }
}

impl Serialize for RandomIndexPack {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let value_len = self.partitions.len() * 12 + 4;
        16 + ber_length_size_for(value_len as u64, self.len_size) + value_len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: buf.len(),
                what: "Random Index Pack",
            });
        }
        buf[0..16].copy_from_slice(&Self::key());
        let value_len = self.partitions.len() * 12 + 4;
        let len_size = encode_ber_length_as(value_len as u64, self.len_size, &mut buf[16..])?;
        let mut pos = 16 + len_size;
        for p in &self.partitions {
            buf[pos..pos + 4].copy_from_slice(&p.body_sid.to_be_bytes());
            pos += 4;
            buf[pos..pos + 8].copy_from_slice(&p.byte_offset.to_be_bytes());
            pos += 8;
        }
        buf[pos..pos + 4].copy_from_slice(&(total as u32).to_be_bytes());
        pos += 4;
        Ok(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rip_round_trip() {
        let rip = RandomIndexPack {
            partitions: alloc::vec![
                PartitionLocation {
                    body_sid: 0,
                    byte_offset: 0,
                },
                PartitionLocation {
                    body_sid: 1,
                    byte_offset: 65536,
                },
                PartitionLocation {
                    body_sid: 0,
                    byte_offset: 131072,
                },
            ],
            ..Default::default()
        };
        let mut buf = alloc::vec![0u8; rip.serialized_len()];
        rip.serialize_into(&mut buf).unwrap();
        let parsed = RandomIndexPack::parse(&buf).unwrap();
        assert_eq!(parsed, rip);
        // Trailing 4 bytes equal the RIP's own total length (§12.2 Note 2).
        let trailing_len = u32::from_be_bytes(buf[buf.len() - 4..].try_into().unwrap());
        assert_eq!(trailing_len as usize, buf.len());
    }

    /// Issue #1047 (audit MX-C1): a non-minimal BER length token must
    /// round-trip byte-identically.
    #[test]
    fn non_minimal_length_token_round_trips_byte_identical() {
        let partitions = alloc::vec![PartitionLocation {
            body_sid: 1,
            byte_offset: 100,
        }];
        let value_len = partitions.len() * 12 + 4; // 16, fits short form
        assert!(value_len <= 0x7F);

        let mut original = alloc::vec::Vec::new();
        original.extend_from_slice(&RandomIndexPack::key());
        original.push(0x81); // long form, 1 following byte
        original.push(value_len as u8);
        for p in &partitions {
            original.extend_from_slice(&p.body_sid.to_be_bytes());
            original.extend_from_slice(&p.byte_offset.to_be_bytes());
        }
        let total_len = 16 + 2 + value_len;
        original.extend_from_slice(&(total_len as u32).to_be_bytes());

        let parsed = RandomIndexPack::parse(&original).unwrap();
        assert_eq!(
            parsed.len_size,
            BerLength::Fixed(core::num::NonZeroU8::new(2).unwrap())
        );

        let mut out = alloc::vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut out).unwrap();
        assert_eq!(out, original);
    }

    #[test]
    fn empty_rip_round_trip() {
        let rip = RandomIndexPack::default();
        let mut buf = alloc::vec![0u8; rip.serialized_len()];
        rip.serialize_into(&mut buf).unwrap();
        assert_eq!(RandomIndexPack::parse(&buf).unwrap(), rip);
    }

    #[test]
    fn wrong_trailing_length_rejected() {
        let rip = RandomIndexPack {
            partitions: alloc::vec![PartitionLocation {
                body_sid: 1,
                byte_offset: 100,
            }],
            ..Default::default()
        };
        let mut buf = alloc::vec![0u8; rip.serialized_len()];
        rip.serialize_into(&mut buf).unwrap();
        let last = buf.len() - 1;
        buf[last] ^= 0xFF; // corrupt the trailing Length field
        assert!(matches!(
            RandomIndexPack::parse(&buf),
            Err(Error::InvalidPropertyLength { .. })
        ));
    }
}
