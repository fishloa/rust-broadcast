//! Pull parsing over [`quick_xml::NsReader`] events.
//!
//! The TTML parsers in [`crate::document`] consume this event stream directly
//! into their typed structs in one pass: [`Pull::next`] hands a parser the next
//! child [`Item`] of the element it is building (a child start tag, a run of
//! character data, or the end), the parser fills its struct from the start
//! tag's attributes and recurses for the children it models, and anything it
//! does not model is captured losslessly as an [`UnknownElement`] while it
//! streams past. No tree of the document is ever built.
//!
//! The lexing, entity and character reference resolution, attribute-value
//! normalization, end-tag matching and namespace resolution are all quick-xml's
//! (`NsReader::read_resolved_event`, `resolve_attribute`, `bindings_of`); the
//! only work done here is to hand the parsers each event in a convenient shape.
//!
//! Strictness matches the `roxmltree` parser this replaced: a `<!DOCTYPE>`
//! (a vector for entity-expansion attacks), an undefined entity reference, an
//! unbound namespace prefix, a second root element or content outside the root
//! is an error, never silently accepted.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use quick_xml::NsReader;
use quick_xml::XmlVersion;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{PrefixDeclaration, QName, ResolveResult};

use crate::error::{Error, Result};

/// One attribute of a start tag (namespace declarations excluded).
#[derive(Debug)]
pub(crate) struct TagAttribute {
    /// Resolved namespace URI; `None` for an unprefixed attribute (which is
    /// never in the default namespace).
    pub(crate) namespace: Option<String>,
    /// Local name.
    pub(crate) name: String,
    /// Normalized, unescaped value.
    pub(crate) value: String,
}

/// The start tag of the element currently being parsed.
#[derive(Debug)]
pub(crate) struct StartTag {
    namespace: Option<String>,
    local: String,
    attributes: Vec<TagAttribute>,
    /// Named (non-default) namespace bindings in scope on this element,
    /// innermost first: the element's own declarations in document order, then
    /// each ancestor's that are not shadowed.
    scope: Vec<(String, String)>,
    /// Self-closing (`<x/>`): no content and no end tag follows.
    empty: bool,
}

impl StartTag {
    /// The local name.
    pub(crate) fn local(&self) -> &str {
        &self.local
    }

    /// The resolved namespace URI, if any.
    pub(crate) fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    /// The value of the unprefixed (no-namespace) attribute `name`.
    pub(crate) fn attribute(&self, name: &str) -> Option<&str> {
        self.attribute_in(None, name)
    }

    /// The value of the attribute `local` in namespace `ns`.
    pub(crate) fn attribute_in(&self, ns: Option<&str>, local: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|a| a.name == local && a.namespace.as_deref() == ns)
            .map(|a| a.value.as_str())
    }

    /// All attributes, in document order.
    pub(crate) fn attributes(&self) -> &[TagAttribute] {
        &self.attributes
    }

    /// The named namespace bindings in scope, innermost first.
    pub(crate) fn scope(&self) -> &[(String, String)] {
        &self.scope
    }
}

/// What a parser sees next inside the element it is building.
#[derive(Debug)]
pub(crate) enum Item {
    /// A child element's start tag (its content follows).
    Start(StartTag),
    /// A run of character data (entity/character references resolved, CDATA
    /// included); a comment or processing instruction ends a run.
    Text(String),
    /// A comment or processing instruction (never represented in the model).
    Other,
    /// The end of the element being built.
    End,
}

/// The pull parser: a [`quick_xml::NsReader`] over the input.
pub(crate) struct Pull<'a> {
    reader: NsReader<&'a [u8]>,
    /// An item already read (and its namespaces resolved) while a text run was
    /// being gathered; returned by the next [`Pull::next`].
    pending: Option<Item>,
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

fn namespace_of(result: ResolveResult<'_>) -> core::result::Result<Option<String>, String> {
    match result {
        ResolveResult::Unbound => Ok(None),
        ResolveResult::Bound(ns) => Ok(Some(ns.into_inner().to_string())),
        ResolveResult::Unknown(prefix) => {
            Err(alloc::format!("unknown namespace prefix '{prefix}'"))
        }
    }
}

impl<'a> Pull<'a> {
    pub(crate) fn new(xml: &'a str) -> Self {
        Self {
            reader: NsReader::from_str(xml),
            pending: None,
        }
    }

    fn fail(&self, message: &str) -> Error {
        Error::XmlParse(alloc::format!(
            "{message} at byte {}",
            self.reader.buffer_position()
        ))
    }

