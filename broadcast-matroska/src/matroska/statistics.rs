//! The de-facto Matroska tag names a real file carries, typed.
//!
//! These names are not in RFC 9559
//! (`private/specs-verified/ietf/specs/rfc9559_matroska.txt` §5.1.8 defines
//! `Tags`, `Tag`, `Targets` and `SimpleTag` but no tag names): mkvmerge's track statistics (`DURATION`,
//! `NUMBER_OF_FRAMES`, `NUMBER_OF_BYTES`, `BPS`, `_STATISTICS_*`) come from
//! mkvtoolnix, and the technical tags (`ENCODER`, `ENCODER_SETTINGS`, `BPS`)
//! from the Matroska tagging page. It is a de-facto convention, taken from
//! those two sources and from the getID3 and FFmpeg readers that agree with
//! them (mkvtoolnix's `src/common/track_statistics.cpp` and its
//! track-statistics tags page; the sources note is recorded with the
//! workspace's spec notes).
//!
//! The typed view is derived: the `SimpleTag` element, its `TagName` and its
//! `TagString` stay in the tree untouched, so a file still rewrites byte for
//! byte. A value that does not match its documented form is kept as a
//! [`MalformedTag`] finding and the rest of the tag still reads: a damaged
//! tag never fails the file.

use alloc::vec::Vec;
use core::str::FromStr;

use crate::matroska::element::{SimpleTag, Tag, Tags};

/// `DURATION`, the total duration of the targeted part.
pub const DURATION: &str = "DURATION";
/// `NUMBER_OF_FRAMES`, the frame count of the targeted part.
pub const NUMBER_OF_FRAMES: &str = "NUMBER_OF_FRAMES";
/// `NUMBER_OF_BYTES`, the byte count of the targeted part.
pub const NUMBER_OF_BYTES: &str = "NUMBER_OF_BYTES";
/// `BPS`, the average bits per second.
pub const BPS: &str = "BPS";
/// `ENCODER`, the software or hardware that encoded the item.
pub const ENCODER: &str = "ENCODER";
/// `ENCODER_SETTINGS`, the settings used to encode the item.
pub const ENCODER_SETTINGS: &str = "ENCODER_SETTINGS";
/// `_STATISTICS_WRITING_APP`, the application that wrote the statistics.
pub const STATISTICS_WRITING_APP: &str = "_STATISTICS_WRITING_APP";
/// `_STATISTICS_WRITING_DATE_UTC`, when the statistics were written.
pub const STATISTICS_WRITING_DATE_UTC: &str = "_STATISTICS_WRITING_DATE_UTC";
/// `_STATISTICS_TAGS`, the statistics tag names the writer covered.
pub const STATISTICS_TAGS: &str = "_STATISTICS_TAGS";
/// The `TargetTypeValue` mkvmerge writes for a whole-file statistics tag.
pub const MOVIE_TARGET_TYPE_VALUE: u64 = 50;

/// Seconds in a minute.
const SECONDS_PER_MINUTE: u64 = 60;
/// Minutes in an hour.
const MINUTES_PER_HOUR: u64 = 60;
/// Nanoseconds in a second.
const NANOS_PER_SECOND: u64 = 1_000_000_000;
/// The most fractional-second digits a duration may carry.
const MAX_FRACTION_DIGITS: u32 = 9;
/// The digits a four-digit year carries.
const YEAR_DIGITS: usize = 4;
/// The digits a month or day carries.
const MONTH_DAY_DIGITS: usize = 2;
/// The radix every decimal integer and fraction digit is read in.
const DECIMAL_RADIX: u64 = 10;
/// The largest month a date may name.
const MAX_MONTH: u8 = 12;
/// The largest day any month may name.
const MAX_DAY: u8 = 31;
/// The largest hour a time may name.
const MAX_HOUR: u8 = 23;
/// The largest minute or second a time may name.
const MAX_MINUTE_SECOND: u8 = 59;

