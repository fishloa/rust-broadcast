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
use quick_xml::Writer;
use quick_xml::XmlVersion;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::name::{QName, ResolveResult};

/// Message for a character outside the XML 1.0 §2.2 `Char` production.
pub const BAD_CHAR: &str = "character not allowed in XML 1.0 (Char production, §2.2)";

/// Why [`char_data`], [`namespace_of`] or [`read_event`] refused part of a
/// document.
///
/// A caller matches the failure kind instead of parsing the description the
/// pull parsers put in their own error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum XmlCharError {
    /// A character outside the XML 1.0 §2.2 `Char` production.
    #[error("{}", BAD_CHAR)]
    BadChar,
    /// A reference to an entity that is neither predefined nor declared (no
    /// document may declare one: DOCTYPE is refused).
    #[error("undefined entity reference &{name};")]
    UndefinedEntity {
        /// The entity name, without the `&` and `;`.
        name: String,
    },
    /// An element or attribute name with a namespace prefix that is never
    /// declared.
    #[error("unknown namespace prefix '{prefix}'")]
    UnknownPrefix {
        /// The prefix, without the `:`.
        prefix: String,
    },
    /// quick-xml's own lexer refused the bytes (malformed markup, an invalid
    /// character reference, an encoding error).
    #[error("{message}")]
    Reader {
        /// The reader's description of the fault.
        message: String,
    },
}

/// XML 1.0 §2.2 `Char` production ranges.
const CHAR_RANGE_1_START: char = '\u{20}';
const CHAR_RANGE_1_END: char = '\u{D7FF}';
const CHAR_RANGE_2_START: char = '\u{E000}';
const CHAR_RANGE_2_END: char = '\u{FFFD}';
const CHAR_RANGE_3_START: char = '\u{10000}';
const CHAR_RANGE_3_END: char = '\u{10FFFF}';
/// Indentation width of the shared XML writer, in spaces.
const INDENT_SPACES: usize = 2;

/// XML 1.0 §2.2 `Char` production: the only characters a document may
/// contain, whether written literally or through a character reference.
pub fn is_xml_char(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\r'
            | CHAR_RANGE_1_START..=CHAR_RANGE_1_END
            | CHAR_RANGE_2_START..=CHAR_RANGE_2_END
            | CHAR_RANGE_3_START..=CHAR_RANGE_3_END
    )
}

/// `Ok(Some(text))` for a text run, CDATA section or entity/character
/// reference (references resolved); `Ok(None)` for every other event;
/// `Err` [`XmlCharError::UndefinedEntity`] for an undefined entity,
/// [`XmlCharError::Reader`] for an invalid character reference, and
/// [`XmlCharError::BadChar`] for a character outside the `Char` production.
pub fn char_data(event: &Event<'_>) -> Result<Option<String>, XmlCharError> {
    let content = match event {
        Event::Text(t) => t.xml10_content().into_owned(),
        Event::CData(c) => c.xml10_content().into_owned(),
        Event::GeneralRef(r) => match r.resolve_char_ref() {
            Ok(Some(c)) => c.to_string(),
            Ok(None) => match resolve_predefined_entity(r) {
                Some(s) => s.to_string(),
                None => {
                    return Err(XmlCharError::UndefinedEntity {
                        name: (**r).to_string(),
                    });
                }
            },
            Err(e) => {
                return Err(XmlCharError::Reader {
                    message: e.to_string(),
                });
            }
        },
        _ => return Ok(None),
    };
    if content.chars().all(is_xml_char) {
        Ok(Some(content))
    } else {
        Err(XmlCharError::BadChar)
    }
}

