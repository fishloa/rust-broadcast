//! Typed `#EXT-X-DATERANGE` (RFC 8216bis §4.4.5.1).
//!
//! The playlist parser keeps a DATERANGE verbatim in
//! [`crate::MediaSegment::pre_tags`] (it is segment-scoped and applies until
//! the next occurrence, §4.4.1). This module is for a producer that has to
//! build the line itself: `ssai-runtime`, `timed-metadata` and the DVR archive
//! writer all assemble DATERANGE tags from typed data, and hand-building tag
//! text is how an unvalidated `"` or newline reaches an attribute list. Every
//! value here goes through [`AttrValue`]/[`render_attribute_list`], the same
//! constructors the rest of the crate renders with.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::{AttrValue, Error, ProgramDateTime, Result, render_attribute_list};

const TAG: &str = "#EXT-X-DATERANGE:";

/// The `X-` namespace of RFC 8216bis §4.4.5.1: a client-defined attribute
/// name, validated when constructed so an unrepresentable name can never reach
/// a rendered tag.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientAttrName(String);

impl ClientAttrName {
    /// Validate `name`: an `X-` prefix followed by at least one character from
    /// the §4.2 `AttributeName` set (`[A-Z]`, `[0-9]`, `-`).
    pub fn new(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        if !name.starts_with("X-") {
            return Err(Error::InvalidClientAttrName {
                name,
                reason: "must start with the X- prefix",
            });
        }
        if name.len() == 2 {
            return Err(Error::InvalidClientAttrName {
                name,
                reason: "must name something after the X- prefix",
            });
        }
        if !is_attribute_name(&name) {
            return Err(Error::InvalidClientAttrName {
                name,
                reason: "characters must be [A-Z], [0-9] or '-'",
            });
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

/// RFC 8216bis §4.2: an `AttributeName` contains only `[A-Z]`, `[0-9]` and `-`.
pub(crate) fn is_attribute_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
}

/// A typed `#EXT-X-DATERANGE` tag (RFC 8216bis §4.4.5.1).
///
/// `start`/`end` are the typed [`ProgramDateTime`] values. `start_date_text`/
/// `end_date_text` retain the exact wire spelling of that date when the tag
/// came from [`Self::parse_tag_line`], because the spec's own examples and
/// real playlists spell the same instant both with and without fractional
/// seconds and a byte-identical round trip must not rewrite a line it did not
/// have to touch. [`Self::to_tag_line`] uses the text only while it still
/// parses to the same typed value; a programmatically built range (text
/// `None`) renders the canonical `YYYY-MM-DDThh:mm:ss.SSSZ` form.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct DateRange {
    /// `ID` — the range's unique identifier (REQUIRED).
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
    /// `CUE` — the quoted enumerated-string-list of trigger identifiers
    /// (`PRE`, `POST`, `ONCE`), kept as its wire content.
    pub cue: Option<String>,
    /// The `X-` client-defined attributes, in wire order.
    pub client_attrs: Vec<(ClientAttrName, AttrValue)>,
    /// Attributes this crate does not model (`SCTE35-OUT`/`SCTE35-IN`/
    /// `SCTE35-CMD`, …), in wire order, so the whole line round-trips.
    pub extra_attrs: Vec<(String, AttrValue)>,
    /// The wire spelling of `START-DATE`, retained for a byte-identical round
    /// trip — see the type docs.
    pub start_date_text: Option<String>,
    /// The wire spelling of `END-DATE`, retained for a byte-identical round
    /// trip — see the type docs.
    pub end_date_text: Option<String>,
}

impl DateRange {
    /// Parse a complete `#EXT-X-DATERANGE:<attribute-list>` line.
    ///
    /// # Errors
    ///
    /// [`Error::HlsParse`] for a missing `ID`, an unparsable date or duration;
    /// [`Error::InvalidDateRange`] for a rule §4.4.5.1 states about this tag
    /// alone (the `END-ON-NEXT` constraints and an `END-DATE` before its
    /// `START-DATE`); [`Error::InvalidClientAttrName`] for an `X-` attribute
    /// whose name is not a legal `AttributeName`.
    pub fn parse_tag_line(line: &str) -> Result<Self> {
        let rest = line.strip_prefix(TAG).ok_or_else(|| Error::HlsParse {
            line_no: 0,
            line: line.to_string(),
            reason: "not an EXT-X-DATERANGE tag line".to_string(),
        })?;
        let (attrs, quoted, order) = super::scan_attribute_list_ordered(rest, true)?;
        let id = attrs.get("ID").cloned().ok_or_else(|| Error::HlsParse {
            line_no: 0,
            line: line.to_string(),
            reason: "EXT-X-DATERANGE missing required ID attribute".to_string(),
        })?;

        let start_date = attrs.get("START-DATE").cloned();
        let start = match &start_date {
            Some(v) => Some(parse_date(v, "START-DATE")?),
            None => None,
        };
        let end_date = attrs.get("END-DATE").cloned();
        let end = match &end_date {
            Some(v) => Some(parse_date(v, "END-DATE")?),
            None => None,
        };
        let duration = match attrs.get("DURATION") {
            Some(v) => Some(super::parse_decimal_seconds(v, 0, line, "DURATION")?),
            None => None,
        };
        let planned_duration = match attrs.get("PLANNED-DURATION") {
            Some(v) => Some(super::parse_decimal_seconds(
                v,
                0,
                line,
                "PLANNED-DURATION",
            )?),
            None => None,
        };
        let end_on_next = match attrs.get("END-ON-NEXT").map(String::as_str) {
            None => false,
            Some("YES") => true,
            Some(_) => {
                return Err(Error::InvalidDateRange {
                    id,
                    reason: "END-ON-NEXT must be YES",
                });
            }
        };

        const KNOWN: &[&str] = &[
            "ID",
            "CLASS",
            "START-DATE",
            "END-DATE",
            "DURATION",
            "PLANNED-DURATION",
            "END-ON-NEXT",
            "CUE",
        ];
        let mut client_attrs = Vec::new();
        let mut extra_attrs = Vec::new();
        for name in &order {
            if KNOWN.contains(&name.as_str()) {
                continue;
            }
            let value = attrs.get(name).cloned().unwrap_or_default();
            let value = if quoted.contains(name) {
                AttrValue::quoted(value)?
            } else {
                AttrValue::bare(value)?
            };
            if name.starts_with("X-") {
                client_attrs.push((ClientAttrName::new(name.clone())?, value));
            } else {
                if !is_attribute_name(name) {
                    return Err(Error::InvalidAttributeName { name: name.clone() });
                }
                extra_attrs.push((name.clone(), value));
            }
        }

        let range = Self {
            id,
            class: attrs.get("CLASS").cloned(),
            start,
            end,
            duration,
            planned_duration,
            end_on_next,
            cue: attrs.get("CUE").cloned(),
            client_attrs,
            extra_attrs,
            start_date_text: start_date,
            end_date_text: end_date,
        };
        range.validate()?;
        Ok(range)
    }