/// A moment a tag states in UTC, read as its civil fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CivilDateTime {
    /// The four-digit year.
    year: u16,
    /// The month, 1 through 12.
    month: u8,
    /// The day of the month, 1 through 31.
    day: u8,
    /// The hour, 0 through 23.
    hour: u8,
    /// The minute, 0 through 59.
    minute: u8,
    /// The second, 0 through 59.
    second: u8,
}

impl CivilDateTime {
    /// The four-digit year.
    #[must_use]
    pub const fn year(&self) -> u16 {
        self.year
    }

    /// The month, 1 through 12.
    #[must_use]
    pub const fn month(&self) -> u8 {
        self.month
    }

    /// The day of the month, 1 through 31.
    #[must_use]
    pub const fn day(&self) -> u8 {
        self.day
    }

    /// The hour, 0 through 23.
    #[must_use]
    pub const fn hour(&self) -> u8 {
        self.hour
    }

    /// The minute, 0 through 59.
    #[must_use]
    pub const fn minute(&self) -> u8 {
        self.minute
    }

    /// The second, 0 through 59.
    #[must_use]
    pub const fn second(&self) -> u8 {
        self.second
    }
}

/// A typed statistics tag whose value does not match its documented form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MalformedTag<'a> {
    /// The `TagName`.
    pub name: &'a str,
    /// The `TagString` as the file carries it.
    pub value: &'a str,
}

/// The typed view of one `Tag`'s `SimpleTag` children.
///
/// Every field is `None` when the file does not carry that tag. The raw
/// `SimpleTag` children stay in the element tree, so nothing here is the only
/// copy of a value.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Statistics<'a> {
    duration_nanoseconds: Option<u64>,
    bits_per_second: Option<f64>,
    number_of_frames: Option<u64>,
    number_of_bytes: Option<u64>,
    encoder: Option<&'a str>,
    encoder_settings: Option<&'a str>,
    writing_app: Option<&'a str>,
    writing_date: Option<CivilDateTime>,
    tags_list: Option<&'a str>,
    malformed: Vec<MalformedTag<'a>>,
}

impl<'a> Statistics<'a> {
    /// The `DURATION` in nanoseconds.
    #[must_use]
    pub const fn duration_nanoseconds(&self) -> Option<u64> {
        self.duration_nanoseconds
    }

    /// The `BPS` average bits per second.
    #[must_use]
    pub const fn bits_per_second(&self) -> Option<f64> {
        self.bits_per_second
    }

    /// The `NUMBER_OF_FRAMES` count.
    #[must_use]
    pub const fn number_of_frames(&self) -> Option<u64> {
        self.number_of_frames
    }

    /// The `NUMBER_OF_BYTES` count.
    #[must_use]
    pub const fn number_of_bytes(&self) -> Option<u64> {
        self.number_of_bytes
    }

