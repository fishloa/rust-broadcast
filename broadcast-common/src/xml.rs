//! Bounded namespace-aware XML reading into a small element tree.
//!
//! quick-xml does the lexing, entity and namespace resolution. This layer adds
//! the hostile-input bounds ([`XmlLimits`]), refuses DOCTYPE declarations (so
//! no entity can be declared, hence no entity-expansion attack), and validates
//! every character against the XML 1.0 `Char` production. Every element below
//! the root is kept with its own namespace recorded; only a foreign element at
//! the root is skipped whole, so a host schema's unqualified children and its
//! foreign-namespace extensions are both readable through [`Elem::attr_in`].

use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use quick_xml::NsReader;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;

use crate::xml_chars::{LimitExceeded, XmlBudget, XmlLimits};

/// One way an element or attribute name resolves: the caller's target
/// namespace, no namespace, or a foreign one. An owner classifies each URI it
/// understands so the reader can record it, and passes the target namespace,
/// which is the only namespace a root element may have.
pub trait Namespace: Copy + Eq {
    /// The target namespace: its elements are kept in the tree.
    const TARGET: Self;

    /// No namespace at all.
    const NONE: Self;

    /// Classify a resolved namespace URI (an unprefixed attribute resolves to
    /// no namespace and is never classified).
    fn classify(uri: &str) -> Self;
}

/// One attribute of a start tag, namespace declarations excluded.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Attr<N> {
    /// The attribute's own namespace (`N::NONE` for an unprefixed attribute).
    pub ns: N,
    /// Its local name.
    pub name: String,
    /// Its normalized, unescaped value.
    pub value: String,
}

/// One element of the tree: its namespace, local name, attributes, kept
/// children and concatenated character data.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Elem<N> {
    /// The element's namespace.
    pub ns: N,
    /// Its local name.
    pub name: String,
    /// Its attributes, in document order.
    pub attrs: Vec<Attr<N>>,
    /// Its children, in document order.
    pub children: Vec<Elem<N>>,
    /// Its direct character data (text nodes that are immediate
    /// children of this element); descendant text is not included.
    pub text: String,
}

impl<N: Namespace> Elem<N> {
    /// The value of the unprefixed attribute `name`.
    #[must_use]
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attr_in(N::NONE, name)
    }

    /// The value of the attribute `name` in namespace `ns`.
    #[must_use]
    pub fn attr_in(&self, ns: N, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.ns == ns && a.name == name)
            .map(|a| a.value.as_str())
    }

    /// Every direct child element named `name` in the target namespace.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Elem<N>> + 'a {
        self.children
            .iter()
            .filter(move |c| c.ns == N::TARGET && c.name == name)
    }

    /// The first direct child element named `name` in the target namespace.
    #[must_use]
    pub fn child(&self, name: &str) -> Option<&Elem<N>> {
        self.children
            .iter()
            .find(|c| c.ns == N::TARGET && c.name == name)
    }

    /// Whether a direct child element named `name` exists in the target
    /// namespace.
    #[must_use]
    pub fn has_child(&self, name: &str) -> bool {
        self.child(name).is_some()
    }

    /// All direct character data inside this element (descendant text
    /// is not included).
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// Why [`read_tree`] rejected a document.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum XmlError {
    /// The bytes are not UTF-8.
    InvalidUtf8 {
        /// Length of the valid prefix.
        valid_up_to: usize,
    },
    /// The document is not well-formed XML, or uses a construct this reader
    /// refuses (a DOCTYPE).
    NotWellFormed {
        /// Byte offset where the reader stopped.
        position: usize,
        /// What was wrong.
        message: String,
    },
    /// The root element's local name is not `root`.
    WrongRoot {
        /// The local name actually found.
        found: String,
    },
    /// A hostile-input bound was exceeded.
    Limit(LimitExceeded),
}

impl Namespace for () {
    const TARGET: Self = ();
    const NONE: Self = ();

    fn classify(_uri: &str) -> Self {}
}

const BOM: char = '\u{FEFF}';

struct Reader<'a, N> {
    inner: NsReader<&'a [u8]>,
    budget: XmlBudget,
    marker: core::marker::PhantomData<N>,
}

fn ns_of<N: Namespace>(result: ResolveResult<'_>) -> Result<N, String> {
    match result {
        ResolveResult::Unbound => Ok(N::NONE),
        ResolveResult::Bound(ns) => Ok(N::classify(ns.into_inner())),
        ResolveResult::Unknown(prefix) => Err(format!("unknown namespace prefix '{prefix}'")),
    }
}

