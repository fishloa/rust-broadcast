//! Content Descriptor — ETSI EN 300 468 §6.2.9 (tag 0x54).
//!
//! Carried inside EIT. Classifies the event's genre via a two-nibble
//! content type plus an 8-bit broadcaster-specific user byte.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for content_descriptor.
pub const TAG: u8 = 0x54;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 2;

/// Content genre level-1 broad category — EN 300 468 Table 29.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ContentGenre {
    /// 0x0 — undefined content.
    UndefinedContent,
    /// 0x1 — Movie/Drama.
    MovieDrama,
    /// 0x2 — News/Current Affairs.
    NewsCurrentAffairs,
    /// 0x3 — Show/Game Show.
    ShowGameShow,
    /// 0x4 — Sports.
    Sports,
    /// 0x5 — Children/Youth programmes.
    ChildrenYouth,
    /// 0x6 — Music/Ballet/Dance.
    MusicBalletDance,
    /// 0x7 — Arts/Culture (without music).
    ArtsCulture,
    /// 0x8 — Social/Political issues/Economics.
    SocialPoliticalEconomics,
    /// 0x9 — Education/Science/Factual topics.
    EducationScienceFactual,
    /// 0xA — Leisure hobbies.
    LeisureHobbies,
    /// 0xB — Special characteristics.
    SpecialCharacteristics,
    /// 0xC — Adult.
    Adult,
    /// 0xD..=0xE — reserved for future use, preserved verbatim.
    Reserved(u8),
    /// 0xF — user defined, preserved verbatim.
    UserDefined(u8),
}

impl ContentGenre {
    /// Convert a level-1 nibble to a [`ContentGenre`].
    ///
    /// The input must be a 4-bit nibble value (`0..=0xF`); values outside
    /// this range are masked with `& 0x0F`.
    #[must_use]
    pub fn from_nibble_1(n1: u8) -> Self {
        let n1 = n1 & 0x0F;
        match n1 {
            0x0 => Self::UndefinedContent,
            0x1 => Self::MovieDrama,
            0x2 => Self::NewsCurrentAffairs,
            0x3 => Self::ShowGameShow,
            0x4 => Self::Sports,
            0x5 => Self::ChildrenYouth,
            0x6 => Self::MusicBalletDance,
            0x7 => Self::ArtsCulture,
            0x8 => Self::SocialPoliticalEconomics,
            0x9 => Self::EducationScienceFactual,
            0xA => Self::LeisureHobbies,
            0xB => Self::SpecialCharacteristics,
            0xC => Self::Adult,
            0xD | 0xE => Self::Reserved(n1),
            0xF => Self::UserDefined(n1),
            _ => Self::Reserved(n1),
        }
    }

    /// Returns the level-1 nibble for this genre (inverse of
    /// [`ContentGenre::from_nibble_1`]).
    #[must_use]
    pub fn to_nibble_1(self) -> u8 {
        self.to_u8()
    }

    /// Returns the wire byte for this value (same as the level-1 nibble).
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::UndefinedContent => 0x0,
            Self::MovieDrama => 0x1,
            Self::NewsCurrentAffairs => 0x2,
            Self::ShowGameShow => 0x3,
            Self::Sports => 0x4,
            Self::ChildrenYouth => 0x5,
            Self::MusicBalletDance => 0x6,
            Self::ArtsCulture => 0x7,
            Self::SocialPoliticalEconomics => 0x8,
            Self::EducationScienceFactual => 0x9,
            Self::LeisureHobbies => 0xA,
            Self::SpecialCharacteristics => 0xB,
            Self::Adult => 0xC,
            Self::Reserved(v) => v,
            Self::UserDefined(v) => v,
        }
    }

    /// Returns the broad level-1 category name per EN 300 468 Table 29.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::UndefinedContent => "undefined content",
            Self::MovieDrama => "Movie/Drama",
            Self::NewsCurrentAffairs => "News/Current Affairs",
            Self::ShowGameShow => "Show/Game Show",
            Self::Sports => "Sports",
            Self::ChildrenYouth => "Children/Youth",
            Self::MusicBalletDance => "Music/Ballet/Dance",
            Self::ArtsCulture => "Arts/Culture",
            Self::SocialPoliticalEconomics => "Social/Political/Economics",
            Self::EducationScienceFactual => "Education/Science/Factual",
            Self::LeisureHobbies => "Leisure hobbies",
            Self::SpecialCharacteristics => "Special characteristics",
            Self::Adult => "Adult",
            Self::Reserved(_) => "reserved",
            Self::UserDefined(_) => "user defined",
        }
    }
}

