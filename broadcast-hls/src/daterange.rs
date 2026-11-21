//! Typed `#EXT-X-DATERANGE` (RFC 8216bis §4.4.5.1).
//!
//! The playlist parser keeps a DATERANGE verbatim in
//! [`crate::MediaSegment::pre_tags`] (it is segment-scoped and applies until
//! the next occurrence, §4.4.1). This module is for a producer that has to
//! build the line itself: `ssai-runtime`, `timed-metadata` and the DVR archive
//! writer all assemble DATERANGE tags from typed data, and hand-building tag
//! text is how an unvalidated `"` or newline reaches an attribute list.
//!
//! The only storage is ONE ordered attribute list, holding each value's exact
//! wire spelling. A parsed tag therefore re-renders byte-for-byte, while a
//! setter replaces that attribute's entry in place (or appends it) and renders
//! the new value canonically — there is no second cache that could go stale.
//! Equality is semantic: the decoded typed values, not spelling or order.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::{
    AttrValue, ClientAttrNameError, DateRangeError, Error, ProgramDateTime, Result,
    render_attribute_list,
};

const TAG: &str = "#EXT-X-DATERANGE:";

/// Attributes in the canonical order a newly built tag uses.
const KNOWN_ORDER: &[&str] = &[
    "ID",
    "CLASS",
    "START-DATE",
    "END-DATE",
    "DURATION",
    "PLANNED-DURATION",
    "END-ON-NEXT",
    "CUE",
];

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

/// Which `SCTE35-*` attribute carries the splice payload (RFC 8216bis
/// §4.4.5.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Scte35Kind {
    /// `SCTE35-CMD` — a splice command other than an out/in pair.
    Cmd,
    /// `SCTE35-OUT` — the start of a break.
    Out,
    /// `SCTE35-IN` — the return from a break.
    In,
}

impl Scte35Kind {
    /// The attribute name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Cmd => "SCTE35-CMD",
            Self::Out => "SCTE35-OUT",
            Self::In => "SCTE35-IN",
        }
    }
}

broadcast_common::impl_spec_display!(Scte35Kind);

/// The `CUE` attribute's set of Trigger Identifiers (RFC 8216bis §4.4.5.1),
/// in wire order. `PRE` and `POST` are mutually exclusive and a trigger may
/// not repeat.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Cue {
    triggers: Vec<CueTrigger>,
}

