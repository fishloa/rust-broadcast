//! Typed `#EXT-X-DATERANGE` (RFC 8216bis §4.4.5.1).
//!
//! The playlist parser keeps a DATERANGE verbatim in
//! [`crate::MediaSegment::pre_tags`] (it is segment-scoped and applies until
//! the next occurrence, §4.4.1). This module is for a producer that has to
//! build the line itself: `ssai-runtime`, `timed-metadata` and the DVR archive
//! writer all assemble DATERANGE tags from typed data, and hand-building tag
//! text is how an unvalidated `"` or newline reaches an attribute list.
//!
//! A parsed tag keeps its attribute list in one wire-order vector, known and
//! unknown attributes alike, with each value's exact [`AttrValue`] (quoting
//! and decimal text included), so [`DateRange::to_tag_line`] reproduces an
//! accepted line byte-for-byte. Attributes outside the §4.2 grammar, or a
//! known attribute with the wrong value kind, are rejected at parse time.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::{
    AttrValue, ClientAttrNameError, DateRangeError, Error, ProgramDateTime, Result,
    render_attribute_list,
};

const TAG: &str = "#EXT-X-DATERANGE:";

/// The `X-` namespace of RFC 8216bis §4.4.5.1: a client-defined attribute
/// name, validated when constructed so an unrepresentable name can never reach
/// a rendered tag.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientAttrName(String);

impl ClientAttrName {
    /// Validate `name`: an `X-` prefix followed by at least one name character
    /// from the §4.2 `AttributeName` set (`[A-Z]`, `[0-9]`, `-`), not ending in
    /// `-`.
    pub fn new(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        let Some(rest) = name.strip_prefix("X-") else {
            return Err(Error::InvalidClientAttrName {
                name,
                reason: ClientAttrNameError::MissingPrefix,
            });
        };
        let reason = if rest.is_empty() {
            Some(ClientAttrNameError::EmptyName)
        } else if !rest
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
        {
            Some(ClientAttrNameError::InvalidCharacter)
        } else if !rest
            .bytes()
            .any(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            Some(ClientAttrNameError::EmptyName)
        } else if rest.ends_with('-') {
            Some(ClientAttrNameError::TrailingHyphen)
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(Error::InvalidClientAttrName { name, reason });
        }
        Ok(Self(name))
    }

    /// The name as it appears on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for ClientAttrName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A Trigger Identifier of an `EXT-X-DATERANGE` `CUE` attribute
/// (RFC 8216bis §4.4.5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CueTrigger {
    /// `PRE` — trigger before playback of the primary asset.
    Pre,
    /// `POST` — trigger after the primary asset ends.
    Post,
    /// `ONCE` — trigger once.
    Once,
}

impl CueTrigger {
    /// The spec token.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Pre => "PRE",
            Self::Post => "POST",
            Self::Once => "ONCE",
        }
    }

    fn parse(token: &str) -> Option<Self> {
        match token {
            "PRE" => Some(Self::Pre),
            "POST" => Some(Self::Post),
            "ONCE" => Some(Self::Once),
            _ => None,
        }
    }
}

broadcast_common::impl_spec_display!(CueTrigger);

/// The `CUE` attribute's set of Trigger Identifiers (RFC 8216bis §4.4.5.1),
/// in wire order. `PRE` and `POST` are mutually exclusive and a trigger may
/// not repeat.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Cue {
    /// The triggers, in wire order.
    pub triggers: Vec<CueTrigger>,
}

impl Cue {
    /// Validate and build a trigger set.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidDateRange`] for an empty list, a repeated trigger, or
    /// `PRE` together with `POST`.
    pub fn new(triggers: Vec<CueTrigger>) -> Result<Self> {
        if triggers.is_empty() {
            return Err(invalid("", DateRangeError::InvalidCue));
        }
        let mut pre = false;
        let mut post = false;
        for (i, trigger) in triggers.iter().enumerate() {
            if triggers[..i].contains(trigger) {
                return Err(invalid("", DateRangeError::DuplicateCueTrigger));
            }
            match trigger {
                CueTrigger::Pre => pre = true,
                CueTrigger::Post => post = true,
                CueTrigger::Once => {}
            }
        }
        if pre && post {
            return Err(invalid("", DateRangeError::CuePreAndPost));
        }
        Ok(Self { triggers })
    }