/// Return the most specific content genre name from EN 300 468 Table 29.
///
/// Maps `(nibble_1, nibble_2)` to the corresponding description string.
/// Returns `"unknown"` for unallocated combinations.
#[must_use]
pub fn content_genre_name(nibble_1: u8, nibble_2: u8) -> &'static str {
    match (nibble_1, nibble_2) {
        (0x0, 0x0..=0xF) => "undefined content",

        // Movie/Drama
        (0x1, 0x0) => "movie/drama (general)",
        (0x1, 0x1) => "detective/thriller",
        (0x1, 0x2) => "adventure/western/war",
        (0x1, 0x3) => "science fiction/fantasy/horror",
        (0x1, 0x4) => "comedy",
        (0x1, 0x5) => "soap/melodrama/folkloric",
        (0x1, 0x6) => "romance",
        (0x1, 0x7) => "serious/classical/religious/historical movie/drama",
        (0x1, 0x8) => "adult movie/drama",
        (0x1, 0x9..=0xE) => "reserved",
        (0x1, 0xF) => "user defined",

        // News/Current Affairs
        (0x2, 0x0) => "news/current affairs (general)",
        (0x2, 0x1) => "news/weather report",
        (0x2, 0x2) => "news magazine",
        (0x2, 0x3) => "documentary",
        (0x2, 0x4) => "discussion/interview/debate",
        (0x2, 0x5..=0xE) => "reserved",
        (0x2, 0xF) => "user defined",

        // Show/Game Show
        (0x3, 0x0) => "show/game show (general)",
        (0x3, 0x1) => "game show/quiz/contest",
        (0x3, 0x2) => "variety show",
        (0x3, 0x3) => "talk show",
        (0x3, 0x4..=0xE) => "reserved",
        (0x3, 0xF) => "user defined",

        // Sports
        (0x4, 0x0) => "sports (general)",
        (0x4, 0x1) => "special events (Olympic Games, World Cup, etc.)",
        (0x4, 0x2) => "sports magazines",
        (0x4, 0x3) => "football/soccer",
        (0x4, 0x4) => "tennis/squash",
        (0x4, 0x5) => "team sports (excluding football)",
        (0x4, 0x6) => "athletics",
        (0x4, 0x7) => "motor sport",
        (0x4, 0x8) => "water sport",
        (0x4, 0x9) => "winter sports",
        (0x4, 0xA) => "equestrian",
        (0x4, 0xB) => "martial sports",
        (0x4, 0xC..=0xE) => "reserved",
        (0x4, 0xF) => "user defined",

        // Children/Youth
        (0x5, 0x0) => "children's/youth programmes (general)",
        (0x5, 0x1) => "pre-school children's programmes",
        (0x5, 0x2) => "entertainment programmes for 6 to 14",
        (0x5, 0x3) => "entertainment programmes for 10 to 16",
        (0x5, 0x4) => "informational/educational/school programmes",
        (0x5, 0x5) => "cartoons/puppets",
        (0x5, 0x6..=0xE) => "reserved",
        (0x5, 0xF) => "user defined",

        // Music/Ballet/Dance
        (0x6, 0x0) => "music/ballet/dance (general)",
        (0x6, 0x1) => "rock/pop",
        (0x6, 0x2) => "serious music/classical music",
        (0x6, 0x3) => "folk/traditional music",
        (0x6, 0x4) => "jazz",
        (0x6, 0x5) => "musical/opera",
        (0x6, 0x6) => "ballet",
        (0x6, 0x7..=0xE) => "reserved",
        (0x6, 0xF) => "user defined",

        // Arts/Culture
        (0x7, 0x0) => "arts/culture (without music, general)",
        (0x7, 0x1) => "performing arts",
        (0x7, 0x2) => "fine arts",
        (0x7, 0x3) => "religion",
        (0x7, 0x4) => "popular culture/traditional arts",
        (0x7, 0x5) => "literature",
        (0x7, 0x6) => "film/cinema",
        (0x7, 0x7) => "experimental film/video",
        (0x7, 0x8) => "broadcasting/press",
        (0x7, 0x9) => "new media",
        (0x7, 0xA) => "arts/culture magazines",
        (0x7, 0xB) => "fashion",
        (0x7, 0xC..=0xE) => "reserved",
        (0x7, 0xF) => "user defined",

        // Social/Political/Economics
        (0x8, 0x0) => "social/political issues/economics (general)",
        (0x8, 0x1) => "magazines/reports/documentary",
        (0x8, 0x2) => "economics/social advisory",
        (0x8, 0x3) => "remarkable people",
        (0x8, 0x4..=0xE) => "reserved",
        (0x8, 0xF) => "user defined",

        // Education/Science/Factual
        (0x9, 0x0) => "education/science/factual topics (general)",
        (0x9, 0x1) => "nature/animals/environment",
        (0x9, 0x2) => "technology/natural sciences",
        (0x9, 0x3) => "medicine/physiology/psychology",
        (0x9, 0x4) => "foreign countries/expeditions",
        (0x9, 0x5) => "social/spiritual sciences",
        (0x9, 0x6) => "further education",
        (0x9, 0x7) => "languages",
        (0x9, 0x8..=0xE) => "reserved",
        (0x9, 0xF) => "user defined",

        // Leisure hobbies
        (0xA, 0x0) => "leisure hobbies (general)",
        (0xA, 0x1) => "tourism/travel",
        (0xA, 0x2) => "handicraft",
        (0xA, 0x3) => "motoring",
        (0xA, 0x4) => "fitness and health",
        (0xA, 0x5) => "cooking",
        (0xA, 0x6) => "advertisement/shopping",
        (0xA, 0x7) => "gardening",
        (0xA, 0x8..=0xE) => "reserved",
        (0xA, 0xF) => "user defined",

        // Special characteristics
        (0xB, 0x0) => "original language",
        (0xB, 0x1) => "black and white",
        (0xB, 0x2) => "unpublished",
        (0xB, 0x3) => "live broadcast",
        (0xB, 0x4) => "plano-stereoscopic",
        (0xB, 0x5) => "local or regional",
        (0xB, 0x6..=0xE) => "reserved",
        (0xB, 0xF) => "user defined",

        // Adult
        (0xC, 0x0) => "adult (general)",
        (0xC, 0x1..=0xE) => "reserved",
        (0xC, 0xF) => "user defined",

        // Reserved and user-defined
        (0xD..=0xE, 0x0..=0xF) => "reserved",
        (0xF, 0x0..=0xF) => "user defined",

        _ => "unknown",
    }
}

