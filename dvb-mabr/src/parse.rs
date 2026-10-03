//! Shared XML pull-parsing core and namespace constants — ETSI TS 103 769
//! V1.2.1 Annex A (baseline schema + extensibility mechanism).
//!
//! Every parser in this crate consumes [`quick_xml::NsReader`] events directly
//! into its typed output structs in one pass: [`Events`] hands a parser the
//! next child start tag of the element it is building ([`StartTag`]), the
//! parser fills its struct from the tag's attributes and recurses for the
//! children it models, and anything else — an Annex A.1 private/implementation
//! extension element in a foreign namespace, or a baseline element the model
//! does not know — is skipped with [`Events::skip`]. No tree of the document is
//! ever built.
//!
//! Elements are matched by `(namespace, local name)`: both the schema-version-1
//! (`:2019:`) and schema-version-2 (`:2024:`) baseline namespaces use the same
//! element/attribute local names (Annex A.0-1), so either matches; an element
//! in any other namespace is an extension and is skipped (MABR-W1, #1121).
//! Namespace resolution is quick-xml's (`NsReader`).

extern crate alloc;

use alloc::string::{String, ToString};

use quick_xml::NsReader;
use quick_xml::XmlVersion;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{QName, ResolveResult};

use crate::error::{Error, Result};

/// Baseline session-configuration namespace, schema version 2 (current) —
/// Annex A.2.
pub const NS_MULTICAST_SESSION_CONFIGURATION_2024: &str =
    "urn:dvb:metadata:MulticastSessionConfiguration:2024";
/// Baseline session-configuration namespace, schema version 1 — superseded
/// by the 2024 namespace; recorded here only for `@schemaVersion`
/// cross-reference (Annex A.0-1 Table).
pub const NS_MULTICAST_SESSION_CONFIGURATION_2019: &str =
    "urn:dvb:metadata:MulticastSessionConfiguration:2019";
/// Extensibility mechanism namespace (Annex A.1) — carries the
/// `NamespaceDelimiter` marker element used to terminate a standardized
/// extension block.
pub const NS_EXTENSIBILITY_2024: &str = "urn:dvb:metadata:Extensibility:2024";
/// `xsi:type` attribute namespace (W3C XML Schema instance).
pub const NS_XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// `true` if `ns` is a recognized MABR baseline namespace (2019 or 2024,
/// Annex A.0-1) — the two schema-version namespaces share every
/// element/attribute local name, so [`child`]/[`children`] match `(namespace,
/// local name)`, not local name alone. A node outside these (including no
/// namespace at all) is an Annex A.1 private/implementation extension and
/// must be skipped wherever it appears, not matched onto a baseline element
/// that happens to reuse the same local name (MABR-W1, #1121).
pub(crate) fn is_baseline_namespace(ns: Option<&str>) -> bool {
    matches!(
        ns,
        Some(NS_MULTICAST_SESSION_CONFIGURATION_2024)
            | Some(NS_MULTICAST_SESSION_CONFIGURATION_2019)
    )
}

/// One attribute of a start tag: namespace URI (`None` when unprefixed — an
/// unprefixed attribute is never in the default namespace), local name, value.
#[derive(Debug)]
struct TagAttribute {
    namespace: Option<String>,
    name: String,
    value: String,
}

/// The start tag of the element currently being parsed: its resolved name and
/// its attributes (namespace declarations excluded), read once through
/// quick-xml's `Attributes` iterator.
#[derive(Debug)]
pub(crate) struct StartTag {
    namespace: Option<String>,
    local: String,
    attributes: alloc::vec::Vec<TagAttribute>,
    /// Self-closing (`<X/>`): no content and no end tag follows.
    empty: bool,
}

impl StartTag {
    /// The local name.
    pub(crate) fn local(&self) -> &str {
        &self.local
    }

