//! Bounded namespace-aware XML reading into a small element tree.
//!
//! quick-xml does the lexing, entity and namespace resolution. This layer adds
//! the hostile-input bounds ([`Limits`]), refuses DOCTYPE declarations (so no
//! entity can be declared, hence no entity-expansion attack), validates every
//! character against the XML 1.0 `Char` production, and drops everything that
//! is not in the MPD namespace (the schema's `any`) before it costs memory.

use quick_xml::NsReader;
use quick_xml::XmlVersion;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;

use crate::error::{Limit, MpdError};
use crate::limits::Limits;

/// ISO/IEC 23009-1 clause 5.3.1 / Annex B: the MPD schema namespace.
pub const NS_MPD: &str = "urn:mpeg:dash:schema:mpd:2011";
/// ETSI TS 103 285 clause 10.8.2: DVB DASH extension namespace.
pub const NS_DVB: &str = "urn:dvb:dash:dash-extensions:2014-1";
/// ISO/IEC 23001-7: `cenc` attribute namespace (`cenc:default_KID`).
pub const NS_CENC: &str = "urn:mpeg:cenc:2013";
/// XLink namespace of `@xlink:href`/`@xlink:actuate` (5.5.3).
pub const NS_XLINK: &str = "http://www.w3.org/1999/xlink";
/// ISO/IEC 23009-1 Annex I.2: URL parameter namespace.
pub const NS_URLPARAM: &str = "urn:mpeg:dash:schema:urlparam:2014";

/// The namespaces the model reads; anything else is `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ns {
    None,
    Mpd,
    Dvb,
    Cenc,
    Xlink,
    UrlParam,
    Other,
}

impl Ns {
    fn classify(uri: &str) -> Self {
        match uri {
            NS_MPD => Self::Mpd,
            NS_DVB => Self::Dvb,
            NS_CENC => Self::Cenc,
            NS_XLINK => Self::Xlink,
            NS_URLPARAM => Self::UrlParam,
            _ => Self::Other,
        }
    }

    fn kept(self) -> bool {
        matches!(self, Self::Mpd | Self::UrlParam)
    }
}

#[derive(Debug)]
pub(crate) struct Attr {
    pub ns: Ns,
    pub name: String,
    pub value: String,
}

#[derive(Debug)]
pub(crate) struct Elem {
    pub ns: Ns,
    pub name: String,
    pub attrs: Vec<Attr>,
    pub children: Vec<Elem>,
    pub text: String,
}

impl Elem {
    pub(crate) fn attr(&self, name: &str) -> Option<&str> {
        self.attr_in(Ns::None, name)
    }

    pub(crate) fn attr_in(&self, ns: Ns, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.ns == ns && a.name == name)
            .map(|a| a.value.as_str())
    }

    pub(crate) fn children_named<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a Elem> + 'a {
        self.children
            .iter()
            .filter(move |c| c.ns == Ns::Mpd && c.name == name)
    }

    pub(crate) fn child(&self, name: &str) -> Option<&Elem> {
        self.children
            .iter()
            .find(|c| c.ns == Ns::Mpd && c.name == name)
    }

    pub(crate) fn has_child(&self, name: &str) -> bool {
        self.child(name).is_some()
    }
}

fn is_xml_char(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'
    )
}

const BAD_CHAR: &str = "character not allowed in XML 1.0 (Char production, 2.2)";
const BOM: char = '\u{FEFF}';

struct Reader<'a> {
    inner: NsReader<&'a [u8]>,
}

fn ns_of(result: ResolveResult<'_>) -> Result<Ns, String> {
    match result {
        ResolveResult::Unbound => Ok(Ns::None),
        ResolveResult::Bound(ns) => Ok(Ns::classify(ns.into_inner())),
        ResolveResult::Unknown(prefix) => Err(format!("unknown namespace prefix '{prefix}'")),
    }
}

impl<'a> Reader<'a> {
    fn fail(&self, message: impl Into<String>) -> MpdError {
        MpdError::Xml {
            pos: usize::try_from(self.inner.buffer_position()).unwrap_or(usize::MAX),
            message: message.into(),
        }
    }