/// One content classification entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ContentEntry {
    /// content_nibble_level_1 (4 bits) — broad genre (ETSI Table 29).
    pub nibble_1: u8,
    /// content_nibble_level_2 (4 bits) — sub-genre.
    pub nibble_2: u8,
    /// Broadcaster-specific user byte.
    pub user_byte: u8,
}

impl ContentEntry {
    /// Level-1 broad category per EN 300 468 Table 29.
    #[must_use]
    pub fn genre(&self) -> ContentGenre {
        ContentGenre::from_nibble_1(self.nibble_1)
    }

    /// Most specific genre name per EN 300 468 Table 29.
    ///
    /// # Examples
    /// ```
    /// use dvb_si::descriptors::content::ContentEntry;
    ///
    /// let e = ContentEntry { nibble_1: 0x1, nibble_2: 0x4, user_byte: 0 };
    /// assert_eq!(e.genre_name(), "comedy");
    /// ```
    #[must_use]
    pub fn genre_name(&self) -> &'static str {
        content_genre_name(self.nibble_1, self.nibble_2)
    }
}

/// Content Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ContentDescriptor {
    /// Entries in wire order. EIT events can carry multiple genre entries.
    pub entries: Vec<ContentEntry>,
}

impl<'a> Parse<'a> for ContentDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ContentDescriptor",
            "unexpected tag for ContentDescriptor",
        )?;

        if body.len() % 2 != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "descriptor_length must be a multiple of 2",
            });
        }

        let mut entries = Vec::with_capacity(body.len() / ENTRY_LEN);

        for chunk in body.chunks_exact(ENTRY_LEN) {
            entries.push(ContentEntry {
                nibble_1: chunk[0] >> 4,
                nibble_2: chunk[0] & 0x0F,
                user_byte: chunk[1],
            });
        }

        Ok(Self { entries })
    }
}

