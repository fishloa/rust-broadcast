//! XML 1.0 character-data validation over `quick-xml` events.
//!
//! Every crate that reads XML through `quick-xml` decodes character data here,
//! so a text run, CDATA section or entity/character reference is validated the
//! same way in a modelled element, between elements, and inside a subtree the
//! parser skips. Decoding is quick-xml's (`BytesText::xml10_content`,
//! `BytesRef::resolve_char_ref`, `escape::resolve_predefined_entity`); this
//! module enforces the XML 1.0 §2.2 `Char` production on the result.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use quick_xml::NsReader;
use quick_xml::XmlVersion;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{QName, ResolveResult};

/// Message for a character outside the XML 1.0 §2.2 `Char` production.
pub const BAD_CHAR: &str = "character not allowed in XML 1.0 (Char production, §2.2)";

/// XML 1.0 §2.2 `Char` production: the only characters a document may
/// contain, whether written literally or through a character reference.
pub fn is_xml_char(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\r'
            | '\u{20}'..='\u{D7FF}'
            | '\u{E000}'..='\u{FFFD}'
            | '\u{10000}'..='\u{10FFFF}'
    )
}

/// `Ok(Some(text))` for a text run, CDATA section or entity/character
/// reference (references resolved); `Ok(None)` for every other event; `Err`
/// with a description for an undefined entity, an invalid character reference,
/// or a character outside the `Char` production ([`BAD_CHAR`]).
pub fn char_data(event: &Event<'_>) -> Result<Option<String>, String> {
    let content = match event {
        Event::Text(t) => t.xml10_content().into_owned(),
        Event::CData(c) => c.xml10_content().into_owned(),
        Event::GeneralRef(r) => match r.resolve_char_ref() {
            Ok(Some(c)) => c.to_string(),
            Ok(None) => match resolve_predefined_entity(r) {
                Some(s) => s.to_string(),
                None => return Err(format!("undefined entity reference &{};", &**r)),
            },
            Err(e) => return Err(e.to_string()),
        },
        _ => return Ok(None),
    };
    if content.chars().all(is_xml_char) {
        Ok(Some(content))
    } else {
        Err(BAD_CHAR.to_string())
    }
}

/// The namespace URI an element or attribute name resolved to: `None` when
/// unbound, `Err` with a description for an undeclared prefix.
pub fn namespace_of(result: ResolveResult<'_>) -> Result<Option<String>, String> {
    match result {
        ResolveResult::Unbound => Ok(None),
        ResolveResult::Bound(ns) => Ok(Some(ns.into_inner().to_string())),
        ResolveResult::Unknown(prefix) => Err(format!("unknown namespace prefix '{prefix}'")),
    }
}

/// One attribute of a start tag, namespace declarations excluded.
#[derive(Debug)]
pub struct TagAttribute {
    /// Resolved namespace URI; `None` for an unprefixed attribute (which is
    /// never in the default namespace).
    pub namespace: Option<String>,
    /// Local name.
    pub name: String,
    /// Normalized, unescaped value, every character inside the `Char`
    /// production.
    pub value: String,
}

/// `message` followed by the reader's byte position, the text every XML error
/// of the pull parsers carries.
pub fn message_at(reader: &NsReader<&[u8]>, message: &str) -> String {
    format!("{message} at byte {}", reader.buffer_position())
}

/// The next event with its owned element-namespace resolution: the
/// namespace outcome is the [`namespace_of`] result, a reader failure is its
/// description.
pub fn read_event<'a>(
    reader: &mut NsReader<&'a [u8]>,
) -> Result<(Result<Option<String>, String>, Event<'a>), String> {
    match reader.read_resolved_event() {
        Ok((ns, event)) => Ok((namespace_of(ns), event)),
        Err(e) => Err(e.to_string()),
    }
}

