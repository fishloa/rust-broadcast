//! Parental Rating Descriptor — ETSI EN 300 468 §6.2.30 (tag 0x55).
//!
//! Carried inside EIT. Per-country minimum-age rating for the event.

use crate::error::{Error, Result};
use crate::text::LangCode;
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for parental_rating_descriptor.
pub const TAG: u8 = 0x55;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 4;

/// One parental rating entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RatingEntry {
    /// ISO 3166 alpha country code (e.g. `LangCode(*b"FRA")`, `LangCode(*b"GBR")`).
    pub country_code: LangCode,
    /// Rating byte per §6.2.28 Table 79.
    pub rating: u8,
}

impl RatingEntry {
    /// Minimum age if `rating` falls in the numeric range, else None.
    #[must_use]
    pub fn minimum_age(&self) -> Option<u8> {
        match self.rating {
            0x01..=0x0F => Some(self.rating + 3),
            _ => None,
        }
    }
}

/// Parental Rating Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ParentalRatingDescriptor {
    /// Entries in wire order.
    pub entries: Vec<RatingEntry>,
}

impl<'a> Parse<'a> for ParentalRatingDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "ParentalRatingDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for parental_rating_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "ParentalRatingDescriptor body",
            });
        }
        if length % ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "descriptor_body_length is not a multiple of 4",
            });
        }
        let body_start = HEADER_LEN;
        let num_entries = length / ENTRY_LEN;
        let mut entries = Vec::with_capacity(num_entries);
        for i in 0..num_entries {
            let entry_start = body_start + i * ENTRY_LEN;
            entries.push(RatingEntry {
                country_code: LangCode([
                    bytes[entry_start],
                    bytes[entry_start + 1],
                    bytes[entry_start + 2],
                ]),
                rating: bytes[entry_start + 3],
            });
        }
        Ok(Self { entries })
    }
}

impl Serialize for ParentalRatingDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.entries.len() * ENTRY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = (len - HEADER_LEN) as u8;
        for (i, entry) in self.entries.iter().enumerate() {
            let entry_start = HEADER_LEN + i * ENTRY_LEN;
            buf[entry_start..entry_start + 3].copy_from_slice(&entry.country_code.0);
            buf[entry_start + 3] = entry.rating;
        }
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for ParentalRatingDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (self.serialized_len() - HEADER_LEN) as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for ParentalRatingDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "PARENTAL_RATING";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_entry_extracts_country_and_rating() {
        let bytes = [TAG, 4, b'F', b'R', b'A', 0x05];
        let d = ParentalRatingDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].country_code, LangCode(*b"FRA"));
        assert_eq!(d.entries[0].rating, 0x05);
    }

    #[test]
    fn parse_multiple_entries_preserves_order() {
        let bytes = [TAG, 8, b'G', b'B', b'R', 0x01, b'U', b'S', b'A', 0x10];
        let d = ParentalRatingDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[0].country_code, LangCode(*b"GBR"));
        assert_eq!(d.entries[0].rating, 0x01);
        assert_eq!(d.entries[1].country_code, LangCode(*b"USA"));
        assert_eq!(d.entries[1].rating, 0x10);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = ParentalRatingDescriptor::parse(&[0x4E, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x4E, .. }));
    }

    #[test]
    fn parse_rejects_length_not_multiple_of_4() {
        let bytes = [TAG, 3, b'F', b'R', b'A'];
        let err = ParentalRatingDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_truncated_body() {
        let bytes = [TAG, 4, b'F', b'R'];
        let err = ParentalRatingDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn minimum_age_maps_0x01_to_4_years() {
        let entry = RatingEntry {
            country_code: LangCode(*b"FRA"),
            rating: 0x01,
        };
        assert_eq!(entry.minimum_age(), Some(4));
    }

    #[test]
    fn minimum_age_returns_none_for_rating_0x00() {
        let entry = RatingEntry {
            country_code: LangCode(*b"USA"),
            rating: 0x00,
        };
        assert!(entry.minimum_age().is_none());
    }

    #[test]
    fn minimum_age_returns_none_for_rating_0x10_and_above() {
        let entry = RatingEntry {
            country_code: LangCode(*b"GBR"),
            rating: 0x10,
        };
        assert!(entry.minimum_age().is_none());
        let entry2 = RatingEntry {
            country_code: LangCode(*b"JPN"),
            rating: 0xFF,
        };
        assert!(entry2.minimum_age().is_none());
    }

    #[test]
    fn serialize_round_trip() {
        let d = ParentalRatingDescriptor {
            entries: vec![
                RatingEntry {
                    country_code: LangCode(*b"FRA"),
                    rating: 0x05,
                },
                RatingEntry {
                    country_code: LangCode(*b"GBR"),
                    rating: 0x01,
                },
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = ParentalRatingDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }
}