    /// Render the tag as one `#EXT-X-DATERANGE:<attribute-list>` line, without
    /// a trailing newline.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidDateRange`] when the fields violate a rule §4.4.5.1
    /// states about this tag alone; [`Error::InvalidQuotedString`] for an `ID`,
    /// `CLASS` or `CUE` that cannot be a quoted-string;
    /// [`Error::InvalidAttributeName`] for an unmodelled attribute name that is
    /// not a legal `AttributeName`.
    pub fn to_tag_line(&self) -> Result<String> {
        self.validate()?;
        let mut attrs = Vec::new();
        attrs.push((
            String::from("ID"),
            AttrValue::quoted(self.id.clone())?,
        ));
        if let Some(class) = &self.class {
            attrs.push((
                String::from("CLASS"),
                AttrValue::quoted(class.clone())?,
            ));
        }
        if let Some(attr) = date_attr("START-DATE", &self.start, &self.start_date_text)? {
            attrs.push(attr);
        }
        if let Some(attr) = date_attr("END-DATE", &self.end, &self.end_date_text)? {
            attrs.push(attr);
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
            attrs.push((String::from("CUE"), AttrValue::quoted(cue.clone())?));
        }
        for (name, value) in &self.client_attrs {
            attrs.push((name.as_str().to_string(), value.clone()));
        }
        for (name, value) in &self.extra_attrs {
            if !is_attribute_name(name) {
                return Err(Error::InvalidAttributeName { name: name.clone() });
            }
            attrs.push((name.clone(), value.clone()));
        }
        let mut s = String::from(TAG);
        let mut list = String::new();
        render_attribute_list(&mut list, &attrs);
        s.push_str(list.strip_prefix(',').unwrap_or(&list));
        Ok(s)
    }

    fn validate(&self) -> Result<()> {
        if self.end_on_next {
            if self.class.is_none() {
                return Err(Error::InvalidDateRange {
                    id: self.id.clone(),
                    reason: "END-ON-NEXT requires a CLASS attribute",
                });
            }
            if self.duration.is_some() {
                return Err(Error::InvalidDateRange {
                    id: self.id.clone(),
                    reason: "END-ON-NEXT must not carry a DURATION attribute",
                });
            }
            if self.end.is_some() {
                return Err(Error::InvalidDateRange {
                    id: self.id.clone(),
                    reason: "END-ON-NEXT must not carry an END-DATE attribute",
                });
            }
        }
        if let (Some(start), Some(end)) = (&self.start, &self.end)
            && end.timestamp() < start.timestamp()
        {
            return Err(Error::InvalidDateRange {
                id: self.id.clone(),
                reason: "END-DATE is before START-DATE",
            });
        }
        Ok(())
    }
}

fn parse_date(value: &str, what: &str) -> Result<ProgramDateTime> {
    ProgramDateTime::parse(value).map_err(|_| Error::HlsParse {
        line_no: 0,
        line: format!("{TAG}{what}={value:?}"),
        reason: format!("EXT-X-DATERANGE {what} is not a valid ISO 8601 date/time"),
    })
}

fn date_attr(
    name: &str,
    typed: &Option<ProgramDateTime>,
    text: &Option<String>,
) -> Result<Option<(String, AttrValue)>> {
    let Some(timestamp) = typed else {
        return Ok(None);
    };
    let rendered = match text {
        Some(text)
            if ProgramDateTime::parse(text).ok().as_ref() == Some(timestamp) =>
        {
            text.clone()
        }
        _ => timestamp.render(),
    };
    Ok(Some((
        name.to_string(),
        AttrValue::quoted(rendered)?,
    )))
}