impl Cue {
    /// The validated triggers, in wire order.
    #[must_use]
    pub fn triggers(&self) -> &[CueTrigger] {
        &self.triggers
    }

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
/// The attributes are private: the one ordered list is the storage, and the
/// typed accessors decode it. `PartialEq` compares those decoded values, so a
/// parsed tag and one built with the same values are equal even when their
/// attribute order or a decimal's spelling differs.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DateRange {
    attrs: Vec<(String, AttrValue)>,
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
            attrs: alloc::vec![(String::from("ID"), AttrValue::quoted(id)?)],
        })
    }

    fn attr(&self, name: &str) -> Option<&AttrValue> {
        self.attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    fn quoted_attr(&self, name: &str) -> Option<&str> {
        self.attr(name)
            .filter(|v| v.is_quoted())
            .map(AttrValue::as_str)
    }

    fn bare_attr(&self, name: &str) -> Option<&str> {
        self.attr(name)
            .filter(|v| !v.is_quoted())
            .map(AttrValue::as_str)
    }

    /// `ID` — always non-empty.
    #[must_use]
    pub fn id(&self) -> &str {
        self.quoted_attr("ID").unwrap_or("")
    }

    /// `CLASS`, decoded from its quoted-string.
    #[must_use]
    pub fn class(&self) -> Option<&str> {
        self.quoted_attr("CLASS")
    }

    /// `START-DATE`.
    #[must_use]
    pub fn start(&self) -> Option<ProgramDateTime> {
        self.quoted_attr("START-DATE")
            .and_then(|v| ProgramDateTime::parse(v).ok())
    }

    /// `END-DATE`.
    #[must_use]
    pub fn end(&self) -> Option<ProgramDateTime> {
        self.quoted_attr("END-DATE")
            .and_then(|v| ProgramDateTime::parse(v).ok())
    }

    /// `DURATION`.
    #[must_use]
    pub fn duration(&self) -> Option<crate::DecimalSeconds> {
        self.bare_attr("DURATION").and_then(decode_duration)
    }

    /// `PLANNED-DURATION`.
    #[must_use]
    pub fn planned_duration(&self) -> Option<crate::DecimalSeconds> {
        self.bare_attr("PLANNED-DURATION").and_then(decode_duration)
    }

    /// `END-ON-NEXT`.
    #[must_use]
    pub fn end_on_next(&self) -> bool {
        self.bare_attr("END-ON-NEXT") == Some("YES")
    }

    /// `CUE`, decoded from its quoted enumerated-string-list.
    #[must_use]
    pub fn cue(&self) -> Option<Cue> {
        self.quoted_attr("CUE").and_then(|v| Cue::parse(v).ok())
    }

    /// The `X-` client attributes, in attribute order.
    #[must_use]
    pub fn client_attrs(&self) -> Vec<(ClientAttrName, AttrValue)> {
        self.attrs
            .iter()
            .filter(|(n, _)| n.starts_with("X-"))
            .filter_map(|(n, v)| {
                ClientAttrName::new(n.clone())
                    .ok()
                    .map(|name| (name, v.clone()))
            })
            .collect()
    }

    /// Attributes this crate does not model (`SCTE35-*` and any unknown
    /// non-`X-` name), in attribute order.
    #[must_use]
    pub fn extra_attrs(&self) -> Vec<(String, AttrValue)> {
        self.attrs
            .iter()
            .filter(|(n, _)| !KNOWN_ORDER.contains(&n.as_str()) && !n.starts_with("X-"))
            .cloned()
            .collect()
    }

    /// A client attribute's value.
    #[must_use]
    pub fn client_attr(&self, name: &ClientAttrName) -> Option<&AttrValue> {
        self.attr(name.as_str())
    }
}

impl DateRange {
    fn remove(&mut self, name: &str) {
        self.attrs.retain(|(n, _)| n != name);
    }

    fn replace_or_append(&mut self, name: &str, value: AttrValue) {
        if let Some((_, existing)) = self.attrs.iter_mut().find(|(n, _)| n == name) {
            *existing = value;
        } else {
            self.attrs.push((name.to_string(), value));
        }
    }

    fn insert_known(&mut self, name: &str, value: AttrValue) {
        if let Some((_, existing)) = self.attrs.iter_mut().find(|(n, _)| n == name) {
            *existing = value;
            return;
        }
        let rank = known_rank(name);
        let pos = rank
            .and_then(|rank| {
                self.attrs
                    .iter()
                    .position(|(n, _)| known_rank(n).is_some_and(|r| r > rank))
            })
            .unwrap_or(self.attrs.len());
        self.attrs.insert(pos, (name.to_string(), value));
    }

    /// Set `ID` in place, or append it when absent. Rejects an empty id.
    ///
    /// # Errors
    ///
    /// [`DateRangeError::EmptyId`] for an empty id.
    pub fn set_id(&mut self, id: impl Into<String>) -> Result<()> {
        let id = id.into();
        if id.is_empty() {
            return Err(invalid("", DateRangeError::EmptyId));
        }
        self.replace_or_append("ID", AttrValue::quoted(id)?);
        Ok(())
    }

    /// Builder form of [`Self::set_id`].
    pub fn with_id(mut self, id: impl Into<String>) -> Result<Self> {
        self.set_id(id)?;
        Ok(self)
    }

    /// Set or clear `CLASS`.
    pub fn set_class(&mut self, class: Option<impl Into<String>>) -> Result<()> {
        match class {
            Some(class) => self.replace_or_append("CLASS", AttrValue::quoted(class.into())?),
            None => self.remove("CLASS"),
        }
        Ok(())
    }

    /// Builder form of [`Self::set_class`], inserted in canonical order.
    pub fn with_class(mut self, class: impl Into<String>) -> Result<Self> {
        self.insert_known("CLASS", AttrValue::quoted(class.into())?);
        Ok(self)
    }

