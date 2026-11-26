//! QuickTime chapter-list titles.
//!
//! A `chap` track reference points at a text track whose samples are chapter
//! titles. Source: `private/specs-verified/apple/isobmff/quicktime-file-format-online.md`,
//! "Chapter lists" and "Track reference atom (`tref`)".
//!
//! The reader never reads sample bytes itself: it reports the `tref` reference
//! on [`TrackInfo`](crate::reader::TrackInfo), and the caller hands each text
//! sample to [`chapter_title`].

use broadcast_common::wire::u16_at;
use broadcast_core::len::need_in;

use crate::error::{Error, Result};

/// The 16-bit length prefix every chapter-title sample starts with.
const LENGTH_PREFIX: usize = 2;

/// Decode one chapter-title sample.
///
/// A sample is a 16-bit big-endian length, that many bytes of text, and
/// optionally a trailing text-encoding atom this function ignores.
///
/// ```
/// use isobmff::quicktime::chapter_title;
///
/// let sample = *b"\x00\x05Intro";
/// assert_eq!(chapter_title(&sample).unwrap(), "Intro");
/// ```
///
/// # Errors
/// [`Error::BufferTooShort`] when the sample is shorter than its own prefix,
/// and [`Error::InvalidString`] when the title bytes are not UTF-8.
pub fn chapter_title(sample: &[u8]) -> Result<&str> {
    need_in(sample, LENGTH_PREFIX, "chapter title prefix")?;
    let len = usize::from(u16_at(sample, 0));
    let text = sample
        .get(LENGTH_PREFIX..LENGTH_PREFIX.saturating_add(len))
        .ok_or(Error::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: LENGTH_PREFIX.saturating_add(len),
                have: sample.len(),
                what: "chapter title text",
            },
        ))?;
    core::str::from_utf8(text).map_err(|_| Error::InvalidString {
        field: "chapter title",
        reason: "not UTF-8",
    })
}
