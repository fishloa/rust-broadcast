//! A vendor tag built and read by broadcast-hls (RFC 8216bis §4.1, §6.3.1).
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

use crate::{AttrValue, Error, Result, render_attribute_list};

/// A vendor tag name: `EXT-` followed by at least one character from the
/// RFC 8216bis §4.1 tag set (`[A-Z]`, `[0-9]`, `-`), without the leading `#`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CustomTagName(String);

impl CustomTagName {
    /// Validate `name` (the tag name without its leading `#`).
    pub fn new(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        if !name.starts_with("EXT-") {
            return Err(Error::InvalidCustomTagName {
                name,
                reason: "must start with the EXT- prefix",
            });
        }
        if name.len() == 4 {
            return Err(Error::InvalidCustomTagName {
                name,
                reason: "must name something after the EXT- prefix",
            });
        }
        if !name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(Error::InvalidCustomTagName {
                name,
                reason: "characters must be [A-Z], [0-9] or '-'",
            });
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
    /// The attribute list, in wire order.
    pub attrs: Vec<(String, AttrValue)>,
}

impl CustomTag {
    /// Parse a complete `#EXT-...:<attribute-list>` line.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidCustomTagName`] for a name that is not an `EXT-` tag,
    /// [`Error::InvalidAttributeName`] for an attribute name that is not a
    /// legal RFC 8216bis §4.2 `AttributeName`, and [`Error::HlsParse`] for a
    /// line that is not a tag at all.
    pub fn parse_tag_line(line: &str) -> Result<Self> {
        let body = line.strip_prefix('#').ok_or_else(|| Error::HlsParse {
            line_no: 0,
            line: line.to_string(),
            reason: "not a tag line".to_string(),
        })?;
        let (name, rest) = match body.split_once(':') {
            Some((name, rest)) => (name, rest),
            None => (body, ""),
        };
        let name = CustomTagName::new(name)?;
        let mut attrs = Vec::new();
        if !rest.is_empty() {
            let (values, quoted, order) = super::scan_attribute_list_ordered(rest, true)?;
            for key in order {
                if !crate::daterange::is_attribute_name(&key) {
                    return Err(Error::InvalidAttributeName { name: key });
                }
                let value = values.get(&key).cloned().unwrap_or_default();
                let value = if quoted.contains(&key) {
                    AttrValue::quoted(value)?
                } else {
                    AttrValue::bare(value)?
                };
                attrs.push((key, value));
            }
        }
        Ok(Self { name, attrs })
    }

    /// Render the tag as one `#EXT-...` line, without a trailing newline.
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
        s.push(':');
        for (name, _) in &self.attrs {
            if !crate::daterange::is_attribute_name(name) {
                return Err(Error::InvalidAttributeName { name: name.clone() });
            }
        }
        let mut list = String::new();
        render_attribute_list(&mut list, &self.attrs);
        s.push_str(list.strip_prefix(',').unwrap_or(&list));
        Ok(s)
    }
}