    /// Set or clear `START-DATE`.
    pub fn set_start(&mut self, start: Option<ProgramDateTime>) {
        match start {
            Some(start) => {
                self.replace_or_append("START-DATE", AttrValue::quoted_unchecked(start.render()));
            }
            None => self.remove("START-DATE"),
        }
    }

    /// Builder form of [`Self::set_start`], inserted in canonical order.
    #[must_use]
    pub fn with_start(mut self, start: ProgramDateTime) -> Self {
        self.insert_known("START-DATE", AttrValue::quoted_unchecked(start.render()));
        self
    }

    /// Set or clear `END-DATE`.
    pub fn set_end(&mut self, end: Option<ProgramDateTime>) {
        match end {
            Some(end) => {
                self.replace_or_append("END-DATE", AttrValue::quoted_unchecked(end.render()));
            }
            None => self.remove("END-DATE"),
        }
    }

    /// Builder form of [`Self::set_end`], inserted in canonical order.
    #[must_use]
    pub fn with_end(mut self, end: ProgramDateTime) -> Self {
        self.insert_known("END-DATE", AttrValue::quoted_unchecked(end.render()));
        self
    }

    /// Set or clear `DURATION`.
    pub fn set_duration(&mut self, duration: Option<crate::DecimalSeconds>) {
        match duration {
            Some(duration) => {
                self.replace_or_append(
                    "DURATION",
                    AttrValue::bare_unchecked(super::format_secs(duration)),
                );
            }
            None => self.remove("DURATION"),
        }
    }

    /// Builder form of [`Self::set_duration`], inserted in canonical order.
    #[must_use]
    pub fn with_duration(mut self, duration: crate::DecimalSeconds) -> Self {
        self.insert_known(
            "DURATION",
            AttrValue::bare_unchecked(super::format_secs(duration)),
        );
        self
    }

    /// Set or clear `PLANNED-DURATION`.
    pub fn set_planned_duration(&mut self, planned: Option<crate::DecimalSeconds>) {
        match planned {
            Some(planned) => {
                self.replace_or_append(
                    "PLANNED-DURATION",
                    AttrValue::bare_unchecked(super::format_secs(planned)),
                );
            }
            None => self.remove("PLANNED-DURATION"),
        }
    }

    /// Builder form of [`Self::set_planned_duration`], inserted in canonical
    /// order.
    #[must_use]
    pub fn with_planned_duration(mut self, planned: crate::DecimalSeconds) -> Self {
        self.insert_known(
            "PLANNED-DURATION",
            AttrValue::bare_unchecked(super::format_secs(planned)),
        );
        self
    }

    /// Set `END-ON-NEXT`.
    pub fn set_end_on_next(&mut self, end_on_next: bool) {
        if end_on_next {
            self.replace_or_append("END-ON-NEXT", AttrValue::bare_unchecked("YES"));
        } else {
            self.remove("END-ON-NEXT");
        }
    }

    /// Builder form of [`Self::set_end_on_next`], inserted in canonical order.
    #[must_use]
    pub fn with_end_on_next(mut self, end_on_next: bool) -> Self {
        if end_on_next {
            self.insert_known("END-ON-NEXT", AttrValue::bare_unchecked("YES"));
        }
        self
    }

    /// Set or clear `CUE`.
    pub fn set_cue(&mut self, cue: Option<Cue>) {
        match cue {
            Some(cue) => {
                self.replace_or_append("CUE", AttrValue::quoted_unchecked(cue.render()));
            }
            None => self.remove("CUE"),
        }
    }

    /// Builder form of [`Self::set_cue`], inserted in canonical order.
    #[must_use]
    pub fn with_cue(mut self, cue: Cue) -> Self {
        self.insert_known("CUE", AttrValue::quoted_unchecked(cue.render()));
        self
    }

    /// Set a client (`X-`) attribute, replacing an existing value in place or
    /// appending a new one. The value must be a quoted-string,
    /// hexadecimal-sequence or unsigned decimal-floating-point.
    ///
    /// # Errors
    ///
    /// [`DateRangeError::InvalidClientAttributeValue`] for any other kind.
    pub fn set_client_attr(&mut self, name: ClientAttrName, value: AttrValue) -> Result<()> {
        if !client_value_ok(&value) {
            return Err(invalid(
                self.id(),
                DateRangeError::InvalidClientAttributeValue,
            ));
        }
        self.replace_or_append(name.as_str(), value);
        Ok(())
    }