/// The attributes of a start tag with namespace declarations skipped, values
/// normalized and validated against the `Char` production, and each name
/// resolved against `reader`'s current namespace scope. `fail` turns a
/// description into the caller's error.
pub fn read_attributes<E>(
    reader: &NsReader<&[u8]>,
    start: &BytesStart<'_>,
    fail: impl Fn(&str) -> E,
) -> Result<Vec<TagAttribute>, E> {
    read_attributes_checked(reader, start, fail, |_, _| Ok(()))
}

/// [`read_attributes`] with `admit(count, value)` run on every attribute after
/// it is normalized and before its characters are validated, so a caller can
/// enforce a count or size limit with its own error; `count` is the 1-based
/// position among the non-declaration attributes.
pub fn read_attributes_checked<E>(
    reader: &NsReader<&[u8]>,
    start: &BytesStart<'_>,
    fail: impl Fn(&str) -> E,
    mut admit: impl FnMut(usize, &str) -> Result<(), E>,
) -> Result<Vec<TagAttribute>, E> {
    let resolver = reader.resolver();
    let mut out = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|e| fail(&e.to_string()))?;
        let key: QName<'_> = attr.key;
        let is_declaration = match key.prefix() {
            None => key.as_ref() == "xmlns",
            Some(p) => p.is_xmlns(),
        };
        if is_declaration {
            continue;
        }
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|e| fail(&e.to_string()))?;
        admit(out.len() + 1, &value)?;
        if !value.chars().all(is_xml_char) {
            return Err(fail(BAD_CHAR));
        }
        let (ns, local) = resolver.resolve_attribute(key);
        out.push(TagAttribute {
            namespace: namespace_of(ns).map_err(|m| fail(&m))?,
            name: local.into_inner().to_string(),
            value: value.into_owned(),
        });
    }
    Ok(out)
}

/// Largest XML document the pull parsers accept, in bytes.
pub const MAX_XML_BYTES: usize = 4 * 1024 * 1024;
/// Deepest element nesting the pull parsers accept.
pub const MAX_XML_DEPTH: usize = 32;
/// Most elements (of any namespace) the pull parsers accept in one document.
pub const MAX_XML_ELEMENTS: usize = 100_000;
/// Longest single attribute value the pull parsers accept, in bytes.
pub const MAX_XML_ATTRIBUTE_BYTES: usize = 8 * 1024;
/// Most attributes the pull parsers accept on one element.
pub const MAX_XML_ATTRIBUTES: usize = 256;

/// Hostile-input bounds for a document read from a server the caller does not
/// control; `Default` is the `MAX_XML_*` constants.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct XmlLimits {
    /// See [`MAX_XML_BYTES`].
    pub max_bytes: usize,
    /// See [`MAX_XML_DEPTH`].
    pub max_depth: usize,
    /// See [`MAX_XML_ELEMENTS`].
    pub max_elements: usize,
    /// See [`MAX_XML_ATTRIBUTE_BYTES`].
    pub max_attribute_bytes: usize,
    /// See [`MAX_XML_ATTRIBUTES`].
    pub max_attributes: usize,
}

impl Default for XmlLimits {
    fn default() -> Self {
        Self {
            max_bytes: MAX_XML_BYTES,
            max_depth: MAX_XML_DEPTH,
            max_elements: MAX_XML_ELEMENTS,
            max_attribute_bytes: MAX_XML_ATTRIBUTE_BYTES,
            max_attributes: MAX_XML_ATTRIBUTES,
        }
    }
}

/// Which bound of [`XmlLimits`] was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum XmlLimit {
    /// Document size in bytes.
    InputBytes,
    /// Element nesting depth.
    Depth,
    /// Total element count.
    Elements,
    /// Length of one attribute value.
    AttributeBytes,
    /// Attributes on one element.
    Attributes,
}

impl XmlLimit {
    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::InputBytes => "input_bytes",
            Self::Depth => "depth",
            Self::Elements => "elements",
            Self::AttributeBytes => "attribute_bytes",
            Self::Attributes => "attributes",
        }
    }
}

crate::impl_spec_display!(XmlLimit);