    /// The next event with its (owned) element-namespace resolution.
    fn read(&mut self) -> Result<(core::result::Result<Option<String>, String>, Event<'a>)> {
        match self.reader.read_resolved_event() {
            Ok((ns, event)) => Ok((namespace_of(ns), event)),
            Err(e) => Err(self.fail(&e.to_string())),
        }
    }

    /// The named namespace bindings in scope right now, innermost first.
    fn scope(&self) -> Vec<(String, String)> {
        let resolver = self.reader.resolver();
        let mut out: Vec<(String, String)> = Vec::new();
        for level in (1..=resolver.level()).rev() {
            for (decl, ns) in resolver.bindings_of(level) {
                if let PrefixDeclaration::Named(prefix) = decl
                    && !out.iter().any(|(p, _)| p == prefix)
                {
                    out.push((prefix.to_string(), ns.into_inner().to_string()));
                }
            }
        }
        out
    }

    /// Build the [`StartTag`] for a start/empty event, resolving attribute
    /// namespaces with the reader's resolver.
    fn start_tag(
        &self,
        ns: core::result::Result<Option<String>, String>,
        e: &BytesStart<'_>,
        empty: bool,
    ) -> Result<StartTag> {
        let namespace = ns.map_err(|m| self.fail(&m))?;
        let resolver = self.reader.resolver();
        let mut attributes = Vec::new();
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
            scope: self.scope(),
            empty,
        })
    }

    /// The one place character data is decoded and validated: `Some(text)` for
    /// a text run, CDATA section or entity/character reference (references
    /// resolved by quick-xml; a reference to an undefined entity or to a
    /// character outside the XML 1.0 `Char` production is an error), `None` for
    /// every other event. Both the item stream and the subtree skip go through
    /// it.
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
    /// and whitespace may follow.
    pub(crate) fn finish(&mut self) -> Result<()> {
        loop {
            let event = match self.reader.read_event() {
                Ok(event) => event,
                Err(e) => return Err(self.fail(&e.to_string())),
            };
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
    /// surface a well-formedness error, which then takes precedence over the
    /// semantic one (as it did when the whole document was read first).
    pub(crate) fn drain(&mut self) -> Result<()> {
        loop {
            let event = match self.reader.read_event() {
                Ok(event) => event,
                Err(e) => return Err(self.fail(&e.to_string())),
            };
            match event {
                Event::Eof => return Ok(()),
                Event::DocType(_) => {
                    return Err(self.fail("DOCTYPE declarations are not allowed"));
                }
                _ => {}
            }
        }
    }

    /// The next [`Item`] inside `parent`. A self-closing parent yields
    /// [`Item::End`] immediately.
    pub(crate) fn next(&mut self, parent: &StartTag) -> Result<Item> {
        if parent.empty {
            return Ok(Item::End);
        }
        if let Some(item) = self.pending.take() {
            return Ok(item);
        }
        let mut run = String::new();
        loop {
            let (ns, event) = self.read()?;
            if let Some(piece) = self.char_data(&event)? {
                run.push_str(&piece);
                continue;
            }
            let item = match event {
                Event::Start(e) => Item::Start(self.start_tag(ns, &e, false)?),
                Event::Empty(e) => Item::Start(self.start_tag(ns, &e, true)?),
                Event::End(_) => Item::End,
                Event::Comment(_) | Event::PI(_) => Item::Other,
                Event::Decl(_) => continue,
                Event::DocType(_) => {
                    return Err(self.fail("DOCTYPE declarations are not allowed"));
                }
                Event::Eof => return Err(self.fail("unexpected end of input inside an element")),
                // Character data was handled above.
                Event::Text(_) | Event::CData(_) | Event::GeneralRef(_) => continue,
            };
            if run.is_empty() {
                return Ok(item);
            }
            self.pending = Some(item);
            return Ok(Item::Text(run));
        }
    }

    /// Skip `tag`'s content up to and including its end tag.
    pub(crate) fn skip(&mut self, tag: &StartTag) -> Result<()> {
        if tag.empty {
            return Ok(());
        }
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

    /// The next child element of `parent` (character data and comments are
    /// passed over), or `None` at its end.
    pub(crate) fn next_element(&mut self, parent: &StartTag) -> Result<Option<StartTag>> {
        loop {
            match self.next(parent)? {
                Item::Start(tag) => return Ok(Some(tag)),
                Item::End => return Ok(None),
                Item::Text(_) | Item::Other => {}
            }
        }
    }
}