/// The namespace URI an element or attribute name resolved to: `None` when
/// unbound, `Err` [`XmlCharError::UnknownPrefix`] for an undeclared prefix.
pub fn namespace_of(result: ResolveResult<'_>) -> Result<Option<String>, XmlCharError> {
    match result {
        ResolveResult::Unbound => Ok(None),
        ResolveResult::Bound(ns) => Ok(Some(ns.into_inner().to_string())),
        ResolveResult::Unknown(prefix) => Err(XmlCharError::UnknownPrefix {
            prefix: prefix.to_string(),
        }),
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
/// namespace outcome is the [`namespace_of`] result, a reader failure is
/// [`XmlCharError::Reader`].
pub fn read_event<'a>(
    reader: &mut NsReader<&'a [u8]>,
) -> Result<(Result<Option<String>, XmlCharError>, Event<'a>), XmlCharError> {
    match reader.read_resolved_event() {
        Ok((ns, event)) => Ok((namespace_of(ns), event)),
        Err(e) => Err(XmlCharError::Reader {
            message: e.to_string(),
        }),
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
            namespace: namespace_of(ns).map_err(|e| fail(&e.to_string()))?,
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

broadcast_core::impl_spec_display!(XmlLimit);

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

/// The in-memory writer the manifest and document writers build into.
pub type XmlWriter = Writer<Vec<u8>>;

/// A two-space-indenting [`XmlWriter`]. Every attribute value and text node
/// written through the helpers below is escaped by quick-xml.
#[must_use]
pub fn new_writer() -> XmlWriter {
    Writer::new_with_indent(Vec::new(), b' ', INDENT_SPACES)
}

/// A write into an [`XmlWriter`]'s outcome.
pub type WriteOutcome = core::result::Result<(), WriteRefusal>;

/// Why a [`XmlWriter`] helper refused to write: its own message for a write
/// the writer rejects, or a tag attribute value that is not valid UTF-8.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WriteRefusal {
    /// The writer refused the event; the string is the writer's message.
    Write(String),
    /// [`open_element`] or [`empty_element`] was given an attribute value
    /// that is not valid UTF-8.
    ValueNotUtf8,
    /// An attribute value or a text node holds a character outside the XML 1.0
    /// §2.2 `Char` production, which `quick-xml` escapes neither (it escapes
    /// only `<`, `>`, `&`, `'` and `"`) and no reader accepts.
    InvalidChar,
}

impl WriteRefusal {
    /// The failure's label — the guard's `name()`/`Display` pair for a
    /// crate error enum.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            WriteRefusal::Write(_) => "write",
            WriteRefusal::ValueNotUtf8 => "value not UTF-8",
            WriteRefusal::InvalidChar => BAD_CHAR,
        }
    }
}

broadcast_core::impl_spec_display!(WriteRefusal);

impl core::error::Error for WriteRefusal {}

fn write_event(w: &mut XmlWriter, event: Event<'_>) -> WriteOutcome {
    w.write_event(event)
        .map_err(|e| WriteRefusal::Write(e.to_string()))
}

/// `<?xml version="1.0" encoding="{encoding}"?>`.
pub fn write_declaration(w: &mut XmlWriter, encoding: &str) -> WriteOutcome {
    write_event(w, Event::Decl(BytesDecl::new("1.0", Some(encoding), None)))
}

/// The attribute list of an element with none.
pub const NO_ATTRS: &[(&str, &str)] = &[];

/// A start tag for `name` carrying `attrs` in order (values escaped when
/// written).
#[must_use]
pub fn start_tag<'a, V: AsRef<str>>(name: &'a str, attrs: &'a [(&str, V)]) -> BytesStart<'a> {
    let mut start = BytesStart::new(name);
    for (key, value) in attrs {
        start.push_attribute((*key, value.as_ref()));
    }
    start
}

/// `<name a="b">`.
///
/// # Errors
/// [`WriteRefusal::ValueNotUtf8`] if an attribute value is not UTF-8, or
/// [`WriteRefusal::Write`] if the writer refuses the event.
pub fn open_element<V: AsRef<str>>(
    w: &mut XmlWriter,
    name: &str,
    attrs: &[(&str, V)],
) -> WriteOutcome {
    let start = start_tag(name, attrs);
    check_values(&start)?;
    write_event(w, Event::Start(start))
}