    fn attrs(&self, e: &BytesStart<'_>, limits: &Limits) -> Result<Vec<Attr>, MpdError> {
        let resolver = self.inner.resolver();
        let mut out = Vec::new();
        let mut seen = 0usize;
        for attr in e.attributes() {
            let attr = attr.map_err(|err| self.fail(err.to_string()))?;
            let key = attr.key;
            let is_declaration = match key.prefix() {
                None => key.as_ref() == "xmlns",
                Some(p) => p.is_xmlns(),
            };
            if is_declaration {
                continue;
            }
            seen += 1;
            if seen > limits.max_attributes {
                return Err(MpdError::TooLarge {
                    limit: Limit::Attributes,
                    max: limits.max_attributes,
                });
            }
            let value = attr
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|err| self.fail(err.to_string()))?;
            if value.len() > limits.max_attribute_bytes {
                return Err(MpdError::TooLarge {
                    limit: Limit::AttributeBytes,
                    max: limits.max_attribute_bytes,
                });
            }
            if !value.chars().all(is_xml_char) {
                return Err(self.fail(BAD_CHAR));
            }
            let (ns, local) = resolver.resolve_attribute(key);
            out.push(Attr {
                ns: ns_of(ns).map_err(|m| self.fail(m))?,
                name: local.into_inner().to_string(),
                value: value.into_owned(),
            });
        }
        Ok(out)
    }

    fn char_data(&self, event: &Event<'_>) -> Result<Option<String>, MpdError> {
        let content = match event {
            Event::Text(t) => t.xml10_content().into_owned(),
            Event::CData(c) => c.xml10_content().into_owned(),
            Event::GeneralRef(r) => match r.resolve_char_ref() {
                Ok(Some(c)) => c.to_string(),
                Ok(None) => match resolve_predefined_entity(r) {
                    Some(s) => s.to_string(),
                    None => return Err(self.fail(format!("undefined entity reference &{};", &**r))),
                },
                Err(e) => return Err(self.fail(e.to_string())),
            },
            _ => return Ok(None),
        };
        if content.chars().all(is_xml_char) {
            Ok(Some(content))
        } else {
            Err(self.fail(BAD_CHAR))
        }
    }
}

enum Frame {
    Kept(Elem),
    Skipped,
}

/// Read `xml` into an element tree rooted at the `MPD` element.
pub(crate) fn read_tree(xml: &[u8], limits: &Limits) -> Result<Elem, MpdError> {
    if xml.len() > limits.max_bytes {
        return Err(MpdError::TooLarge {
            limit: Limit::InputBytes,
            max: limits.max_bytes,
        });
    }
    let text = core::str::from_utf8(xml).map_err(|e| MpdError::InvalidUtf8 {
        valid_up_to: e.valid_up_to(),
    })?;
    let text = text.strip_prefix(BOM).unwrap_or(text);
    let mut reader = Reader {
        inner: NsReader::from_str(text),
    };

    let mut stack: Vec<Frame> = Vec::new();
    let mut elements = 0usize;
    let mut root: Option<Elem> = None;

    loop {
        let (ns_result, event) = match reader.inner.read_resolved_event() {
            Ok((ns, event)) => (ns_of(ns), event),
            Err(e) => return Err(reader.fail(e.to_string())),
        };
        if let Some(piece) = reader.char_data(&event)? {
            if stack.is_empty() && !piece.trim().is_empty() {
                return Err(reader.fail("character data outside the root element"));
            }
            if let Some(Frame::Kept(top)) = stack.last_mut() {
                top.text.push_str(&piece);
            }
            continue;
        }
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                if root.is_some() && stack.is_empty() {
                    return Err(reader.fail("more than one root element"));
                }
                elements += 1;
                if elements > limits.max_elements {
                    return Err(MpdError::TooLarge {
                        limit: Limit::Elements,
                        max: limits.max_elements,
                    });
                }
                if stack.len() + 1 > limits.max_depth {
                    return Err(MpdError::TooLarge {
                        limit: Limit::Depth,
                        max: limits.max_depth,
                    });
                }
                let ns = ns_result.map_err(|m| reader.fail(m))?;
                let local = e.local_name().into_inner();
                let is_root = stack.is_empty();
                if is_root && (ns != Ns::Mpd || local != "MPD") {
                    return Err(MpdError::NotMpd {
                        found: local.to_string(),
                    });
                }
                let parent_kept = matches!(stack.last(), None | Some(Frame::Kept(_)));
                let frame = if parent_kept && ns.kept() {
                    Frame::Kept(Elem {
                        ns,
                        name: local.to_string(),
                        attrs: reader.attrs(e, limits)?,
                        children: Vec::new(),
                        text: String::new(),
                    })
                } else {
                    Frame::Skipped
                };
                if empty {
                    attach(frame, &mut stack, &mut root);
                } else {
                    stack.push(frame);
                }
            }
            Event::End(_) => match stack.pop() {
                Some(frame) => attach(frame, &mut stack, &mut root),
                None => return Err(reader.fail("unexpected end tag")),
            },
            Event::DocType(_) => return Err(reader.fail("DOCTYPE declarations are not allowed")),
            Event::Eof => {
                if !stack.is_empty() {
                    return Err(reader.fail("unexpected end of input inside an element"));
                }
                return root.ok_or_else(|| reader.fail("no root element"));
            }
            _ => {}
        }
    }
}

fn attach(frame: Frame, stack: &mut [Frame], root: &mut Option<Elem>) {
    let Frame::Kept(elem) = frame else { return };
    match stack.last_mut() {
        Some(Frame::Kept(parent)) => parent.children.push(elem),
        Some(Frame::Skipped) => {}
        None => *root = Some(elem),
    }
}