/// A bound that was exceeded and its ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitExceeded {
    /// The bound that tripped.
    pub limit: XmlLimit,
    /// Its ceiling.
    pub max: usize,
}

/// Running tally of one document against its [`XmlLimits`].
#[derive(Debug, Clone)]
pub struct XmlBudget {
    limits: XmlLimits,
    elements: usize,
    depth: usize,
}

impl XmlBudget {
    /// A fresh tally for one document.
    #[must_use]
    pub fn new(limits: XmlLimits) -> Self {
        Self {
            limits,
            elements: 0,
            depth: 0,
        }
    }

    /// The bounds this tally enforces.
    #[must_use]
    pub fn limits(&self) -> &XmlLimits {
        &self.limits
    }

    /// Reject a document longer than [`XmlLimits::max_bytes`].
    pub fn check_input(&self, len: usize) -> Result<(), LimitExceeded> {
        if len > self.limits.max_bytes {
            return Err(LimitExceeded {
                limit: XmlLimit::InputBytes,
                max: self.limits.max_bytes,
            });
        }
        Ok(())
    }

    /// Count a start tag (its end, for an empty element, is
    /// [`close_element`](Self::close_element)); fails past the element-count
    /// or depth ceiling.
    pub fn open_element(&mut self) -> Result<(), LimitExceeded> {
        self.elements += 1;
        if self.elements > self.limits.max_elements {
            return Err(LimitExceeded {
                limit: XmlLimit::Elements,
                max: self.limits.max_elements,
            });
        }
        self.depth += 1;
        if self.depth > self.limits.max_depth {
            return Err(LimitExceeded {
                limit: XmlLimit::Depth,
                max: self.limits.max_depth,
            });
        }
        Ok(())
    }

