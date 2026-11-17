#![cfg(feature = "xml")]

//! The XML reader's hostile-input bounds and the element-tree lookup helpers
//! (audit 21, #2923): an attribute-count or attribute-size cap that is not
//! enforced lets a hostile manifest carry an unbounded attribute list, and a
//! lookup that ignores the namespace resolves the wrong element or attribute.

use core::fmt::Write as _;

use broadcast_common::xml::{Elem, Namespace, XmlError, read_tree};
use broadcast_common::xml_chars::{LimitExceeded, XmlLimit, XmlLimits};

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

fn read_with(xml: &str, limits: &XmlLimits) -> Result<Elem<Ns>, XmlError> {
    read_tree(xml.as_bytes(), "Root", limits)
}

fn with_attributes(count: usize) -> String {
    let mut doc = String::from("<Root xmlns=\"urn:target\"");
    for index in 0..count {
        write!(doc, " a{index}=\"v\"").expect("a String never fails a write");
    }
    doc.push_str("/>");
    doc
}

fn with_attribute_value(len: usize) -> String {
    format!("<Root xmlns=\"urn:target\" a=\"{}\"/>", "x".repeat(len))
}

#[test]
fn an_element_may_carry_exactly_the_attribute_cap() {
    let mut limits = XmlLimits::default();
    limits.max_attributes = 4;
    assert!(
        read_with(&with_attributes(4), &limits).is_ok(),
        "the cap itself is allowed"
    );
    assert_eq!(
        read_with(&with_attributes(5), &limits).err(),
        Some(XmlError::Limit(LimitExceeded {
            limit: XmlLimit::Attributes,
            max: 4,
        }))
    );
    let mut default_limits = XmlLimits::default();
    default_limits.max_attributes = 256;
    assert!(read_with(&with_attributes(256), &default_limits).is_ok());
    assert_eq!(
        read_with(&with_attributes(257), &default_limits).err(),
        Some(XmlError::Limit(LimitExceeded {
            limit: XmlLimit::Attributes,
            max: 256,
        }))
    );
}

#[test]
fn an_attribute_value_may_be_exactly_the_byte_cap() {
    let mut limits = XmlLimits::default();
    limits.max_attribute_bytes = 8;
    assert!(
        read_with(&with_attribute_value(8), &limits).is_ok(),
        "the cap itself is allowed"
    );
    assert_eq!(
        read_with(&with_attribute_value(9), &limits).err(),
        Some(XmlError::Limit(LimitExceeded {
            limit: XmlLimit::AttributeBytes,
            max: 8,
        }))
    );
}

#[test]
fn an_element_lookup_matches_on_namespace_and_name() {
    let tree = read(
        r#"<Root xmlns="urn:target" xmlns:o="urn:other" a="1" o:a="2"><a>first</a><a>second</a><o:a>foreign</o:a><o:b/></Root>"#,
    )
    .expect("a well-formed document inside every bound");
    assert_eq!(tree.child("a").map(Elem::text), Some("first"));
    assert_eq!(tree.children_named("a").count(), 2);
    assert_eq!(tree.children_named("b").count(), 0);
    assert!(tree.has_child("a"));
    assert!(!tree.has_child("b"));
    assert_eq!(tree.child("b"), None);
    assert_eq!(
        tree.attr("a"),
        Some("1"),
        "an unprefixed attribute has no namespace"
    );
    assert_eq!(tree.attr_in(Ns::None, "a"), Some("1"));
    assert_eq!(tree.attr_in(Ns::Other, "a"), Some("2"));
    assert_eq!(tree.attr_in(Ns::Target, "a"), None);
    assert_eq!(tree.attr_in(Ns::None, "y"), None);
}

#[test]
fn a_foreign_child_keeps_its_own_namespace() {
    let tree = read(r#"<Root xmlns="urn:target" xmlns:o="urn:other"><o:a>x</o:a></Root>"#)
        .expect("a well-formed document");
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].ns, Ns::Other);
    assert_eq!(tree.children[0].name, "a");
    assert_eq!(tree.children[0].text(), "x");
}