    /// Parse the content of a quoted `CUE` value.
    ///
    /// # Errors
    ///
    /// As [`Self::new`], plus [`DateRangeError::InvalidCue`] for an unknown
    /// trigger token.
    pub fn parse(value: &str) -> Result<Self> {
        let mut triggers = Vec::new();
        for token in value.split(',') {
            let Some(trigger) = CueTrigger::parse(token) else {
                return Err(invalid("", DateRangeError::InvalidCue));
            };
            triggers.push(trigger);
        }
        Self::new(triggers)
    }

    /// The comma-separated wire content (`PRE,ONCE`), without quotes.
    #[must_use]
    pub fn render(&self) -> String {
        let mut s = String::new();
        for (i, trigger) in self.triggers.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(trigger.name());
        }
        s
    }
}

/// A typed `#EXT-X-DATERANGE` tag (RFC 8216bis §4.4.5.1).
///
/// Build one with [`Self::new`] and set the optional fields, or parse a line:
/// a parsed tag retains its exact attribute order, quoting and decimal text
/// (privately) so [`Self::to_tag_line`] reproduces that line byte-for-byte.
/// `end` must not precede `start`, and when `start`, `end` and `duration` are
/// all present `end` must equal `start + duration` (§4.4.5.1).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct DateRange {
    /// `ID` — the range's unique identifier (REQUIRED, non-empty).
    pub id: String,
    /// `CLASS` — the attribute-semantics set this range belongs to.
    pub class: Option<String>,
    /// `START-DATE` — the instant the range begins. `None` only for an
    /// augmentation tag that repeats an earlier range's `ID` without restating
    /// its start (§4.4.5.1).
    pub start: Option<ProgramDateTime>,
    /// `END-DATE` — the instant the range ends.
    pub end: Option<ProgramDateTime>,
    /// `DURATION` — the range length in seconds.
    pub duration: Option<crate::DecimalSeconds>,
    /// `PLANNED-DURATION` — the expected range length in seconds.
    pub planned_duration: Option<crate::DecimalSeconds>,
    /// `END-ON-NEXT=YES` — the range ends where the following range of the
    /// same `CLASS` begins.
    pub end_on_next: bool,
    /// `CUE` — the trigger identifiers.
    pub cue: Option<Cue>,
    /// The `X-` client-defined attributes, in wire order.
    pub client_attrs: Vec<(ClientAttrName, AttrValue)>,
    /// Attributes this crate does not model (`SCTE35-OUT`/`SCTE35-IN`/
    /// `SCTE35-CMD`, and any unknown name), in wire order.
    pub extra_attrs: Vec<(String, AttrValue)>,
    wire: Option<Vec<(String, AttrValue)>>,
}

impl DateRange {
    /// Start a range with the required `ID`; optional fields stay absent.
    ///
    /// # Errors
    ///
    /// [`DateRangeError::EmptyId`] when `id` is empty.
    pub fn new(id: impl Into<String>) -> Result<Self> {
        let id = id.into();
        if id.is_empty() {
            return Err(invalid("", DateRangeError::EmptyId));
        }
        Ok(Self {
            id,
            class: None,
            start: None,
            end: None,
            duration: None,
            planned_duration: None,
            end_on_next: false,
            cue: None,
            client_attrs: Vec::new(),
            extra_attrs: Vec::new(),
            wire: None,
        })
    }

    /// Whether this tag was parsed from the wire and still carries its exact
    /// attribute list.
    #[must_use]
    pub fn has_wire_attributes(&self) -> bool {
        self.wire.is_some()
    }