/// `<name a="b"/>`.
///
/// # Errors
/// As [`open_element`].
pub fn empty_element<V: AsRef<str>>(
    w: &mut XmlWriter,
    name: &str,
    attrs: &[(&str, V)],
) -> WriteOutcome {
    let start = start_tag(name, attrs);
    check_values(&start)?;
    write_event(w, Event::Empty(start))
}

/// Every attribute value of `start` is valid UTF-8 and inside the XML 1.0
/// §2.2 `Char` production.
fn check_values(start: &BytesStart<'_>) -> WriteOutcome {
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|e| WriteRefusal::Write(e.to_string()))?;
        let value = attribute
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|_| WriteRefusal::ValueNotUtf8)?;
        if !value.chars().all(is_xml_char) {
            return Err(WriteRefusal::InvalidChar);
        }
    }
    Ok(())
}

/// `</name>`.
///
/// # Errors
/// [`WriteRefusal::Write`] if the writer refuses the event.
pub fn close_element(w: &mut XmlWriter, name: &str) -> WriteOutcome {
    write_event(w, Event::End(BytesEnd::new(name)))
}

/// Escaped character data.
///
/// # Errors
/// [`WriteRefusal::InvalidChar`] if `text` holds a character outside the XML 1.0
/// §2.2 `Char` production, or [`WriteRefusal::Write`] if the writer refuses the
/// event.
pub fn write_text(w: &mut XmlWriter, text: &str) -> WriteOutcome {
    if !text.chars().all(is_xml_char) {
        return Err(WriteRefusal::InvalidChar);
    }
    write_event(w, Event::Text(BytesText::new(text)))
}

/// A leaf element with escaped text content on one line (`<name>text</name>`).
///
/// # Errors
/// As [`open_element`].
pub fn text_element(w: &mut XmlWriter, name: &str, text: &str) -> WriteOutcome {
    open_element(w, name, NO_ATTRS)?;
    write_text(w, text)?;
    close_element(w, name)
}

