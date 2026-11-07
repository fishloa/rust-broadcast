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

use broadcast_common::xml_chars::{LimitExceeded, TagAttribute, XmlBudget, XmlLimits};
use quick_xml::NsReader;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::PrefixDeclaration;

use crate::document::XmlDeclaration;
use crate::error::{Error, Result};

/// The start tag of the element currently being parsed.
#[derive(Debug)]
pub(crate) struct StartTag {
    namespace: Option<String>,
    /// The element name's own prefix as written (`None` for an unprefixed
    /// name, i.e. one in the default namespace).
    prefix: Option<String>,
    local: String,
    attributes: Vec<TagAttribute>,
    /// Named (non-default) namespace bindings in scope on this element,
    /// innermost first: the element's own declarations in document order, then
    /// each ancestor's that are not shadowed.
    scope: Vec<(String, String)>,
    /// Self-closing (`<x/>`): no content and no end tag follows.
    empty: bool,
    /// Per-attribute "a parser read this" flags (parallel to `attributes`),
    /// set by [`StartTag::attribute_in`]. Whatever is still unset once an
    /// element's own fields are filled is an attribute the crate does not
    /// model on that element and must be preserved as foreign (TTML2 §7.2,
    /// #1110/TT-W1).
    consumed: core::cell::RefCell<Vec<bool>>,
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

    /// The prefix the element name was written with (`None` if unprefixed).
    pub(crate) fn prefix(&self) -> Option<&str> {
        self.prefix.as_deref()
    }

    /// The value of the unprefixed (no-namespace) attribute `name`.
    pub(crate) fn attribute(&self, name: &str) -> Option<&str> {
        self.attribute_in(None, name)
    }

    /// The value of the attribute `local` in namespace `ns`.
    pub(crate) fn attribute_in(&self, ns: Option<&str>, local: &str) -> Option<&str> {
        let idx = self
            .attributes
            .iter()
            .position(|a| a.name == local && a.namespace.as_deref() == ns)?;
        if let Some(flag) = self.consumed.borrow_mut().get_mut(idx) {
            *flag = true;
        }
        Some(self.attributes[idx].value.as_str())
    }

    /// Whether the attribute at `idx` (an index into [`StartTag::attributes`])
    /// has been read through [`StartTag::attribute_in`].
    pub(crate) fn is_consumed(&self, idx: usize) -> bool {
        self.consumed.borrow().get(idx).copied().unwrap_or(false)
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
    /// The XML declaration, if the prolog carried one (set by [`Pull::root`]).
    declaration: Option<XmlDeclaration>,
    budget: XmlBudget,
}

fn exceeded(e: LimitExceeded) -> Error {
    Error::TooLarge {
        limit: e.limit,
        max: e.max,
    }
}

impl<'a> Pull<'a> {
    pub(crate) fn new(xml: &'a str, limits: XmlLimits) -> Result<Self> {
        let budget = XmlBudget::new(limits);
        budget.check_input(xml.len()).map_err(exceeded)?;
        Ok(Self {
            reader: NsReader::from_str(xml),
            pending: None,
            declaration: None,
            budget,
        })
    }