    /// The namespace URI, if any.
    pub(crate) fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    /// `true` if this element is `name` in a baseline namespace.
    pub(crate) fn is(&self, name: &str) -> bool {
        self.local == name && is_baseline_namespace(self.namespace())
    }

    /// An unprefixed (no-namespace) attribute's value — every MABR attribute
    /// except `xsi:type`.
    pub(crate) fn attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|a| a.namespace.is_none() && a.name == name)
            .map(|a| a.value.as_str())
    }

    pub(crate) fn attr_owned(&self, name: &str) -> Option<String> {
        self.attr(name).map(ToString::to_string)
    }

    /// The `xsi:type` attribute's local name (any namespace prefix on the
    /// *value* itself is stripped — only the local type name distinguishes the
    /// `ServiceComponentIdentifier` variants, clause 10.2.4).
    pub(crate) fn xsi_type(&self) -> Option<String> {
        self.attributes
            .iter()
            .find(|a| a.namespace.as_deref() == Some(NS_XSI) && a.name == "type")
            .map(|a| match a.value.rsplit_once(':') {
                Some((_, local)) => local.to_string(),
                None => a.value.clone(),
            })
    }
}

/// The pull parser: a [`quick_xml::NsReader`] over the input.
pub(crate) struct Events<'a> {
    reader: NsReader<&'a [u8]>,
}

/// XML 1.0 §2.2 `Char` production: the only characters a document may contain,
/// whether written literally or through a character reference.
fn is_xml_char(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'
    )
}

const BAD_CHAR: &str = "character not allowed in XML 1.0 (Char production, §2.2)";

fn xml_error(reader: &NsReader<&[u8]>, message: &str) -> Error {
    Error::XmlParse(alloc::format!(
        "{message} at byte {}",
        reader.buffer_position()
    ))
}

fn namespace_of(result: ResolveResult<'_>) -> core::result::Result<Option<String>, String> {
    match result {
        ResolveResult::Unbound => Ok(None),
        ResolveResult::Bound(ns) => Ok(Some(ns.into_inner().to_string())),
        ResolveResult::Unknown(prefix) => {
            Err(alloc::format!("unknown namespace prefix '{prefix}'"))
        }
    }
}

impl<'a> Events<'a> {
    pub(crate) fn new(xml: &'a str) -> Self {
        Self {
            reader: NsReader::from_str(xml),
        }
    }

    fn fail(&self, message: &str) -> Error {
        xml_error(&self.reader, message)
    }

    /// Build the [`StartTag`] for a start/empty event, resolving the element
    /// and attribute namespaces with the reader's resolver.
    fn start_tag(
        &self,
        ns: core::result::Result<Option<String>, String>,
        e: &BytesStart<'_>,
        empty: bool,
    ) -> Result<StartTag> {
        let namespace = ns.map_err(|m| self.fail(&m))?;
        let resolver = self.reader.resolver();
        let mut attributes = alloc::vec::Vec::new();
        for attr in e.attributes() {
            let attr = attr.map_err(|e| self.fail(&e.to_string()))?;
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
                .map_err(|e| self.fail(&e.to_string()))?
                .into_owned();
            if !value.chars().all(is_xml_char) {
                return Err(self.fail(BAD_CHAR));
            }
            let (attr_ns, attr_local) = resolver.resolve_attribute(key);
            attributes.push(TagAttribute {
                namespace: namespace_of(attr_ns).map_err(|m| self.fail(&m))?,
                name: attr_local.into_inner().to_string(),
                value,
            });
        }
        Ok(StartTag {
            namespace,
            local: e.local_name().into_inner().to_string(),
            attributes,
            empty,
        })
    }

    /// The next event with its (owned) element-namespace resolution.
    fn read(&mut self) -> Result<(core::result::Result<Option<String>, String>, Event<'a>)> {
        match self.reader.read_resolved_event() {
            Ok((ns, event)) => Ok((namespace_of(ns), event)),
            Err(e) => Err(self.fail(&e.to_string())),
        }
    }

