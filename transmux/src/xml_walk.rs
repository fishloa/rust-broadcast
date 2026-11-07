//! The quick-xml pull plumbing the DASH MPD and Smooth Streaming manifest
//! parsers share: element start/end events with decoded attributes, subtree
//! skipping, and the attribute lookup/parse helpers. Each parser supplies only
//! its own error enum through [`XmlWalkError`] and instantiates the wrappers
//! with [`xml_walk_fns!`].

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::str::FromStr;

use broadcast_common::xml_chars::{
    LimitExceeded, XmlBudget, XmlLimit, XmlLimits, char_data, namespace_of, read_attributes_checked,
};
use quick_xml::NsReader;
use quick_xml::errors::IllFormedError;
use quick_xml::events::Event;
use quick_xml::name::QName;

const UNKNOWN_PREFIX: &str = "unknown namespace prefix";

/// A namespace-aware `quick-xml` reader over the manifest text. Element names
/// are resolved against the namespace the parser accepts; hostile-input bounds
/// ([`XmlLimits`]) are enforced as events are read.
pub(crate) struct XmlReader<'a> {
    inner: NsReader<&'a [u8]>,
    accepted: Option<&'static str>,
    budget: XmlBudget,
    input_len: usize,
    input_checked: bool,
    root_seen: bool,
    root_unbound: bool,
}

/// What a parse loop sees: the start of an element (with its decoded
/// attributes) or the end of one. Text, comments, processing instructions, the
/// XML declaration and DOCTYPE are consumed by [`next_tag`] and never surface.
///
/// An element in the namespace the parser accepts (or, when the document root
/// is unprefixed and unbound, an unbound element) is reported by its local
/// name. An element in any other namespace is reported in Clark notation
/// (`{uri}local`), which no parser arm matches, so foreign content is skipped
/// and never read as the parser's own. Attributes follow the same rule: an
/// unprefixed attribute keeps its name, a namespaced one is `{uri}local`.
pub(crate) enum Tag {
    /// `<Name a="b">` or the self-closing `<Name a="b"/>`.
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
    fn ill_formed(pos: usize, message: String) -> Self;
    fn malformed_attribute(pos: usize) -> Self;
    fn unexpected_eof() -> Self;
    fn missing_attribute(element: &'static str, attr: &'static str) -> Self;
    fn invalid_attribute_value(element: &'static str, attr: &'static str, value: String) -> Self;
    fn too_large(limit: XmlLimit, max: usize) -> Self;
}

/// `{uri}local`, the name of a namespaced element or attribute.
pub(crate) fn clark(uri: &str, local: &str) -> String {
    format!("{{{uri}}}{local}")
}

pub(crate) fn new_reader<'a>(
    xml: &'a str,
    accepted: Option<&'static str>,
    limits: XmlLimits,
) -> XmlReader<'a> {
    XmlReader {
        inner: NsReader::from_str(xml),
        accepted,
        budget: XmlBudget::new(limits),
        input_len: xml.len(),
        input_checked: false,
        root_seen: false,
        root_unbound: false,
    }
}

fn position(reader: &XmlReader<'_>) -> usize {
    usize::try_from(reader.inner.buffer_position()).unwrap_or(usize::MAX)
}

fn xml_error<E: XmlWalkError>(reader: &XmlReader<'_>, err: &quick_xml::Error) -> E {
    use quick_xml::Error as Qe;
    let pos = position(reader);
    match err {
        Qe::Syntax(_) => E::unterminated_tag(pos),
        Qe::IllFormed(IllFormedError::MismatchedEndTag { expected, found }) => {
            E::mismatched_end_tag(local_part(expected), local_part(found))
        }
        _ => E::ill_formed(pos, err.to_string()),
    }
}

fn exceeded<E: XmlWalkError>(e: LimitExceeded) -> E {
    E::too_large(e.limit, e.max)
}

/// A well-formedness error described by `message` at the reader's position.
pub(crate) fn xml_message<E: XmlWalkError>(reader: &XmlReader<'_>, message: String) -> E {
    E::ill_formed(position(reader), message)
}

/// The local part of a (possibly prefixed) qualified name.
fn local_part(qname: &str) -> String {
    QName(qname).local_name().into_inner().to_string()
}

fn element_name(reader: &XmlReader<'_>, ns: Option<&str>, local: &str) -> String {
    match ns {
        None if reader.accepted.is_none() || reader.root_unbound => local.to_string(),
        None => clark("", local),
        Some(uri) if Some(uri) == reader.accepted => local.to_string(),
        Some(uri) => clark(uri, local),
    }
}