impl<N: Namespace> Reader<'_, N> {
    fn attrs(&self, e: &BytesStart<'_>) -> Result<Vec<Attr<N>>, XmlError> {
        let attributes = crate::xml_chars::read_attributes_checked(
            &self.inner,
            e,
            |m| self.fail(m),
            |count, value| self.admit_attribute(count, value),
        )?;
        Ok(attributes
            .into_iter()
            .map(|a| Attr {
                ns: a.namespace.as_deref().map_or(N::NONE, N::classify),
                name: a.name,
                value: a.value,
            })
            .collect())
    }

    fn admit_attribute(&self, count: usize, value: &str) -> Result<(), XmlError> {
        self.budget
            .admit_attribute(count, value)
            .map_err(XmlError::Limit)
    }

    fn char_data(&self, event: &Event<'_>) -> Result<Option<String>, XmlError> {
        crate::xml_chars::char_data(event).map_err(|m| self.fail(m))
    }

    fn fail(&self, message: impl Into<String>) -> XmlError {
        XmlError::NotWellFormed {
            position: usize::try_from(self.inner.buffer_position()).unwrap_or(usize::MAX),
            message: message.into(),
        }
    }
}

/// A start tag that has not been closed yet: the kept element under
/// construction, or the marker for a foreign root that is skipped whole.
enum Frame<N> {
    Kept(Elem<N>),
    Branch,
}

/// Read `xml` into an element tree rooted at `root` in namespace `N::TARGET`.
///
/// The root must be `root` in `N::TARGET` (else [`XmlError::WrongRoot`]); it
/// and every element below it are kept with their resolved namespace recorded.
/// `limits` bounds the document size, depth, element count and attribute
/// size/count.
///
/// # Errors
/// [`XmlError`] for invalid UTF-8, a malformed or refused document, a root
/// element that is not `root` in the target namespace, or an exceeded bound.
pub fn read_tree<N: Namespace>(
    xml: &[u8],
    root: &str,
    limits: &XmlLimits,
) -> Result<Elem<N>, XmlError> {
    let budget = XmlBudget::new(limits.clone());
    budget.check_input(xml.len()).map_err(XmlError::Limit)?;
    let text = core::str::from_utf8(xml).map_err(|e| XmlError::InvalidUtf8 {
        valid_up_to: e.valid_up_to(),
    })?;
    let text = text.strip_prefix(BOM).unwrap_or(text);
    let mut reader = Reader::<N> {
        inner: quick_xml::NsReader::from_str(text),
        budget,
        marker: core::marker::PhantomData,
    };

    let mut stack: Vec<Frame<N>> = Vec::new();
    let mut doc_root: Option<Elem<N>> = None;

    loop {
        let (ns_of_event, event) = match reader.inner.read_resolved_event() {
            Ok((ns, event)) => (ns_of::<N>(ns), event),
            Err(e) => return Err(reader.fail(e.to_string())),
        };
        if let Some(piece) = reader.char_data(&event)? {
            if stack.is_empty() {
                if matches!(event, Event::CData(_)) {
                    return Err(reader.fail("character data outside the root element"));
                }
                if piece
                    .chars()
                    .any(|c| !matches!(c, ' ' | '\t' | '\r' | '\n'))
                {
                    return Err(reader.fail("character data outside the root element"));
                }
            }
            match stack.last_mut() {
                Some(Frame::Kept(top)) => top.text.push_str(&piece),
                Some(Frame::Branch) => {}
                None => {}
            }
            continue;
        }
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let is_empty = matches!(event, Event::Empty(_));
                if doc_root.is_some() && stack.is_empty() {
                    return Err(reader.fail("more than one root element"));
                }
                reader.budget.open_element().map_err(XmlError::Limit)?;
                if is_empty {
                    reader.budget.close_element();
                }
                let local = e.local_name().into_inner();
                let resolved = ns_of_event.map_err(|m| reader.fail(m))?;
                if stack.is_empty() && (resolved != N::TARGET || local != root) {
                    return Err(XmlError::WrongRoot {
                        found: local.to_owned(),
                    });
                }
                let keep_element = resolved != N::TARGET || local == root || !stack.is_empty();
                let frame = if keep_element {
                    Frame::Kept(Elem {
                        ns: resolved,
                        name: local.to_string(),
                        attrs: reader.attrs(e)?,
                        children: Vec::new(),
                        text: String::new(),
                    })
                } else {
                    Frame::Branch
                };
                if is_empty {
                    attach(frame, &mut stack, &mut doc_root);
                } else {
                    stack.push(frame);
                }
            }
            Event::End(_) => match stack.pop() {
                Some(frame) => {
                    reader.budget.close_element();
                    attach(frame, &mut stack, &mut doc_root);
                }
                None => return Err(reader.fail("unexpected end tag")),
            },
            Event::DocType(_) => return Err(reader.fail("DOCTYPE declarations are not allowed")),
            Event::Eof => {
                if !stack.is_empty() {
                    return Err(reader.fail("unexpected end of input inside an element"));
                }
                return doc_root.ok_or_else(|| reader.fail("no root element"));
            }
            _ => {}
        }
    }
}

