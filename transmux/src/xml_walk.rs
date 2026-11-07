//! The quick-xml pull plumbing the DASH MPD and Smooth Streaming manifest
//! parsers share: element start/end events with decoded attributes, subtree
//! skipping, and the attribute lookup/parse helpers. Each parser supplies only
//! its own error enum through [`XmlWalkError`] and instantiates the wrappers
//! with [`xml_walk_fns!`].

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::str::FromStr;

use broadcast_common::xml_chars::{char_data, is_xml_char};
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::errors::IllFormedError;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::QName;

/// A `quick-xml` reader over the manifest text.
pub(crate) type XmlReader<'a> = Reader<&'a [u8]>;

/// What a parse loop sees: the start of an element (with its decoded
/// attributes) or the end of one. Text, comments, processing instructions, the
/// XML declaration and DOCTYPE are consumed by [`next_tag`] and never surface.
pub(crate) enum Tag {
    /// `<Name a="b">` or the self-closing `<Name a="b"/>`; `name` is the local
    /// name (namespace prefix stripped), attribute names keep any prefix.
    Open {
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
    },
    /// `</Name>`.
    Close { name: String },
}

/// The error constructors a manifest parser's error enum provides to the
/// shared walker.
pub(crate) trait XmlWalkError: Sized {
    fn unterminated_tag(pos: usize) -> Self;
    fn mismatched_end_tag(expected: String, found: String) -> Self;
    fn xml(pos: usize, message: String) -> Self;
    fn malformed_attribute(pos: usize) -> Self;
    fn unexpected_eof() -> Self;
    fn missing_attribute(element: &'static str, attr: &'static str) -> Self;
    fn invalid_attribute_value(element: &'static str, attr: &'static str, value: String) -> Self;
}

pub(crate) fn new_reader(xml: &str) -> XmlReader<'_> {
    Reader::from_str(xml)
}

fn position(reader: &XmlReader<'_>) -> usize {
    usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX)
}

fn xml_error<E: XmlWalkError>(reader: &XmlReader<'_>, err: &quick_xml::Error) -> E {
    use quick_xml::Error as Qe;
    let pos = position(reader);
    match err {
        Qe::Syntax(_) => E::unterminated_tag(pos),
        Qe::IllFormed(IllFormedError::MismatchedEndTag { expected, found }) => {
            E::mismatched_end_tag(local_part(expected), local_part(found))
        }
        _ => E::xml(pos, err.to_string()),
    }
}

/// A well-formedness error described by `message` at the reader's position.
pub(crate) fn xml_message<E: XmlWalkError>(reader: &XmlReader<'_>, message: String) -> E {
    E::xml(position(reader), message)
}

/// The local part of a (possibly prefixed) qualified name.
fn local_part(qname: &str) -> String {
    QName(qname).local_name().into_inner().to_string()
}

pub(crate) fn read_event<'a, E: XmlWalkError>(reader: &mut XmlReader<'a>) -> Result<Event<'a>, E> {
    reader.read_event().map_err(|e| xml_error(reader, &e))
}

/// Decode a start tag's attributes (values unescaped and normalized by
/// quick-xml).
fn read_attrs<E: XmlWalkError>(
    reader: &XmlReader<'_>,
    e: &BytesStart<'_>,
) -> Result<Vec<(String, String)>, E> {
    let pos = position(reader);
    let mut attrs = Vec::new();
    for attr in e.attributes() {
        let attr = attr.map_err(|_| E::malformed_attribute(pos))?;
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|_| E::malformed_attribute(pos))?;
        if !value.chars().all(is_xml_char) {
            return Err(E::malformed_attribute(pos));
        }
        attrs.push((attr.key.as_ref().to_string(), value.into_owned()));
    }
    Ok(attrs)
}