    /// The `ENCODER` text.
    #[must_use]
    pub const fn encoder(&self) -> Option<&'a str> {
        self.encoder
    }

    /// The `ENCODER_SETTINGS` text.
    #[must_use]
    pub const fn encoder_settings(&self) -> Option<&'a str> {
        self.encoder_settings
    }

    /// The `_STATISTICS_WRITING_APP` text.
    #[must_use]
    pub const fn writing_app(&self) -> Option<&'a str> {
        self.writing_app
    }

    /// The `_STATISTICS_WRITING_DATE_UTC` moment.
    #[must_use]
    pub const fn writing_date(&self) -> Option<CivilDateTime> {
        self.writing_date
    }

    /// The `_STATISTICS_TAGS` name list as the file carries it.
    #[must_use]
    pub const fn tags_list(&self) -> Option<&'a str> {
        self.tags_list
    }

    /// Every known tag whose value did not match its documented form.
    #[must_use]
    pub fn malformed(&self) -> &[MalformedTag<'a>] {
        &self.malformed
    }

    /// Whether the tag carried no known name at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.duration_nanoseconds.is_none()
            && self.bits_per_second.is_none()
            && self.number_of_frames.is_none()
            && self.number_of_bytes.is_none()
            && self.encoder.is_none()
            && self.encoder_settings.is_none()
            && self.writing_app.is_none()
            && self.writing_date.is_none()
            && self.tags_list.is_none()
            && self.malformed.is_empty()
    }

    /// Read one `SimpleTag` into this view.
    fn absorb(&mut self, tag: &SimpleTag<'a>) {
        let (Some(name), Some(value)) = (tag.name(), tag.string()) else {
            return;
        };
        let name = semantic_text(name);
        let value = semantic_text(value);
        match name {
            DURATION => match parse_duration_nanoseconds(value) {
                Some(nanoseconds) => self.duration_nanoseconds = Some(nanoseconds),
                None => self.record(name, value),
            },
            BPS => match f64::from_str(value) {
                Ok(bits_per_second) if bits_per_second.is_finite() && bits_per_second >= 0.0 => {
                    self.bits_per_second = Some(bits_per_second);
                }
                _ => self.record(name, value),
            },
            NUMBER_OF_FRAMES => match u64::from_str(value) {
                Ok(count) => self.number_of_frames = Some(count),
                Err(_) => self.record(name, value),
            },
            NUMBER_OF_BYTES => match u64::from_str(value) {
                Ok(count) => self.number_of_bytes = Some(count),
                Err(_) => self.record(name, value),
            },
            ENCODER => self.encoder = Some(value),
            ENCODER_SETTINGS => self.encoder_settings = Some(value),
            STATISTICS_WRITING_APP => self.writing_app = Some(value),
            STATISTICS_WRITING_DATE_UTC => match parse_civil_datetime(value) {
                Some(when) => self.writing_date = Some(when),
                None => self.record(name, value),
            },
            STATISTICS_TAGS => self.tags_list = Some(value),
            _ => {}
        }
    }

    /// Keep a known tag whose value did not parse.
    fn record(&mut self, name: &'a str, value: &'a str) {
        self.malformed.push(MalformedTag { name, value });
    }
}

impl<'a> Tag<'a> {
    /// The typed statistics and encoder tags this tag carries.
    #[must_use]
    pub fn statistics(&self) -> Statistics<'a> {
        let mut statistics = Statistics::default();
        for tag in self.simple_tags() {
            statistics.absorb(tag);
        }
        statistics
    }
}

impl<'a> Tags<'a> {
    /// Every tag that targets `track_uid`, in file order.
    pub fn statistics_for_track(
        &self,
        track_uid: u64,
    ) -> impl Iterator<Item = Statistics<'a>> + '_ {
        self.tags()
            .filter(move |tag| {
                tag.targets()
                    .is_some_and(|targets| targets.track_uids().any(|uid| uid == track_uid))
            })
            .map(Tag::statistics)
    }
}

/// The value a String/UTF-8 element carries: it ends at its first Null
/// octet, which a writer may add as a terminator (RFC 8794 §13).
fn semantic_text(text: &str) -> &str {
    text.split('\0').next().unwrap_or(text)
}

/// Parse `HH:MM:SS.nnnnnnnnn` into nanoseconds.
fn parse_duration_nanoseconds(text: &str) -> Option<u64> {
    let mut fields = text.split(':');
    let hours = fields.next()?.parse::<u64>().ok()?;
    let minutes = fields.next()?.parse::<u64>().ok()?;
    let seconds_field = fields.next()?;
    if fields.next().is_some() || minutes >= SECONDS_PER_MINUTE {
        return None;
    }
    let (seconds, fraction) = match seconds_field.split_once('.') {
        Some((seconds, fraction)) => (seconds, Some(fraction)),
        None => (seconds_field, None),
    };
    let seconds = seconds.parse::<u64>().ok()?;
    if seconds >= SECONDS_PER_MINUTE {
        return None;
    }
    let mut nanoseconds = hours
        .checked_mul(MINUTES_PER_HOUR)?
        .checked_add(minutes)?
        .checked_mul(SECONDS_PER_MINUTE)?
        .checked_add(seconds)?
        .checked_mul(NANOS_PER_SECOND)?;
    if let Some(fraction) = fraction {
        if fraction.len() > MAX_FRACTION_DIGITS as usize
            || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let mut scale = NANOS_PER_SECOND;
        let mut value = 0u64;
        for byte in fraction.bytes() {
            value = value
                .checked_mul(DECIMAL_RADIX)?
                .checked_add(u64::from(byte - b'0'))?;
            scale /= DECIMAL_RADIX;
        }
        nanoseconds = nanoseconds.checked_add(value.checked_mul(scale)?)?;
    }
    Some(nanoseconds)
}