    /// Builder form of [`Self::set_client_attr`].
    pub fn with_client_attr(mut self, name: ClientAttrName, value: AttrValue) -> Result<Self> {
        self.set_client_attr(name, value)?;
        Ok(self)
    }

    /// Remove a client (`X-`) attribute.
    pub fn remove_client_attr(&mut self, name: &ClientAttrName) {
        self.remove(name.as_str());
    }
}

impl PartialEq for DateRange {
    /// Semantic equality: the decoded typed values, ignoring attribute order
    /// and decimal/date spelling.
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
            && self.class() == other.class()
            && self.start() == other.start()
            && self.end() == other.end()
            && self.duration() == other.duration()
            && self.planned_duration() == other.planned_duration()
            && self.end_on_next() == other.end_on_next()
            && self.cue() == other.cue()
            && client_semantics(self) == client_semantics(other)
            && extra_semantics(self) == extra_semantics(other)
    }
}

impl Eq for DateRange {}

fn known_rank(name: &str) -> Option<usize> {
    KNOWN_ORDER.iter().position(|n| *n == name)
}

fn client_semantics(range: &DateRange) -> Vec<(String, bool, String)> {
    let mut v: Vec<(String, bool, String)> = range
        .client_attrs()
        .into_iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.is_quoted(),
                value.as_str().to_string(),
            )
        })
        .collect();
    v.sort();
    v
}

fn extra_semantics(range: &DateRange) -> Vec<(String, bool, String)> {
    let mut v: Vec<(String, bool, String)> = range
        .extra_attrs()
        .into_iter()
        .map(|(name, value)| (name, value.is_quoted(), value.as_str().to_string()))
        .collect();
    v.sort();
    v
}

impl DateRange {
    /// Parse a complete `#EXT-X-DATERANGE:<attribute-list>` line.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidAttributeList`] for text outside the §4.2 grammar;
    /// [`Error::InvalidClientAttrName`] for an illegal `X-` name;
    /// [`Error::InvalidDateRange`] for an empty `ID`, a known attribute with
    /// the wrong value kind, or a rule §4.4.5.1 states about this tag alone;
    /// [`Error::HlsParse`] for a missing `ID` or an unparsable date.
    pub fn parse_tag_line(line: &str) -> Result<Self> {
        let rest = line.strip_prefix(TAG).ok_or_else(|| Error::HlsParse {
            line_no: 0,
            line: line.to_string(),
            reason: "not an EXT-X-DATERANGE tag line".to_string(),
        })?;
        let attrs = super::scan_attribute_list_wire(rest)?;

        let id_value = attrs
            .iter()
            .find(|(name, _)| name == "ID")
            .map(|(_, value)| value)
            .ok_or_else(|| Error::HlsParse {
                line_no: 0,
                line: line.to_string(),
                reason: "EXT-X-DATERANGE missing required ID attribute".to_string(),
            })?;
        if !id_value.is_quoted() {
            return Err(invalid("", DateRangeError::WrongAttributeKind));
        }
        let id = id_value.as_str();
        if id.is_empty() {
            return Err(invalid("", DateRangeError::EmptyId));
        }

        for (name, value) in &attrs {
            match name.as_str() {
                "ID" => {}
                "CLASS" => {
                    quoted_value(value, id)?;
                }
                "START-DATE" => {
                    parse_date_attr(value, id, "START-DATE")?;
                }
                "END-DATE" => {
                    parse_date_attr(value, id, "END-DATE")?;
                }
                "DURATION" => {
                    duration_attr(value, id)?;
                }
                "PLANNED-DURATION" => {
                    duration_attr(value, id)?;
                }
                "END-ON-NEXT" if value.is_quoted() || value.as_str() != "YES" => {
                    return Err(invalid(id, DateRangeError::EndOnNextNotYes));
                }
                "CUE" => {
                    if !value.is_quoted() {
                        return Err(invalid(id, DateRangeError::WrongAttributeKind));
                    }
                    Cue::parse(value.as_str()).map_err(|e| with_id(e, id))?;
                }
                name if name.starts_with("X-") => {
                    if !client_value_ok(value) {
                        return Err(invalid(id, DateRangeError::InvalidClientAttributeValue));
                    }
                    ClientAttrName::new(name)?;
                }
                "SCTE35-CMD" | "SCTE35-OUT" | "SCTE35-IN"
                    if value.is_quoted() || !super::is_hexadecimal_sequence(value.as_str()) =>
                {
                    return Err(invalid(id, DateRangeError::InvalidScte35Value));
                }
                _ => {}
            }
        }
        let range = Self { attrs };
        range.validate()?;
        Ok(range)
    }