    /// The one place character data is decoded and validated: `Some(text)` for
    /// a text run, CDATA section or entity/character reference (references
    /// resolved by quick-xml; a reference to an undefined entity or to a
    /// character outside the XML 1.0 `Char` production is an error), `None` for
    /// every other event. Every path that sees content — the modelled `text`,
    /// the child walk, and the subtree skip — goes through it.
    fn char_data(&self, event: &Event<'_>) -> Result<Option<String>> {
        let content = match event {
            Event::Text(t) => t.xml10_content().into_owned(),
            Event::CData(c) => c.xml10_content().into_owned(),
            Event::GeneralRef(r) => match r.resolve_char_ref() {
                Ok(Some(c)) => c.to_string(),
                Ok(None) => match resolve_predefined_entity(r) {
                    Some(s) => s.to_string(),
                    None => {
                        return Err(
                            self.fail(&alloc::format!("undefined entity reference &{};", &**r))
                        );
                    }
                },
                Err(e) => return Err(self.fail(&e.to_string())),
            },
            _ => return Ok(None),
        };
        if content.chars().all(is_xml_char) {
            Ok(Some(content))
        } else {
            Err(self.fail(BAD_CHAR))
        }
    }

    /// Consume the content of an already-open element up to and including its
    /// end tag, validating all character data on the way (end-tag matching is
    /// quick-xml's).
    fn skip_content(&mut self) -> Result<()> {
        let mut depth: usize = 1;
        while depth > 0 {
            let event = match self.reader.read_event() {
                Ok(event) => event,
                Err(e) => return Err(self.fail(&e.to_string())),
            };
            if self.char_data(&event)?.is_some() {
                continue;
            }
            match event {
                Event::Start(_) => depth += 1,
                Event::End(_) => depth -= 1,
                Event::DocType(_) => {
                    return Err(self.fail("DOCTYPE declarations are not allowed"));
                }
                Event::Eof => return Err(self.fail("unexpected end of input inside an element")),
                _ => {}
            }
        }
        Ok(())
    }

    /// Read events until the next start tag, skipping declarations, comments,
    /// processing instructions and character data (this model has no mixed
    /// content; the data is still validated). `Ok(None)` at the parent's end
    /// tag.
    fn next_tag(&mut self) -> Result<Option<StartTag>> {
        loop {
            let (ns, event) = self.read()?;
            if self.char_data(&event)?.is_some() {
                continue;
            }
            match event {
                Event::Start(e) => return self.start_tag(ns, &e, false).map(Some),
                Event::Empty(e) => return self.start_tag(ns, &e, true).map(Some),
                Event::End(_) => return Ok(None),
                // The parent element is still open: a truncated document, not
                // an end of its children.
                Event::Eof => return Err(self.fail("unexpected end of input inside an element")),
                Event::DocType(_) => {
                    return Err(self.fail("DOCTYPE declarations are not allowed"));
                }
                _ => {}
            }
        }
    }

    /// The document's root start tag; rejects content before it.
    pub(crate) fn root(&mut self) -> Result<StartTag> {
        loop {
            let (ns, event) = self.read()?;
            match event {
                Event::Start(e) => return self.start_tag(ns, &e, false),
                Event::Empty(e) => return self.start_tag(ns, &e, true),
                Event::Text(t) if !t.trim_ascii().is_empty() => {
                    return Err(self.fail("character data outside the root element"));
                }
                Event::CData(_) | Event::GeneralRef(_) => {
                    return Err(self.fail("character data outside the root element"));
                }
                Event::DocType(_) => {
                    return Err(self.fail("DOCTYPE declarations are not allowed"));
                }
                Event::Eof => return Err(self.fail("no root element")),
                _ => {}
            }
        }
    }