/// The written document plus a trailing newline.
#[must_use]
pub fn finish_document(w: XmlWriter) -> String {
    let mut out = String::from_utf8_lossy(&w.into_inner()).into_owned();
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_char_data(xml: &str) -> Result<Option<String>, XmlCharError> {
        let mut reader = NsReader::from_str(xml);
        loop {
            let event = reader.read_event().map_err(|e| XmlCharError::Reader {
                message: e.to_string(),
            })?;
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
        assert_eq!(
            first_char_data("<a>&nope;</a>").unwrap_err(),
            XmlCharError::UndefinedEntity {
                name: String::from("nope")
            }
        );
        assert_eq!(
            first_char_data("<a>&#1;</a>").unwrap_err(),
            XmlCharError::BadChar
        );
        // The description is unchanged, so a log line still reads the same.
        assert_eq!(
            XmlCharError::UndefinedEntity {
                name: String::from("nope")
            }
            .to_string(),
            "undefined entity reference &nope;"
        );
        assert_eq!(XmlCharError::BadChar.to_string(), BAD_CHAR);
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
        assert_eq!(
            namespace_of(ns).unwrap_err(),
            XmlCharError::UnknownPrefix {
                prefix: String::from("p")
            }
        );
        // A caller can match the failure kind without reading the text.
        assert_eq!(
            XmlCharError::UnknownPrefix {
                prefix: String::from("p")
            }
            .to_string(),
            "unknown namespace prefix 'p'"
        );
    }

    /// Wrapper for the reader-failure variant.
    #[test]
    fn reader_failure_keeps_its_description() {
        let err = first_char_data("<a>&#xZZ;</a>").unwrap_err();
        assert!(matches!(err, XmlCharError::Reader { .. }), "{err:?}");
    }

    #[test]
    fn read_attributes_resolves_each_prefix_and_skips_the_declarations() {
        let xml = r#"<a xmlns:p="urn:p" x="1" p:y="2"></a>"#;
        let mut reader = NsReader::from_str(xml);
        let (_, event) = reader.read_resolved_event().unwrap();
        let Event::Start(start) = event else {
            panic!("the first event is a start tag");
        };
        let attributes = read_attributes(&reader, &start, |message| message.to_string()).unwrap();
        assert_eq!(
            attributes.len(),
            2,
            "the xmlns declaration is not an attribute"
        );
        assert_eq!(attributes[0].namespace, None);
        assert_eq!(attributes[0].name, "x");
        assert_eq!(attributes[0].value, "1");
        assert_eq!(attributes[1].namespace.as_deref(), Some("urn:p"));
        assert_eq!(attributes[1].name, "y");
        assert_eq!(attributes[1].value, "2");
    }

    #[test]
    fn read_attributes_reports_a_value_outside_the_char_production() {
        let xml = "<a x=\"\u{1}\"></a>";
        let mut reader = NsReader::from_str(xml);
        let (_, event) = reader.read_resolved_event().unwrap();
        let Event::Start(start) = event else {
            panic!("the first event is a start tag");
        };
        assert_eq!(
            read_attributes(&reader, &start, |_| String::from("refused")).unwrap_err(),
            "refused"
        );
    }

    fn start_attributes(
        xml: &str,
        admit: impl FnMut(usize, &str) -> Result<(), String>,
    ) -> Result<Vec<TagAttribute>, String> {
        let mut reader = NsReader::from_str(xml);
        loop {
            let (_, event) = read_event(&mut reader).map_err(|e| e.to_string())?;
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

    #[test]
    fn writer_helpers_escape_and_indent() {
        let mut w = new_writer();
        write_declaration(&mut w, "utf-8").unwrap();
        open_element(&mut w, "R", &[("k", "a&b\"c".to_string())]).unwrap();
        empty_element(&mut w, "E", &[("v", "<")]).unwrap();
        text_element(&mut w, "T", "x<y").unwrap();
        close_element(&mut w, "R").unwrap();
        assert_eq!(
            finish_document(w),
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<R k=\"a&amp;b&quot;c\">\n  <E v=\"&lt;\"/>\n  <T>x&lt;y</T>\n</R>\n"
        );
    }

    #[test]
    fn writer_helpers_refuse_a_character_outside_the_char_production() {
        for illegal in [
            "\u{0}", "\u{1}", "\u{8}", "\u{B}", "\u{C}", "\u{1F}", "\u{FFFE}",
        ] {
            let mut w = new_writer();
            assert_eq!(
                open_element(&mut w, "R", &[("k", format!("a{illegal}b"))]),
                Err(WriteRefusal::InvalidChar),
                "attribute value {illegal:?}"
            );
            assert!(w.get_ref().is_empty(), "a refused start tag writes nothing");
            assert_eq!(
                empty_element(&mut w, "E", &[("k", format!("a{illegal}b"))]),
                Err(WriteRefusal::InvalidChar),
                "empty element attribute value {illegal:?}"
            );
            let mut text = new_writer();
            assert_eq!(
                write_text(&mut text, &format!("a{illegal}b")),
                Err(WriteRefusal::InvalidChar),
                "text {illegal:?}"
            );
            assert!(
                text.get_ref().is_empty(),
                "a refused text node writes nothing"
            );
        }
    }

    #[test]
    fn writer_helpers_accept_the_char_production_edges() {
        let mut w = new_writer();
        open_element(&mut w, "R", &[("k", "a\tb\nc\rd\u{FFFD}")]).unwrap();
        text_element(&mut w, "T", "\u{10FFFF}").unwrap();
        close_element(&mut w, "R").unwrap();
        let doc = finish_document(w);
        assert!(
            doc.contains("\u{FFFD}") && doc.contains("\u{10FFFF}"),
            "{doc}"
        );
    }

    #[test]
    fn the_refusal_names_the_char_production() {
        assert_eq!(WriteRefusal::InvalidChar.name(), BAD_CHAR);
        assert_eq!(WriteRefusal::InvalidChar.to_string(), BAD_CHAR);
    }
}