    /// Parse a complete `#EXT-X-DATERANGE:<attribute-list>` line.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidAttributeList`] for text outside the §4.2 grammar;
    /// [`Error::InvalidClientAttrName`] for an illegal `X-` name;
    /// [`Error::InvalidDateRange`] for a known attribute with the wrong value
    /// kind or a rule §4.4.5.1 states about this tag alone; [`Error::HlsParse`]
    /// for a missing `ID` or an unparsable date.
    pub fn parse_tag_line(line: &str) -> Result<Self> {
        let rest = line.strip_prefix(TAG).ok_or_else(|| Error::HlsParse {
            line_no: 0,
            line: line.to_string(),
            reason: "not an EXT-X-DATERANGE tag line".to_string(),
        })?;
        let wire = super::scan_attribute_list_wire(rest)?;
        let mut id = String::new();
        let mut class = None;
        let mut start = None;
        let mut end = None;
        let mut duration = None;
        let mut planned_duration = None;
        let mut end_on_next = false;
        let mut cue = None;
        let mut client_attrs = Vec::new();
        let mut extra_attrs = Vec::new();

        for (name, value) in &wire {
            match name.as_str() {
                "ID" => id = quoted_value(value, &id)?,
                "CLASS" => class = Some(quoted_value(value, &id)?),
                "START-DATE" => start = Some(parse_date_attr(value, &id, "START-DATE")?),
                "END-DATE" => end = Some(parse_date_attr(value, &id, "END-DATE")?),
                "DURATION" => duration = Some(duration_attr(value, &id)?),
                "PLANNED-DURATION" => planned_duration = Some(duration_attr(value, &id)?),
                "END-ON-NEXT" => {
                    if value.is_quoted() || value.as_str() != "YES" {
                        return Err(invalid(&id, DateRangeError::EndOnNextNotYes));
                    }
                    end_on_next = true;
                }
                "CUE" => {
                    if !value.is_quoted() {
                        return Err(invalid(&id, DateRangeError::WrongAttributeKind));
                    }
                    cue = Some(Cue::parse(value.as_str())?);
                }
                name if name.starts_with("X-") => {
                    if !client_value_ok(value) {
                        return Err(invalid(&id, DateRangeError::InvalidClientAttributeValue));
                    }
                    client_attrs.push((ClientAttrName::new(name)?, value.clone()));
                }
                "SCTE35-CMD" | "SCTE35-OUT" | "SCTE35-IN" => {
                    if value.is_quoted() || !super::is_hexadecimal_sequence(value.as_str()) {
                        return Err(invalid(&id, DateRangeError::InvalidScte35Value));
                    }
                    extra_attrs.push((name.clone(), value.clone()));
                }
                _ => extra_attrs.push((name.clone(), value.clone())),
            }
        }
        if id.is_empty() {
            return Err(Error::HlsParse {
                line_no: 0,
                line: line.to_string(),
                reason: "EXT-X-DATERANGE missing required ID attribute".to_string(),
            });
        }
        let range = Self {
            id,
            class,
            start,
            end,
            duration,
            planned_duration,
            end_on_next,
            cue,
            client_attrs,
            extra_attrs,
            wire: Some(wire),
        };
        range.validate()?;
        Ok(range)
    }

    /// Render the tag as one `#EXT-X-DATERANGE:<attribute-list>` line, without
    /// a trailing newline. A parsed tag reproduces its accepted line
    /// byte-for-byte (attribute order, quoting and decimal text included).
    ///
    /// # Errors
    ///
    /// [`DateRangeError`] for a rule §4.4.5.1 states about this tag alone or
    /// an attribute value of the wrong kind; [`Error::InvalidQuotedString`] for
    /// an `ID`, `CLASS` or `CUE` that cannot be a quoted-string;
    /// [`Error::InvalidAttributeName`] for an unmodelled attribute name that is
    /// not a legal `AttributeName`.
    pub fn to_tag_line(&self) -> Result<String> {
        self.validate()?;
        let attrs = match &self.wire {
            Some(wire) => wire.clone(),
            None => self.render_attrs()?,
        };
        let mut s = String::from(TAG);
        let mut list = String::new();
        render_attribute_list(&mut list, &attrs);
        s.push_str(list.strip_prefix(',').unwrap_or(&list));
        Ok(s)
    }

    fn render_attrs(&self) -> Result<Vec<(String, AttrValue)>> {
        let mut attrs = Vec::new();
        attrs.push((String::from("ID"), AttrValue::quoted(self.id.clone())?));
        if let Some(class) = &self.class {
            attrs.push((String::from("CLASS"), AttrValue::quoted(class.clone())?));
        }
        if let Some(start) = &self.start {
            attrs.push((
                String::from("START-DATE"),
                AttrValue::quoted(start.render())?,
            ));
        }
        if let Some(end) = &self.end {
            attrs.push((String::from("END-DATE"), AttrValue::quoted(end.render())?));
        }
        if let Some(duration) = self.duration {
            attrs.push((
                String::from("DURATION"),
                AttrValue::bare(super::format_secs(duration))?,
            ));
        }
        if let Some(planned) = self.planned_duration {
            attrs.push((
                String::from("PLANNED-DURATION"),
                AttrValue::bare(super::format_secs(planned))?,
            ));
        }
        if self.end_on_next {
            attrs.push((String::from("END-ON-NEXT"), AttrValue::bare("YES")?));
        }
        if let Some(cue) = &self.cue {
            attrs.push((String::from("CUE"), AttrValue::quoted(cue.render())?));
        }
        for (name, value) in &self.client_attrs {
            if !client_value_ok(value) {
                return Err(invalid(
                    &self.id,
                    DateRangeError::InvalidClientAttributeValue,
                ));
            }
            attrs.push((name.as_str().to_string(), value.clone()));
        }
        for (name, value) in &self.extra_attrs {
            if !is_attribute_name(name) {
                return Err(Error::InvalidAttributeName { name: name.clone() });
            }
            if matches!(name.as_str(), "SCTE35-CMD" | "SCTE35-OUT" | "SCTE35-IN")
                && (value.is_quoted() || !super::is_hexadecimal_sequence(value.as_str()))
            {
                return Err(invalid(&self.id, DateRangeError::InvalidScte35Value));
            }
            attrs.push((name.clone(), value.clone()));
        }
        Ok(attrs)
    }