    /// After the root element closed: only comments, processing instructions
    /// and whitespace may follow. Also used after a semantic error to surface a
    /// well-formedness error elsewhere in the document first.
    pub(crate) fn finish(&mut self) -> Result<()> {
        loop {
            let event = self
                .reader
                .read_event()
                .map_err(|e| xml_error(&self.reader, &e.to_string()))?;
            match event {
                Event::Eof => return Ok(()),
                Event::Text(t) if t.trim_ascii().is_empty() => {}
                Event::Comment(_) | Event::PI(_) => {}
                Event::Start(_) | Event::Empty(_) => {
                    return Err(self.fail("more than one root element"));
                }
                _ => return Err(self.fail("content after the root element")),
            }
        }
    }

    /// After a semantic error mid-document: read the rest of the input only to
    /// surface a well-formedness error (which then takes precedence over the
    /// semantic one, as it would have when the whole document was read first).
    pub(crate) fn drain(&mut self) -> Result<()> {
        loop {
            let event = self
                .reader
                .read_event()
                .map_err(|e| xml_error(&self.reader, &e.to_string()))?;
            match event {
                Event::Eof => return Ok(()),
                Event::DocType(_) => {
                    return Err(self.fail("DOCTYPE declarations are not allowed"));
                }
                _ => {}
            }
        }
    }

    /// The next child element of the element being parsed, or `None` at its
    /// end tag. A self-closing parent (`tag.empty`) has no children: callers
    /// check [`StartTag::empty`] via [`Events::children_of`].
    pub(crate) fn next_child(&mut self, parent: &StartTag) -> Result<Option<StartTag>> {
        if parent.empty {
            return Ok(None);
        }
        self.next_tag()
    }

    /// Skip `tag`'s content up to and including its end tag.
    pub(crate) fn skip(&mut self, tag: &StartTag) -> Result<()> {
        if tag.empty {
            return Ok(());
        }
        self.skip_content()
    }

    /// The direct character data of `tag` (entity and character references
    /// resolved, CDATA included), trimmed; nested elements are skipped. Consumes
    /// the end tag.
    pub(crate) fn text(&mut self, tag: &StartTag) -> Result<String> {
        if tag.empty {
            return Ok(String::new());
        }
        let mut text = String::new();
        loop {
            let event = match self.reader.read_event() {
                Ok(event) => event,
                Err(e) => return Err(self.fail(&e.to_string())),
            };
            if let Some(piece) = self.char_data(&event)? {
                text.push_str(&piece);
                continue;
            }
            match event {
                Event::Start(_) => self.skip_content()?,
                Event::End(_) => return Ok(text.trim().to_string()),
                Event::DocType(_) => {
                    return Err(self.fail("DOCTYPE declarations are not allowed"));
                }
                Event::Eof => return Err(self.fail("unexpected end of input inside an element")),
                _ => {}
            }
        }
    }
}