/// The next raw event with its element namespace; counts elements and depth
/// against the limits and rejects an undeclared element prefix.
fn read_resolved<'a, E: XmlWalkError>(
    reader: &mut XmlReader<'a>,
) -> Result<(Option<String>, Event<'a>), E> {
    if !reader.input_checked {
        reader.input_checked = true;
        reader
            .budget
            .check_input(reader.input_len)
            .map_err(exceeded)?;
    }
    let read = reader.inner.read_resolved_event();
    let (resolved, event) = match read {
        Ok(pair) => pair,
        Err(e) => return Err(xml_error(reader, &e)),
    };
    let ns = namespace_of(resolved);
    let ns = ns.map_err(|m| xml_message::<E>(reader, m))?;
    match &event {
        Event::Start(_) => reader.budget.open_element().map_err(exceeded)?,
        Event::Empty(_) => {
            reader.budget.open_element().map_err(exceeded)?;
            reader.budget.close_element();
        }
        Event::End(_) => reader.budget.close_element(),
        _ => {}
    }
    Ok((ns, event))
}

pub(crate) fn read_event<'a, E: XmlWalkError>(reader: &mut XmlReader<'a>) -> Result<Event<'a>, E> {
    read_resolved(reader).map(|(_, event)| event)
}

fn read_attrs<E: XmlWalkError>(
    reader: &XmlReader<'_>,
    e: &quick_xml::events::BytesStart<'_>,
) -> Result<Vec<(String, String)>, E> {
    let pos = position(reader);
    let attributes = read_attributes_checked(
        &reader.inner,
        e,
        |m| {
            if m.starts_with(UNKNOWN_PREFIX) {
                E::ill_formed(pos, m.to_string())
            } else {
                E::malformed_attribute(pos)
            }
        },
        |count, value| {
            reader
                .budget
                .admit_attribute(count, value)
                .map_err(exceeded)
        },
    )?;
    Ok(attributes
        .into_iter()
        .map(|a| {
            let key = match a.namespace {
                Some(uri) => clark(&uri, &a.name),
                None => a.name,
            };
            (key, a.value)
        })
        .collect())
}

/// The next element start/end, or `Ok(None)` at end of input.
pub(crate) fn next_tag<E: XmlWalkError>(reader: &mut XmlReader<'_>) -> Result<Option<Tag>, E> {
    loop {
        let (ns, event) = read_resolved(reader)?;
        if let Some(text) =
            char_data(&event).map_err(|message| xml_message::<E>(reader, message))?
        {
            if !reader.root_seen && !text.trim().is_empty() {
                return Err(xml_message(
                    reader,
                    "character data before the root element".to_string(),
                ));
            }
            continue;
        }
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                if !reader.root_seen {
                    reader.root_seen = true;
                    reader.root_unbound = ns.is_none();
                }
                let name = element_name(reader, ns.as_deref(), e.local_name().into_inner());
                return Ok(Some(Tag::Open {
                    name,
                    attrs: read_attrs(reader, e)?,
                    self_closing: matches!(event, Event::Empty(_)),
                }));
            }
            Event::End(e) => {
                let name = element_name(reader, ns.as_deref(), e.local_name().into_inner());
                return Ok(Some(Tag::Close { name }));
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
}

/// Require that nothing but whitespace, comments and processing instructions
/// follows the root element.
pub(crate) fn finish<E: XmlWalkError>(reader: &mut XmlReader<'_>) -> Result<(), E> {
    loop {
        let event = read_event(reader)?;
        if let Some(text) =
            char_data(&event).map_err(|message| xml_message::<E>(reader, message))?
        {
            if !text.trim().is_empty() {
                return Err(xml_message(
                    reader,
                    "character data after the root element".to_string(),
                ));
            }
            continue;
        }
        match event {
            Event::Start(_) | Event::Empty(_) => {
                return Err(xml_message(
                    reader,
                    "content after the root element".to_string(),
                ));
            }
            Event::Eof => return Ok(()),
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
    ($err:ty, $accepted:expr, $limits:expr) => {
        use $crate::xml_walk::{Tag, XmlReader, attr, attr_owned};

        fn new_reader(xml: &str) -> XmlReader<'_> {
            $crate::xml_walk::new_reader(xml, $accepted, $limits)
        }

        fn next_tag(reader: &mut XmlReader<'_>) -> Result<Option<Tag>> {
            $crate::xml_walk::next_tag(reader)
        }

        fn finish(reader: &mut XmlReader<'_>) -> Result<()> {
            $crate::xml_walk::finish(reader)
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