    /// Count an end tag.
    pub fn close_element(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Check the `count`-th (1-based) attribute of a start tag, whose
    /// normalized value is `value`.
    pub fn admit_attribute(&self, count: usize, value: &str) -> Result<(), LimitExceeded> {
        if count > self.limits.max_attributes {
            return Err(LimitExceeded {
                limit: XmlLimit::Attributes,
                max: self.limits.max_attributes,
            });
        }
        if value.len() > self.limits.max_attribute_bytes {
            return Err(LimitExceeded {
                limit: XmlLimit::AttributeBytes,
                max: self.limits.max_attribute_bytes,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_char_data(xml: &str) -> Result<Option<String>, String> {
        let mut reader = NsReader::from_str(xml);
        loop {
            let event = reader.read_event().map_err(|e| e.to_string())?;
            if matches!(event, Event::Eof) {
                return Ok(None);
            }
            if let Some(text) = char_data(&event)? {
                return Ok(Some(text));
            }
        }
    }

    #[test]
    fn char_production_boundaries() {
        assert!(is_xml_char('\t') && is_xml_char('\u{10FFFF}'));
        assert!(!is_xml_char('\u{0}') && !is_xml_char('\u{FFFE}') && !is_xml_char('\u{B}'));
    }

    #[test]
    fn resolves_predefined_and_numeric_references() {
        assert_eq!(
            first_char_data("<a>&amp;</a>").unwrap().as_deref(),
            Some("&")
        );
        assert_eq!(
            first_char_data("<a>&#x41;</a>").unwrap().as_deref(),
            Some("A")
        );
    }

    #[test]
    fn rejects_undefined_entity_and_forbidden_character_reference() {
        assert!(
            first_char_data("<a>&nope;</a>")
                .unwrap_err()
                .contains("undefined entity")
        );
        assert_eq!(first_char_data("<a>&#1;</a>").unwrap_err(), BAD_CHAR);
    }

    #[test]
    fn cdata_and_text_pass_through() {
        assert_eq!(
            first_char_data("<a><![CDATA[x<y]]></a>")
                .unwrap()
                .as_deref(),
            Some("x<y")
        );
    }

    #[test]
    fn unknown_prefix_is_an_error() {
        let mut reader = NsReader::from_str("<p:a/>");
        let (ns, _) = reader.read_resolved_event().unwrap();
        assert!(
            namespace_of(ns)
                .unwrap_err()
                .contains("unknown namespace prefix")
        );
    }

    fn start_attributes(
        xml: &str,
        admit: impl FnMut(usize, &str) -> Result<(), String>,
    ) -> Result<Vec<TagAttribute>, String> {
        let mut reader = NsReader::from_str(xml);
        loop {
            let (_, event) = read_event(&mut reader)?;
            if let Event::Start(e) | Event::Empty(e) = event {
                return read_attributes_checked(&reader, &e, |m| message_at(&reader, m), admit);
            }
        }
    }

    #[test]
    fn attributes_skip_declarations_and_resolve_namespaces() {
        let attrs = start_attributes(
            r#"<a xmlns="urn:d" xmlns:p="urn:p" plain="1" p:q="2"/>"#,
            |_, _| Ok(()),
        )
        .unwrap();
        assert_eq!(attrs.len(), 2);
        assert_eq!(
            (
                attrs[0].namespace.as_deref(),
                attrs[0].name.as_str(),
                attrs[0].value.as_str()
            ),
            (None, "plain", "1")
        );
        assert_eq!(
            (
                attrs[1].namespace.as_deref(),
                attrs[1].name.as_str(),
                attrs[1].value.as_str()
            ),
            (Some("urn:p"), "q", "2")
        );
    }

    #[test]
    fn attribute_values_are_normalized_and_char_checked() {
        let attrs = start_attributes("<a v=\"x&#10;y&amp;z\"/>", |_, _| Ok(())).unwrap();
        assert_eq!(attrs[0].value, "x\ny&z");
        let literal = start_attributes("<a v=\"a\nb\"/>", |_, _| Ok(())).unwrap();
        assert_eq!(literal[0].value, "a b");
        let err = start_attributes("<a v=\"&#1;\"/>", |_, _| Ok(())).unwrap_err();
        assert!(
            err.starts_with(BAD_CHAR) && err.contains("at byte"),
            "{err}"
        );
    }

    #[test]
    fn admit_sees_the_one_based_count_and_can_veto() {
        let mut seen = Vec::new();
        start_attributes("<a xmlns:p=\"urn:p\" x=\"1\" y=\"22\"/>", |n, v| {
            seen.push((n, v.to_string()));
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, [(1, "1".to_string()), (2, "22".to_string())]);
        let err = start_attributes("<a x=\"1\" y=\"22\"/>", |n, _| {
            if n > 1 {
                Err("too many".to_string())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(err, "too many");
    }

    #[test]
    fn unknown_attribute_prefix_is_an_error() {
        let err = start_attributes("<a q:x=\"1\"/>", |_, _| Ok(())).unwrap_err();
        assert!(err.contains("unknown namespace prefix"), "{err}");
    }

    #[test]
    fn budget_trips_each_bound() {
        let limits = XmlLimits {
            max_bytes: 4,
            max_depth: 2,
            max_elements: 4,
            max_attribute_bytes: 2,
            max_attributes: 1,
        };
        let mut b = XmlBudget::new(limits);
        assert_eq!(b.check_input(5).unwrap_err().limit, XmlLimit::InputBytes);
        b.check_input(4).unwrap();
        b.open_element().unwrap();
        b.open_element().unwrap();
        assert_eq!(b.open_element().unwrap_err().limit, XmlLimit::Depth);
        b.close_element();
        b.close_element();
        b.close_element();
        b.open_element().unwrap();
        assert_eq!(b.open_element().unwrap_err().limit, XmlLimit::Elements);
        assert_eq!(
            b.admit_attribute(2, "").unwrap_err(),
            LimitExceeded {
                limit: XmlLimit::Attributes,
                max: 1
            }
        );
        assert_eq!(
            b.admit_attribute(1, "abc").unwrap_err().limit,
            XmlLimit::AttributeBytes
        );
        b.admit_attribute(1, "ab").unwrap();
    }
}