/// Visit every baseline child of `tag`: `f` is called with each child whose
/// name it may claim and returns `true` once it has consumed the child's
/// content; a `false` return (or a child in a foreign namespace) skips the
/// child's subtree.
pub(crate) fn for_each_child<F>(ev: &mut Events<'_>, tag: &StartTag, mut f: F) -> Result<()>
where
    F: FnMut(&mut Events<'_>, &StartTag) -> Result<bool>,
{
    while let Some(child) = ev.next_child(tag)? {
        let handled = if is_baseline_namespace(child.namespace()) {
            f(ev, &child)?
        } else {
            false
        };
        if !handled {
            ev.skip(&child)?;
        }
    }
    Ok(())
}

pub(crate) fn require_attr(
    tag: &StartTag,
    element: &'static str,
    name: &'static str,
) -> Result<String> {
    tag.attr_owned(name).ok_or(Error::MissingAttribute {
        element,
        attr: name,
    })
}

/// The "required child element absent" error.
pub(crate) fn missing_element(parent: &'static str, child: &'static str) -> Error {
    Error::MissingElement { parent, child }
}

fn invalid(
    element: &'static str,
    attr_name: &'static str,
    value: &str,
    reason: &'static str,
) -> Error {
    Error::InvalidAttribute {
        element,
        attr: attr_name,
        value: value.to_string(),
        reason,
    }
}

pub(crate) fn parse_u16(
    element: &'static str,
    attr_name: &'static str,
    value: &str,
) -> Result<u16> {
    value.trim().parse::<u16>().map_err(|_| {
        invalid(
            element,
            attr_name,
            value,
            "expected an unsigned 16-bit integer",
        )
    })
}

pub(crate) fn parse_u32(
    element: &'static str,
    attr_name: &'static str,
    value: &str,
) -> Result<u32> {
    value.trim().parse::<u32>().map_err(|_| {
        invalid(
            element,
            attr_name,
            value,
            "expected an unsigned 32-bit integer",
        )
    })
}

pub(crate) fn parse_u64(
    element: &'static str,
    attr_name: &'static str,
    value: &str,
) -> Result<u64> {
    value.trim().parse::<u64>().map_err(|_| {
        invalid(
            element,
            attr_name,
            value,
            "expected an unsigned 64-bit integer",
        )
    })
}

pub(crate) fn parse_bool(
    element: &'static str,
    attr_name: &'static str,
    value: &str,
) -> Result<bool> {
    match value.trim() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(invalid(
            element,
            attr_name,
            value,
            "expected xs:boolean (true/false/1/0)",
        )),
    }
}

pub(crate) fn parse_f64(
    element: &'static str,
    attr_name: &'static str,
    value: &str,
) -> Result<f64> {
    let parsed = value
        .trim()
        .parse::<f64>()
        .map_err(|_| invalid(element, attr_name, value, "expected a decimal number"))?;
    // `f64::from_str` accepts "NaN"/"inf"/"infinity", none of which is a
    // valid `xs:decimal`/`xs:double` lexical form, and `NaN != NaN` would
    // break the documented parse -> to_xml -> parse round trip (MABR-W3,
    // #1121).
    if !parsed.is_finite() {
        return Err(invalid(
            element,
            attr_name,
            value,
            "must be a finite decimal number (NaN/infinity are not valid xs:decimal)",
        ));
    }
    Ok(parsed)
}

pub(crate) fn req_attr_u32(
    tag: &StartTag,
    element: &'static str,
    name: &'static str,
) -> Result<u32> {
    parse_u32(element, name, &require_attr(tag, element, name)?)
}

pub(crate) fn req_attr_u64(
    tag: &StartTag,
    element: &'static str,
    name: &'static str,
) -> Result<u64> {
    parse_u64(element, name, &require_attr(tag, element, name)?)
}

pub(crate) fn opt_attr_u32(
    tag: &StartTag,
    element: &'static str,
    name: &'static str,
) -> Result<Option<u32>> {
    match tag.attr(name) {
        Some(v) => Ok(Some(parse_u32(element, name, v)?)),
        None => Ok(None),
    }
}

pub(crate) fn opt_attr_u64(
    tag: &StartTag,
    element: &'static str,
    name: &'static str,
) -> Result<Option<u64>> {
    match tag.attr(name) {
        Some(v) => Ok(Some(parse_u64(element, name, v)?)),
        None => Ok(None),
    }
}

pub(crate) fn opt_attr_bool(
    tag: &StartTag,
    element: &'static str,
    name: &'static str,
) -> Result<Option<bool>> {
    match tag.attr(name) {
        Some(v) => Ok(Some(parse_bool(element, name, v)?)),
        None => Ok(None),
    }
}

pub(crate) fn opt_attr_f64(
    tag: &StartTag,
    element: &'static str,
    name: &'static str,
) -> Result<Option<f64>> {
    match tag.attr(name) {
        Some(v) => Ok(Some(parse_f64(element, name, v)?)),
        None => Ok(None),
    }
}