/// The next element start/end, or `Ok(None)` at end of input.
pub(crate) fn next_tag<E: XmlWalkError>(reader: &mut XmlReader<'_>) -> Result<Option<Tag>, E> {
    loop {
        let event = read_event(reader)?;
        if char_data(&event)
            .map_err(|message| xml_message::<E>(reader, message))?
            .is_some()
        {
            continue;
        }
        match event {
            Event::Start(e) => {
                return Ok(Some(Tag::Open {
                    name: e.local_name().into_inner().to_string(),
                    attrs: read_attrs(reader, &e)?,
                    self_closing: false,
                }));
            }
            Event::Empty(e) => {
                return Ok(Some(Tag::Open {
                    name: e.local_name().into_inner().to_string(),
                    attrs: read_attrs(reader, &e)?,
                    self_closing: true,
                }));
            }
            Event::End(e) => {
                return Ok(Some(Tag::Close {
                    name: e.local_name().into_inner().to_string(),
                }));
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
}

/// Skip an already-open element's subtree, up to and including its matching
/// end tag.
pub(crate) fn skip_element<E: XmlWalkError>(reader: &mut XmlReader<'_>) -> Result<(), E> {
    let mut depth: usize = 1;
    while depth > 0 {
        match next_tag(reader)? {
            Some(Tag::Open { self_closing, .. }) => {
                if !self_closing {
                    depth += 1;
                }
            }
            Some(Tag::Close { .. }) => depth -= 1,
            None => return Err(E::unexpected_eof()),
        }
    }
    Ok(())
}

pub(crate) fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

pub(crate) fn attr_owned(attrs: &[(String, String)], key: &str) -> Option<String> {
    attr(attrs, key).map(String::from)
}

pub(crate) fn required_attr_owned<E: XmlWalkError>(
    attrs: &[(String, String)],
    key: &'static str,
    element: &'static str,
) -> Result<String, E> {
    attr(attrs, key)
        .map(String::from)
        .ok_or_else(|| E::missing_attribute(element, key))
}

pub(crate) fn parse_attr<T: FromStr, E: XmlWalkError>(
    attrs: &[(String, String)],
    key: &'static str,
    element: &'static str,
) -> Result<Option<T>, E> {
    match attr(attrs, key) {
        Some(v) => v
            .trim()
            .parse::<T>()
            .map(Some)
            .map_err(|_| E::invalid_attribute_value(element, key, v.to_string())),
        None => Ok(None),
    }
}

pub(crate) fn required_attr_parse<T: FromStr, E: XmlWalkError>(
    attrs: &[(String, String)],
    key: &'static str,
    element: &'static str,
) -> Result<T, E> {
    let v = attr(attrs, key).ok_or_else(|| E::missing_attribute(element, key))?;
    v.trim()
        .parse::<T>()
        .map_err(|_| E::invalid_attribute_value(element, key, v.to_string()))
}

/// Instantiates the walker's generic helpers for one parser's error type, so
/// call sites keep inferring the error through `?`.
macro_rules! xml_walk_fns {
    ($err:ty) => {
        use $crate::xml_walk::{Tag, XmlReader, attr, attr_owned, new_reader};

        fn next_tag(reader: &mut XmlReader<'_>) -> Result<Option<Tag>> {
            $crate::xml_walk::next_tag(reader)
        }

        fn skip_element(reader: &mut XmlReader<'_>) -> Result<()> {
            $crate::xml_walk::skip_element(reader)
        }

        fn required_attr_owned(
            attrs: &[(String, String)],
            key: &'static str,
            element: &'static str,
        ) -> Result<String> {
            $crate::xml_walk::required_attr_owned(attrs, key, element)
        }

        fn parse_attr<T: FromStr>(
            attrs: &[(String, String)],
            key: &'static str,
            element: &'static str,
        ) -> Result<Option<T>> {
            $crate::xml_walk::parse_attr(attrs, key, element)
        }

        fn required_attr_parse<T: FromStr>(
            attrs: &[(String, String)],
            key: &'static str,
            element: &'static str,
        ) -> Result<T> {
            $crate::xml_walk::required_attr_parse(attrs, key, element)
        }
    };
}
pub(crate) use xml_walk_fns;