    /// Render the tag as one `#EXT-X-DATERANGE:<attribute-list>` line, without
    /// a trailing newline. An untouched parsed tag reproduces its accepted
    /// line byte-for-byte; a setter's change is rendered where that attribute
    /// already sits (or at the end when it was absent).
    ///
    /// # Errors
    ///
    /// [`DateRangeError`] for a rule §4.4.5.1 states about this tag alone;
    /// [`Error::InvalidQuotedString`] for an `ID`, `CLASS` or `CUE` that
    /// cannot be a quoted-string; [`Error::InvalidAttributeName`] for an
    /// unmodelled attribute name that is not a legal `AttributeName`.
    pub fn to_tag_line(&self) -> Result<String> {
        self.validate()?;
        let mut s = String::from(TAG);
        let mut list = String::new();
        render_attribute_list(&mut list, &self.attrs);
        s.push_str(list.strip_prefix(',').unwrap_or(&list));
        Ok(s)
    }

    fn validate(&self) -> Result<()> {
        let id = self.id();
        if id.is_empty() {
            return Err(invalid("", DateRangeError::EmptyId));
        }
        if self.end_on_next() {
            if self.class().is_none() {
                return Err(invalid(id, DateRangeError::EndOnNextWithoutClass));
            }
            if self.duration().is_some() {
                return Err(invalid(id, DateRangeError::EndOnNextWithDuration));
            }
            if self.end().is_some() {
                return Err(invalid(id, DateRangeError::EndOnNextWithEndDate));
            }
        }
        if let (Some(start), Some(end)) = (self.start(), self.end())
            && end.timestamp() < start.timestamp()
        {
            return Err(invalid(id, DateRangeError::EndBeforeStart));
        }
        if let (Some(start), Some(end), Some(duration)) =
            (self.start(), self.end(), self.duration())
        {
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
                return Err(invalid(id, DateRangeError::EndDateDurationMismatch));
            }
        }
        for (_, value) in self.client_attrs() {
            if !client_value_ok(&value) {
                return Err(invalid(id, DateRangeError::InvalidClientAttributeValue));
            }
        }
        for (name, value) in self.extra_attrs() {
            if !is_attribute_name(&name) {
                return Err(Error::InvalidAttributeName { name });
            }
            if matches!(name.as_str(), "SCTE35-CMD" | "SCTE35-OUT" | "SCTE35-IN")
                && (value.is_quoted() || !super::is_hexadecimal_sequence(value.as_str()))
            {
                return Err(invalid(id, DateRangeError::InvalidScte35Value));
            }
        }
        Ok(())
    }
}

fn with_id(error: Error, id: &str) -> Error {
    match error {
        Error::InvalidDateRange { reason, .. } => invalid(id, reason),
        other => other,
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
    decode_duration(value.as_str()).ok_or_else(|| invalid(id, DateRangeError::InvalidDurationValue))
}

fn decode_duration(value: &str) -> Option<crate::DecimalSeconds> {
    crate::DecimalSeconds::new(value.parse::<f64>().ok()?).ok()
}

fn client_value_ok(value: &AttrValue) -> bool {
    value.is_quoted()
        || super::is_hexadecimal_sequence(value.as_str())
        || super::is_valid_decimal_token(value.as_str(), false)
}

/// RFC 8216bis §4.2: an `AttributeName` contains only `[A-Z]`, `[0-9]` and `-`.
pub(crate) fn is_attribute_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
}