    fn validate(&self) -> Result<()> {
        if self.id.is_empty() {
            return Err(invalid("", DateRangeError::EmptyId));
        }
        if self.end_on_next {
            if self.class.is_none() {
                return Err(invalid(&self.id, DateRangeError::EndOnNextWithoutClass));
            }
            if self.duration.is_some() {
                return Err(invalid(&self.id, DateRangeError::EndOnNextWithDuration));
            }
            if self.end.is_some() {
                return Err(invalid(&self.id, DateRangeError::EndOnNextWithEndDate));
            }
        }
        if let (Some(start), Some(end)) = (&self.start, &self.end)
            && end.timestamp() < start.timestamp()
        {
            return Err(invalid(&self.id, DateRangeError::EndBeforeStart));
        }
        if let (Some(start), Some(end), Some(duration)) = (&self.start, &self.end, self.duration) {
            let millis = duration.get() * 1000.0;
            let delta = if millis.is_finite() && millis >= 0.0 && millis <= i64::MAX as f64 {
                Some((millis + 0.5) as i64)
            } else {
                None
            };
            let actual = end
                .timestamp()
                .as_millisecond()
                .checked_sub(start.timestamp().as_millisecond());
            if delta.is_none() || actual != delta {
                return Err(invalid(&self.id, DateRangeError::EndDateDurationMismatch));
            }
        }
        for (_, value) in &self.client_attrs {
            if !client_value_ok(value) {
                return Err(invalid(
                    &self.id,
                    DateRangeError::InvalidClientAttributeValue,
                ));
            }
        }
        for (name, value) in &self.extra_attrs {
            if !is_attribute_name(name) {
                return Err(Error::InvalidAttributeName { name: name.clone() });
            }
            if matches!(name.as_str(), "SCTE35-CMD" | "SCTE35-OUT" | "SCTE35-IN")
                && (value.is_quoted() || !super::is_hexadecimal_sequence(value.as_str()))
            {
                return Err(invalid(&self.id, DateRangeError::InvalidScte35Value));
            }
        }
        Ok(())
    }
}

fn invalid(id: &str, reason: DateRangeError) -> Error {
    Error::InvalidDateRange {
        id: id.to_string(),
        reason,
    }
}

fn quoted_value(value: &AttrValue, id: &str) -> Result<String> {
    if !value.is_quoted() {
        return Err(invalid(id, DateRangeError::WrongAttributeKind));
    }
    Ok(value.as_str().to_string())
}

fn parse_date_attr(value: &AttrValue, id: &str, what: &str) -> Result<ProgramDateTime> {
    if !value.is_quoted() {
        return Err(invalid(id, DateRangeError::WrongAttributeKind));
    }
    ProgramDateTime::parse(value.as_str()).map_err(|_| Error::HlsParse {
        line_no: 0,
        line: format!("{TAG}{what}={:?}", value.as_str()),
        reason: format!("EXT-X-DATERANGE {what} is not a valid ISO 8601 date/time"),
    })
}

fn duration_attr(value: &AttrValue, id: &str) -> Result<crate::DecimalSeconds> {
    if value.is_quoted() || !super::is_valid_decimal_token(value.as_str(), false) {
        return Err(invalid(id, DateRangeError::InvalidDurationValue));
    }
    let seconds = value
        .as_str()
        .parse::<f64>()
        .map_err(|_| invalid(id, DateRangeError::InvalidDurationValue))?;
    crate::DecimalSeconds::new(seconds)
        .map_err(|_| invalid(id, DateRangeError::InvalidDurationValue))
}

fn client_value_ok(value: &AttrValue) -> bool {
    value.is_quoted()
        || super::is_hexadecimal_sequence(value.as_str())
        || super::is_valid_decimal_token(value.as_str(), true)
}

/// RFC 8216bis §4.2: an `AttributeName` contains only `[A-Z]`, `[0-9]` and `-`.
pub(crate) fn is_attribute_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
}