impl Serialize for ContentDescriptor {
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
        let mut pos = HEADER_LEN;
        for entry in &self.entries {
            buf[pos] = (entry.nibble_1 << 4) | entry.nibble_2;
            buf[pos + 1] = entry.user_byte;
            pos += ENTRY_LEN;
        }
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for ContentDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "CONTENT";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_entry_extracts_nibbles_and_user_byte() {
        let result = ContentDescriptor::parse(&[TAG, 2, 0x31, 0xFF]).unwrap();
        assert_eq!(result.entries.len(), 1);
        let e = result.entries[0];
        assert_eq!(e.nibble_1, 3);
        assert_eq!(e.nibble_2, 1);
        assert_eq!(e.user_byte, 0xFF);
    }

    #[test]
    fn parse_multiple_entries_preserves_order() {
        let bytes = [TAG, 6, 0x31, 0xAA, 0x42, 0xBB, 0x53, 0xCC];
        let result = ContentDescriptor::parse(&bytes).unwrap();
        assert_eq!(result.entries.len(), 3);
        assert_eq!(result.entries[0].nibble_1, 3);
        assert_eq!(result.entries[0].nibble_2, 1);
        assert_eq!(result.entries[0].user_byte, 0xAA);
        assert_eq!(result.entries[1].nibble_1, 4);
        assert_eq!(result.entries[1].nibble_2, 2);
        assert_eq!(result.entries[1].user_byte, 0xBB);
        assert_eq!(result.entries[2].nibble_1, 5);
        assert_eq!(result.entries[2].nibble_2, 3);
        assert_eq!(result.entries[2].user_byte, 0xCC);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            ContentDescriptor::parse(&[0x55, 2, 0x00, 0x00]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x55, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_header() {
        assert!(matches!(
            ContentDescriptor::parse(&[TAG]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_body_truncation() {
        assert!(matches!(
            ContentDescriptor::parse(&[TAG, 4, 0x01]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_length_not_multiple_of_2() {
        assert!(matches!(
            ContentDescriptor::parse(&[TAG, 3, 0x01, 0x02, 0x03]).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let original = ContentDescriptor {
            entries: vec![
                ContentEntry {
                    nibble_1: 3,
                    nibble_2: 1,
                    user_byte: 0xAA,
                },
                ContentEntry {
                    nibble_1: 4,
                    nibble_2: 2,
                    user_byte: 0xBB,
                },
            ],
        };
        let mut buf = vec![0u8; original.serialized_len()];
        original.serialize_into(&mut buf).unwrap();
        let parsed = ContentDescriptor::parse(&buf).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn empty_descriptor_valid() {
        let bytes = [TAG, 0];
        let result = ContentDescriptor::parse(&bytes).unwrap();
        assert!(result.entries.is_empty());
    }

    #[test]
    fn content_genre_nibble1_round_trip() {
        for n1 in 0..=0xF_u8 {
            let genre = ContentGenre::from_nibble_1(n1);
            assert_eq!(
                genre.to_nibble_1(),
                n1,
                "round-trip failed for nibble 0x{n1:02X}"
            );
        }
    }

    #[test]
    fn content_genre_nibble1_masks_wide_input() {
        assert_eq!(ContentGenre::from_nibble_1(0x21), ContentGenre::MovieDrama);
        assert_eq!(ContentGenre::from_nibble_1(0xFF).to_nibble_1(), 0x0F);
    }

    #[test]
    fn content_genre_name_for_all_categories() {
        assert_eq!(ContentGenre::UndefinedContent.name(), "undefined content");
        assert_eq!(ContentGenre::MovieDrama.name(), "Movie/Drama");
        assert_eq!(ContentGenre::Sports.name(), "Sports");
        assert_eq!(ContentGenre::Reserved(0xD).name(), "reserved");
        assert_eq!(ContentGenre::UserDefined(0xF).name(), "user defined");
    }
}
