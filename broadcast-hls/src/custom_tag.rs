//! A vendor tag built and read by broadcast-hls (RFC 8216bis §4.1, §4.2:
//! `private/specs-verified/hls/ietf/rfc8216bis-playlists.md`; §6.3.1:
//! `private/specs-verified/hls/ietf/rfc8216bis-client-requirements.md`).
//!
//! RFC 8216bis defines no registry for vendor tags. It requires every line
//! beginning `#EXT` to be a tag and requires clients to ignore tags they do
//! not recognise, which is what makes a vendor tag safe in a playlist a
//! standard player reads. [`CustomTagName`] validates the `EXT-` prefix and
//! the tag character set; [`CustomTag`] carries the attribute list and renders
//! it through the crate's shared lexer/renderer rather than by hand.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::{AttrValue, CustomTagNameError, Error, Result, render_attribute_list};

/// Standard tags this crate parses itself; a [`CustomTagName`] may not shadow
/// one.
const STANDARD_TAGS: &[&str] = &[
    "EXTM3U",
    "EXTINF",
    "EXT-X-VERSION",
    "EXT-X-TARGETDURATION",
    "EXT-X-MEDIA-SEQUENCE",
    "EXT-X-DISCONTINUITY-SEQUENCE",
    "EXT-X-I-FRAMES-ONLY",
    "EXT-X-INDEPENDENT-SEGMENTS",
    "EXT-X-START",
    "EXT-X-DEFINE",
    "EXT-X-PLAYLIST-TYPE",
    "EXT-X-ENDLIST",
    "EXT-X-DISCONTINUITY",
    "EXT-X-GAP",
    "EXT-X-BITRATE",
    "EXT-X-BYTERANGE",
    "EXT-X-MAP",
    "EXT-X-PART-INF",
    "EXT-X-SERVER-CONTROL",
    "EXT-X-PART",
    "EXT-X-PRELOAD-HINT",
    "EXT-X-RENDITION-REPORT",
    "EXT-X-SKIP",
    "EXT-X-PROGRAM-DATE-TIME",
    "EXT-X-DATERANGE",
    "EXT-X-KEY",
    "EXT-X-MEDIA",
    "EXT-X-STREAM-INF",
    "EXT-X-I-FRAME-STREAM-INF",
    "EXT-X-SESSION-DATA",
    "EXT-X-SESSION-KEY",
    "EXT-X-CONTENT-STEERING",
];

fn is_standard_tag(name: &str) -> bool {
    STANDARD_TAGS.contains(&name)
}

/// A vendor tag name: `EXT-` followed by at least one name character from the
/// RFC 8216bis §4.1 tag set (`[A-Z]`, `[0-9]`, `-`), not ending in `-`, and
/// not one this crate parses as a standard tag. Without the leading `#`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CustomTagName(String);

impl CustomTagName {
    /// Validate `name` (the tag name without its leading `#`).
    pub fn new(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        let Some(rest) = name.strip_prefix("EXT-") else {
            return Err(Error::InvalidCustomTagName {
                name,
                reason: CustomTagNameError::MissingPrefix,
            });
        };
        let reason = if rest.is_empty() {
            Some(CustomTagNameError::EmptyName)
        } else if !rest
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
        {
            Some(CustomTagNameError::InvalidCharacter)
        } else if !rest
            .bytes()
            .any(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            Some(CustomTagNameError::EmptyName)
        } else if rest.ends_with('-') {
            Some(CustomTagNameError::TrailingHyphen)
        } else if is_standard_tag(&name) {
            Some(CustomTagNameError::StandardTag)
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(Error::InvalidCustomTagName { name, reason });
        }
        Ok(Self(name))
    }

    /// The name as it appears on the wire, without the leading `#`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for CustomTagName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A vendor tag line: a validated [`CustomTagName`] plus its attribute list.
///
/// Attribute order is the order supplied (and, after [`Self::parse_tag_line`],
/// the order it appeared on the wire), so a parse/render round trip does not
/// reorder a vendor's attribute list.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CustomTag {
    /// The tag name, without the leading `#`.
    pub name: CustomTagName,
    attrs: Vec<(String, AttrValue)>,
}