    fn account(&mut self, event: &Event<'_>) -> Result<()> {
        match event {
            Event::Start(_) => self.budget.open_element().map_err(exceeded),
            Event::Empty(_) => {
                self.budget.open_element().map_err(exceeded)?;
                self.budget.close_element();
                Ok(())
            }
            Event::End(_) => {
                self.budget.close_element();
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn raw(&mut self) -> Result<Event<'a>> {
        let event = match self.reader.read_event() {
            Ok(event) => event,
            Err(e) => return Err(self.fail(&e.to_string())),
        };
        self.account(&event)?;
        Ok(event)
    }

    fn fail(&self, message: &str) -> Error {
        Error::XmlParse(broadcast_common::xml_chars::message_at(
            &self.reader,
            message,
        ))
    }

    /// The next event with its (owned) element-namespace resolution.
    fn read(&mut self) -> Result<(core::result::Result<Option<String>, String>, Event<'a>)> {
        let (ns, event) =
            broadcast_common::xml_chars::read_event(&mut self.reader).map_err(|m| self.fail(&m))?;
        self.account(&event)?;
        Ok((ns, event))
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
        let attributes = broadcast_common::xml_chars::read_attributes_checked(
            &self.reader,
            e,
            |m| self.fail(m),
            |count, value| {
                self.budget
                    .admit_attribute(count, value)
                    .map_err(exceeded)
            },
        )?;
        let consumed = core::cell::RefCell::new(alloc::vec![false; attributes.len()]);
        let prefix = e.name().prefix().map(|p| String::from(p.into_inner()));
        Ok(StartTag {
            namespace,
            prefix,
            local: e.local_name().into_inner().to_string(),
            attributes,
            scope: self.scope(),
            empty,
            consumed,
        })
    }

    /// The one place character data is decoded and validated: `Some(text)` for
    /// a text run, CDATA section or entity/character reference (references
    /// resolved by quick-xml; a reference to an undefined entity or to a
    /// character outside the XML 1.0 `Char` production is an error), `None` for
    /// every other event. Both the item stream and the subtree skip go through
    /// it.
    fn char_data(&self, event: &Event<'_>) -> Result<Option<String>> {
        broadcast_common::xml_chars::char_data(event).map_err(|m| self.fail(&m))
    }

    /// The document's root start tag; rejects content before it.
    pub(crate) fn root(&mut self) -> Result<StartTag> {
        let mut at_start = true;
        loop {
            let (ns, event) = self.read()?;
            // XML 1.0 §2.8: the XMLDecl may only be the very first thing in
            // the document (no comment, whitespace or PI before it).
            let is_first = core::mem::replace(&mut at_start, false);
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
                Event::Decl(d) => {
                    if !is_first {
                        return Err(self.fail(
                            "the XML declaration must be at the very start of the document",
                        ));
                    }
                    // XML 1.0 §2.8 XMLDecl: version, optional encoding and
                    // standalone ("yes"/"no").
                    let version = d
                        .version()
                        .map(alloc::borrow::Cow::into_owned)
                        .map_err(|e| self.fail(&e.to_string()))?;
                    if !crate::document::is_version_num(&version) {
                        return Err(self.fail("XML declaration version must be '1.' [0-9]+"));
                    }
                    let encoding = d
                        .encoding()
                        .map(|r| {
                            r.map(alloc::borrow::Cow::into_owned)
                                .map_err(|e| self.fail(&e.to_string()))
                        })
                        .transpose()?;
                    if let Some(enc) = &encoding
                        && !crate::document::is_enc_name(enc)
                    {
                        return Err(self.fail("XML declaration encoding is not a valid EncName"));
                    }
                    let standalone = match d
                        .standalone()
                        .map(|r| {
                            r.map(alloc::borrow::Cow::into_owned)
                                .map_err(|e| self.fail(&e.to_string()))
                        })
                        .transpose()?
                        .as_deref()
                    {
                        None => None,
                        Some("yes") => Some(true),
                        Some("no") => Some(false),
                        Some(_) => return Err(self.fail("standalone must be \"yes\" or \"no\"")),
                    };
                    self.declaration =
                        Some(XmlDeclaration::from_parts(version, encoding, standalone));
                }
                Event::Eof => return Err(self.fail("no root element")),
                _ => {}
            }
        }
    }

    /// The XML declaration read by [`Pull::root`], if any.
    pub(crate) fn take_declaration(&mut self) -> Option<XmlDeclaration> {
        self.declaration.take()
    }

    /// After the root element closed: only comments, processing instructions
    /// and whitespace may follow.
    pub(crate) fn finish(&mut self) -> Result<()> {
        loop {
            let event = self.raw()?;
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
            let event = self.raw()?;
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
            let event = self.raw()?;
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