/// Parse `YYYY-MM-DD HH:MM:SS` into its civil fields.
fn parse_civil_datetime(text: &str) -> Option<CivilDateTime> {
    let (date, time) = text.split_once(' ')?;
    let mut date_fields = date.split('-');
    let year = date_fields.next()?;
    let month = date_fields.next()?;
    let day = date_fields.next()?;
    if date_fields.next().is_some()
        || year.len() != YEAR_DIGITS
        || month.len() != MONTH_DAY_DIGITS
        || day.len() != MONTH_DAY_DIGITS
    {
        return None;
    }
    let mut time_fields = time.split(':');
    let hour = time_fields.next()?;
    let minute = time_fields.next()?;
    let second = time_fields.next()?;
    if time_fields.next().is_some() {
        return None;
    }
    let when = CivilDateTime {
        year: u16::from_str(year).ok()?,
        month: u8::from_str(month).ok()?,
        day: u8::from_str(day).ok()?,
        hour: u8::from_str(hour).ok()?,
        minute: u8::from_str(minute).ok()?,
        second: u8::from_str(second).ok()?,
    };
    if when.month == 0
        || when.month > MAX_MONTH
        || when.day == 0
        || when.day > MAX_DAY
        || when.hour > MAX_HOUR
        || when.minute > MAX_MINUTE_SECOND
        || when.second > MAX_MINUTE_SECOND
    {
        return None;
    }
    Some(when)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_reads_hours_minutes_seconds_and_nanoseconds() {
        assert_eq!(
            parse_duration_nanoseconds("00:00:02.008000000"),
            Some(2_008_000_000)
        );
        assert_eq!(
            parse_duration_nanoseconds("01:02:03"),
            Some(3_723_000_000_000)
        );
        assert_eq!(
            parse_duration_nanoseconds("00:00:01.5"),
            Some(1_500_000_000)
        );
    }

    #[test]
    fn a_duration_that_is_not_hh_mm_ss_is_refused() {
        assert_eq!(parse_duration_nanoseconds("00:00"), None);
        assert_eq!(parse_duration_nanoseconds("00:60:00"), None);
        assert_eq!(parse_duration_nanoseconds("00:00:60"), None);
        assert_eq!(parse_duration_nanoseconds("00:00:01.0000000001"), None);
        assert_eq!(parse_duration_nanoseconds("00:00:01.x"), None);
        assert_eq!(parse_duration_nanoseconds("1:2:3:4"), None);
    }

    #[test]
    fn the_statistics_date_is_its_civil_fields() {
        let when = parse_civil_datetime("2019-11-18 15:45:32").expect("a date");
        assert_eq!((when.year(), when.month(), when.day()), (2019, 11, 18));
        assert_eq!((when.hour(), when.minute(), when.second()), (15, 45, 32));
    }

    #[test]
    fn an_out_of_range_date_is_refused() {
        assert_eq!(parse_civil_datetime("2019-13-18 15:45:32"), None);
        assert_eq!(parse_civil_datetime("2019-11-32 15:45:32"), None);
        assert_eq!(parse_civil_datetime("2019-11-18 24:45:32"), None);
        assert_eq!(parse_civil_datetime("2019-11-18"), None);
        assert_eq!(parse_civil_datetime("19-11-18 15:45:32"), None);
    }

    #[test]
    fn a_null_terminator_ends_the_value() {
        assert_eq!(semantic_text("ENCODER\0garbage"), "ENCODER");
        assert_eq!(semantic_text("ENCODER"), "ENCODER");
    }
}
