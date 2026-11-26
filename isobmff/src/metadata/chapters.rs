//! Nero chapter lists (`chpl`).
//!
//! A `chpl` box is a full box: a version/flags word, a 32-bit reserved zero,
//! a one-byte chapter count, then per chapter a 64-bit start in 100 ns units
//! and a one-byte-length prefixed UTF-8 title. The layout is not in the Apple
//! or ISO transcriptions in `private/`; it is typed from the bytes of
//! `private/fixtures/mp4/tags/mp4box-chapters-nero.m4a` (written by GPAC), as
//! the brief requires. In that fixture the version is 1, the reserved word is
//! zero, the count byte is 2, and the two starts are 0 and 15 000 000 (1.5 s
//! at 100 ns units). Every field was checked against those bytes.

use alloc::string::String;
use alloc::vec::Vec;

use broadcast_core::len::{fit_u8, need_in, need_out};
use broadcast_core::{Parse, Serialize};

use crate::box_types::{
    BOX_HEADER_MIN_SIZE, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_OFFSET, box_len, write_box_header,
};
use crate::error::{Error, Result};

use super::U64_LEN;

/// The `chpl` box type.
pub const CHPL: [u8; 4] = *b"chpl";

/// The `FullBox` extension: version + flags.
const FULLBOX_LEN: usize = FULLBOX_EXTRA_SIZE;
/// The reserved word after the full-box header.
const RESERVED_LEN: usize = 4;
/// The one-byte chapter count.
const COUNT_LEN: usize = 1;
/// The smallest chapter entry: an 8-byte start and a one-byte title length.
const MIN_CHAPTER_LEN: usize = U64_LEN + 1;
/// The length field of one chapter title.
const TITLE_LEN_LEN: usize = 1;
/// The minimum `chpl` body: full-box header, reserved word and count.
const MIN_BODY_LEN: usize = FULLBOX_LEN + RESERVED_LEN + COUNT_LEN;

/// One chapter: a start time and a title.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Chapter {
    /// Start time in 100 ns units.
    pub start_100ns: u64,
    /// The chapter title.
    pub title: String,
}

impl Chapter {
    /// The start time in seconds.
    #[must_use]
    pub fn start_seconds(&self) -> f64 {
        self.start_100ns as f64 / super::CHAPTER_TIME_UNITS_PER_SECOND as f64
    }
}

/// A Nero chapter list (`chpl`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Chpl {
    /// FullBox version.
    pub version: u8,
    /// FullBox flags.
    pub flags: u32,
    /// The reserved word after the flags (zero in real files, kept so the
    /// round trip is exact).
    pub reserved: u32,
    /// Chapters in wire order.
    pub chapters: Vec<Chapter>,
}

impl Chpl {
    /// A version-1 chapter list from its chapters.
    #[must_use]
    pub fn new(chapters: Vec<Chapter>) -> Self {
        Self {
            version: 1,
            flags: 0,
            reserved: 0,
            chapters,
        }
    }

    fn body_len(&self) -> usize {
        MIN_BODY_LEN
            + self
                .chapters
                .iter()
                .map(|chapter| MIN_CHAPTER_LEN + chapter.title.len())
                .sum::<usize>()
    }
}

impl<'a> Parse<'a> for Chpl {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = crate::box_types::parse_box(bytes)?.0;
        if !item.header.box_type.is(&CHPL) {
            return Err(Error::UnexpectedBox { expected: "chpl" });
        }
        Self::parse_body(item.body)
    }
}

impl Chpl {
    /// Parse a `chpl` from its body (after the box header).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, MIN_BODY_LEN, "chpl body")?;
        let version = body[0];
        let flags = broadcast_common::wire::u24_at(body, FULLBOX_FLAGS_OFFSET);
        let reserved = super::u32_at(body, FULLBOX_LEN);
        let count = usize::from(body[FULLBOX_LEN + RESERVED_LEN]);
        let remaining = &body[MIN_BODY_LEN..];
        if count > remaining.len() / MIN_CHAPTER_LEN {
            return Err(Error::InvalidInput(
                "chpl chapter count cannot fit the remaining bytes",
            ));
        }
        let mut chapters = Vec::with_capacity(count);
        let mut cursor = MIN_BODY_LEN;
        for _ in 0..count {
            need_in(body, cursor + MIN_CHAPTER_LEN, "chpl chapter")?;
            let start_100ns = super::u64_at(body, cursor);
            cursor += U64_LEN;
            let title_len = usize::from(body[cursor]);
            cursor += TITLE_LEN_LEN;
            let end = cursor
                .checked_add(title_len)
                .ok_or(Error::InvalidInput("chpl title length overflows"))?;
            need_in(body, end, "chpl title")?;
            let title = core::str::from_utf8(&body[cursor..end])
                .map_err(|_| Error::InvalidInput("chpl title is not UTF-8"))?
                .into();
            cursor = end;
            chapters.push(Chapter { start_100ns, title });
        }
        if cursor != body.len() {
            return Err(Error::InvalidInput("chpl has trailing bytes"));
        }
        Ok(Self {
            version,
            flags,
            reserved,
            chapters,
        })
    }
}

impl Serialize for Chpl {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.body_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &CHPL, need)?;
        buf[cursor] = self.version;
        cursor += 1;
        let flags = self.flags.to_be_bytes();
        buf[cursor..cursor + (FULLBOX_LEN - 1)].copy_from_slice(&flags[1..]);
        cursor += FULLBOX_LEN - 1;
        buf[cursor..cursor + RESERVED_LEN].copy_from_slice(&self.reserved.to_be_bytes());
        cursor += RESERVED_LEN;
        let count = fit_u8(self.chapters.len(), "chpl chapter count")?;
        buf[cursor] = count;
        cursor += COUNT_LEN;
        for chapter in &self.chapters {
            buf[cursor..cursor + U64_LEN].copy_from_slice(&chapter.start_100ns.to_be_bytes());
            cursor += U64_LEN;
            let title_len = fit_u8(chapter.title.len(), "chpl title length")?;
            buf[cursor] = title_len;
            cursor += TITLE_LEN_LEN;
            buf[cursor..cursor + chapter.title.len()].copy_from_slice(chapter.title.as_bytes());
            cursor += chapter.title.len();
        }
        Ok(cursor)
    }
}