fn attach<N: Namespace>(frame: Frame<N>, stack: &mut [Frame<N>], root: &mut Option<Elem<N>>) {
    let Frame::Kept(elem) = frame else {
        return;
    };
    match stack.last_mut() {
        Some(Frame::Kept(parent)) => {
            parent.children.push(elem);
        }
        Some(Frame::Branch) => {}
        None => *root = Some(elem),
    }
}

impl From<LimitExceeded> for XmlError {
    fn from(e: LimitExceeded) -> Self {
        Self::Limit(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Ns {
        None,
        Target,
        Other,
    }

    impl Namespace for Ns {
        const TARGET: Self = Ns::Target;
        const NONE: Self = Ns::None;

        fn classify(uri: &str) -> Self {
            match uri {
                "urn:target" => Self::Target,
                _ => Self::Other,
            }
        }
    }

    fn read(xml: &str) -> Result<Elem<Ns>, XmlError> {
        read_tree(xml.as_bytes(), "Root", &XmlLimits::default())
    }

    #[test]
    fn foreign_children_keep_their_namespace_and_children_survive() {
        let tree = read(
            r#"<Root xmlns="urn:target" xmlns:x="urn:other" a="1" x:b="2"><x:Child><Child>t</Child></x:Child></Root>"#,
        )
        .unwrap();
        assert_eq!(tree.attr("a"), Some("1"));
        assert_eq!(tree.attr_in(Ns::Other, "b"), Some("2"));
        assert_eq!(tree.children.len(), 1);
        assert_eq!(tree.children[0].ns, Ns::Other);
        assert_eq!(tree.children[0].name, "Child");
        assert_eq!(tree.children[0].child("Child").unwrap().text(), "t");
    }

    #[test]
    fn wrong_namespace_root_is_reported() {
        let err = read(r#"<Root xmlns="urn:other"/>"#).unwrap_err();
        assert!(matches!(err, XmlError::WrongRoot { .. }), "{err:?}");
    }

    #[test]
    fn doctype_is_refused() {
        let err = read(r#"<!DOCTYPE x [<!ENTITY e "v">]><Root>&e;</Root>"#).unwrap_err();
        assert!(matches!(err, XmlError::NotWellFormed { .. }), "{err:?}");
    }

    #[test]
    fn namespace_less_elements_and_text() {
        let tree = read(r#"<Root xmlns="urn:target">a<B>c</B>b</Root>"#).unwrap();
        assert_eq!(tree.text(), "ab");
        assert_eq!(tree.child("B").unwrap().text(), "c");
    }

    #[test]
    fn mixed_text_pieces_accumulate() {
        let tree = read(r#"<Root xmlns="urn:target">abc&amp;defab<![CDATA[<x>]]>cda&#13;b</Root>"#)
            .unwrap();
        assert_eq!(tree.text(), "abc&defab<x>cda\rb");
    }
}

#[cfg(test)]
mod xml_ws_tests {
    use super::*;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Ns {
        None,
        Other,
    }
    impl Namespace for Ns {
        const TARGET: Self = Ns::None;
        const NONE: Self = Ns::None;
        fn classify(_uri: &str) -> Self {
            Ns::Other
        }
    }

    #[test]
    fn leading_nbsp_rejected() {
        let xml = "\u{00A0}<Root/>";
        let res = read_tree::<Ns>(xml.as_bytes(), "Root", &XmlLimits::default());
        assert!(res.is_err());
    }

    #[test]
    fn trailing_cdata_rejected() {
        let xml = "<Root/><![CDATA[x]]>";
        let res = read_tree::<Ns>(xml.as_bytes(), "Root", &XmlLimits::default());
        assert!(res.is_err());
    }

    #[test]
    fn leading_xml_s_accepted() {
        let xml = "\r\n\t <Root/>";
        let res = read_tree::<Ns>(xml.as_bytes(), "Root", &XmlLimits::default());
        assert!(res.is_ok());
    }
}