impl CustomTag {
    /// Start a tag with no attributes.
    #[must_use]
    pub fn new(name: CustomTagName) -> Self {
        Self {
            name,
            attrs: Vec::new(),
        }
    }

    /// The attribute list, in wire order.
    #[must_use]
    pub fn attrs(&self) -> &[(String, AttrValue)] {
        &self.attrs
    }

    /// Set an attribute, replacing an existing name in place or appending a
    /// new one. Enforces the wire scanner's rules: a legal uppercase
    /// `AttributeName`, no empty bare value and no control characters.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidAttributeName`] for an illegal name,
    /// [`Error::InvalidAttributeList`] for an empty bare value or a control
    /// character.
    pub fn set_attr(&mut self, name: impl Into<String>, value: AttrValue) -> Result<()> {
        let name = name.into();
        validate_attr(&name, &value)?;
        if let Some((_, existing)) = self.attrs.iter_mut().find(|(n, _)| *n == name) {
            *existing = value;
        } else {
            self.attrs.push((name, value));
        }
        Ok(())
    }

    /// Builder form of [`Self::set_attr`].
    pub fn with_attr(mut self, name: impl Into<String>, value: AttrValue) -> Result<Self> {
        self.set_attr(name, value)?;
        Ok(self)
    }

    /// Remove an attribute by name.
    pub fn remove_attr(&mut self, name: &str) {
        self.attrs.retain(|(n, _)| n != name);
    }

    /// Parse a complete `#EXT-...:<attribute-list>` line.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidCustomTagName`] for a name that is not an `EXT-` tag,
    /// [`Error::InvalidAttributeList`] for text outside the §4.2 grammar
    /// (including an empty list after `:`), [`Error::InvalidAttributeName`]
    /// for an attribute name that is not a legal §4.2 `AttributeName`, and
    /// [`Error::HlsParse`] for a line that is not a tag at all.
    pub fn parse_tag_line(line: &str) -> Result<Self> {
        let body = line.strip_prefix('#').ok_or_else(|| Error::HlsParse {
            line_no: 0,
            line: line.to_string(),
            reason: "not a tag line".to_string(),
        })?;
        let (name, rest) = match body.split_once(':') {
            Some((name, rest)) => (name, Some(rest)),
            None => (body, None),
        };
        let name = CustomTagName::new(name)?;
        let attrs = match rest {
            None => Vec::new(),
            Some(rest) => super::scan_attribute_list_wire(rest)?,
        };
        Ok(Self { name, attrs })
    }

    /// Render the tag as one `#EXT-...` line, without a trailing newline.
    /// A parsed tag reproduces its accepted line byte-for-byte.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidAttributeName`] when an attribute name is not a legal
    /// RFC 8216bis §4.2 `AttributeName`.
    pub fn to_tag_line(&self) -> Result<String> {
        let mut s = format!("#{}", self.name.as_str());
        if self.attrs.is_empty() {
            return Ok(s);
        }
        let mut seen = alloc::collections::BTreeSet::new();
        for (name, value) in &self.attrs {
            validate_attr(name, value)?;
            if !seen.insert(name.as_str()) {
                return Err(Error::DuplicateAttribute { name: name.clone() });
            }
        }
        s.push(':');
        let mut list = String::new();
        render_attribute_list(&mut list, &self.attrs);
        s.push_str(list.strip_prefix(',').unwrap_or(&list));
        Ok(s)
    }
}

fn validate_attr(name: &str, value: &AttrValue) -> Result<()> {
    if !crate::daterange::is_attribute_name(name) {
        return Err(Error::InvalidAttributeName {
            name: name.to_string(),
        });
    }
    if !value.is_quoted() && value.as_str().is_empty() {
        return Err(Error::InvalidAttributeList {
            list: String::from(name),
            reason: crate::AttributeListError::EmptyValue,
        });
    }
    if super::has_forbidden_control(value.as_str()) {
        return Err(Error::InvalidAttributeList {
            list: String::from(name),
            reason: crate::AttributeListError::ControlCharacter,
        });
    }
    Ok(())
}
