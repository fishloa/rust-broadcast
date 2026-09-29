//! Foreign (unmodeled) XML content preservation — TTML2 §7.2, §7.3.
//!
//! TTML2 §7.2 ("Foreign Document Elements and Attributes") permits elements
//! and attributes in foreign namespaces to appear anywhere in a TTML
//! document; §7.3 permits additional elements even in the TT Namespace
//! itself. Rather than silently dropping them (the pre-#1110 behaviour),
//! [`Document::parse_str`](crate::Document::parse_str) captures them here and [`Document::to_xml`](crate::Document::to_xml)
//! writes them back:
//!
//! - Unmodeled *attributes* (any namespace outside the TT core, styling,
//!   metadata, parameter, audio and IMSC/EBU/SMPTE extension namespaces this
//!   crate models explicitly) are kept as `(namespace URI, local name,
//!   value)` triples together with the **original prefix binding**
//!   (`xmlns:prefix="uri"`) that was in scope, so a vendor namespace such as
//!   EBU-TT (`urn:ebu:tt:metadata`) is re-emitted with the same prefix.
//! - Unmodeled *child elements* are kept as an [`UnknownElement`] subtree
//!   (name, namespace, attributes, children, text) on the element that
//!   carried them, written back at their original position in document
//!   order.
//!
//! The types are deliberately lossless: nothing is normalized or rewritten,
//! so the serialized document re-parses to an equal tree, and a comparison
//! against the original XML with an independent XML parser (e.g.
//! `roxmltree`) finds the same elements, attributes, namespace URIs and
//! text.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// An unmodeled attribute plus the prefix binding that was in scope.
///
/// The pair `(namespace, local)` identifies the attribute per XML
/// namespace-name semantics; `prefix` records the original
/// `xmlns:prefix="namespace"` declaration so the serializer can re-declare
/// the same prefix instead of inventing one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ForeignAttribute {
    /// The original `xmlns:` prefix bound to this attribute's namespace
    /// (`None` for an unprefixed attribute in no namespace).
    pub prefix: Option<String>,
    /// The attribute's resolved namespace URI (`None` = no namespace).
    pub namespace: Option<String>,
    /// The attribute's local name.
    pub local_name: String,
    /// The attribute's value.
    pub value: String,
    /// `xmlns:` declarations scoped to the element that carried this
    /// attribute (TTML2 §7.2, #1110/TT-W1). roxmltree consumes them, so the
    /// parse records them here; the serializer re-declares each one on the
    /// `<tt>` element (widening a namespace scope is always namespace-
    /// equivalent) and mirrors the binding internally so any original
    /// prefix spelling is reproduced.
    pub scoped_namespaces: Vec<(String, String)>,
}

impl ForeignAttribute {
    /// Create a foreign attribute, optionally recording the original
    /// `xmlns:` prefix binding.
    ///
    /// Without a `prefix` the serializer declares a prefix for the
    /// namespace at write time (a known one such as `ebutts:`, or a
    /// generated `ttmfallbackN`), so nothing is lost either way; supplying
    /// the original prefix preserves its exact spelling when available.
    pub fn new(
        prefix: Option<&str>,
        namespace: Option<&str>,
        local_name: &str,
        value: &str,
    ) -> ForeignAttribute {
        ForeignAttribute {
            prefix: prefix.map(String::from),
            namespace: namespace.map(String::from),
            local_name: String::from(local_name),
            value: String::from(value),
            scoped_namespaces: Vec::new(),
        }
    }
}

/// An unmodeled element, kept as a lossless subtree — TTML2 §7.2/§7.3.
///
/// Every child of a modeled element that the crate does not model (foreign
/// namespaces such as EBU-TT or SMPTE-TT's `<image>`-style extensions,
/// vendor namespaces, unrecognized `tt:`-namespace elements, and — inside
/// content elements — unmodeled *modeled-namespace* elements such as
/// `<audio>`, `<chunk>`, `<data>`, `<font>`, `<resources>` or `<source>`)
/// is captured with its full subtree: the prefix it was written with, its
/// namespace URI, its attributes (including any `xmlns:` declarations
/// scoped to it), child elements and text, all in original document order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct UnknownElement {
    /// The original element-name prefix (e.g. `Some("ebutts")` for
    /// `ebutts:foo`; `None` for an unprefixed name).
    pub prefix: Option<String>,
    /// The element's resolved namespace URI (`None` = no namespace).
    pub namespace: Option<String>,
    /// The element's local name.
    pub local_name: String,
    /// The element's attributes, in document order.
    pub attributes: Vec<ForeignAttribute>,
    /// The element's children, in document order.
    pub children: Vec<UnknownNode>,
    /// `xmlns:` declarations scoped to this element (TTML2 §7.2,
    /// #1110/TT-W1). Always present on elements captured by
    /// `Document::parse_str` (possibly empty); `None` only for subtrees
    /// built through the struct API. The serializer writes them as extra
    /// `xmlns:prefix="uri"` declarations on this element's opening tag and
    /// mirrors the binding internally, so descendants that used the prefix
    /// still resolve and re-parsing reproduces exactly this field.
    pub scoped_namespaces: Option<Vec<(String, String)>>,
}

impl UnknownElement {
    /// Create an empty unknown element, optionally recording the original
    /// `xmlns:` prefix binding (see [`ForeignAttribute::new`]).
    pub fn new(prefix: Option<&str>, namespace: Option<&str>, local_name: &str) -> UnknownElement {
        UnknownElement {
            prefix: prefix.map(String::from),
            namespace: namespace.map(String::from),
            local_name: String::from(local_name),
            attributes: Vec::new(),
            children: Vec::new(),
            scoped_namespaces: None,
        }
    }
}

/// A node inside an [`UnknownElement`] subtree.
///
/// Comments and processing instructions are *not* representable: the
/// serializer never emits them, so a parsed-and-reserialized subtree is
/// stable (re-parsing the output reproduces this exact tree).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnknownNode {
    /// An element child.
    Element(Box<UnknownElement>),
    /// A text node (character data).
    Text(String),
}
