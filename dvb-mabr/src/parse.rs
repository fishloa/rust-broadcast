//! Shared XML parsing helpers and namespace constants — ETSI TS 103 769
//! V1.2.1 Annex A (baseline schema + extensibility mechanism).
//!
//! Elements are matched by local name only, never namespace-qualified: both
//! the schema-version-1 (`:2019:`) and schema-version-2 (`:2024:`) baseline
//! namespaces use the same element/attribute local names (Annex A.0-1), and
//! a private/implementation extension element in a third, unknown namespace
//! is silently skipped wherever it appears (Annex A.1) rather than rejected.

extern crate alloc;

use alloc::string::{String, ToString};

use roxmltree::Node;

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

/// The first element child with the given local name, if any.
pub(crate) fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}

/// All element children with the given local name, in document order.
pub(crate) fn children<'a, 'i>(
    node: Node<'a, 'i>,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children()
        .filter(move |n| n.is_element() && n.tag_name().name() == name)
}

/// Trimmed text content of a named child element, if that child is present.
pub(crate) fn child_text(node: Node<'_, '_>, name: &str) -> Option<String> {
    child(node, name).map(own_text)
}

/// Trimmed text content of this element itself (its element content, not
/// its attributes) — used for leaf elements whose value is a URI/string
/// (`PresentationManifestLocator`, `ReportingLocator`, `ResourceLocator`,
/// `BaseURL`, the macro elements). An empty element yields `""`.
pub(crate) fn own_text(node: Node<'_, '_>) -> String {
    node.text().unwrap_or("").trim().to_string()
}

/// An unprefixed (no-namespace) attribute's raw string value — every MABR
/// attribute except `xsi:type`.
pub(crate) fn attr<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attribute(name)
}

pub(crate) fn attr_owned(node: Node<'_, '_>, name: &str) -> Option<String> {
    attr(node, name).map(ToString::to_string)
}

/// The `xsi:type` attribute's local name (any namespace prefix on the
/// *value* itself is stripped — only the local type name distinguishes the
/// `ServiceComponentIdentifier` variants, clause 10.2.4).
pub(crate) fn xsi_type(node: Node<'_, '_>) -> Option<String> {
    node.attributes()
        .find(|a| a.namespace() == Some(NS_XSI) && a.name() == "type")
        .map(|a| match a.value().rsplit_once(':') {
            Some((_, local)) => local.to_string(),
            None => a.value().to_string(),
        })
}

pub(crate) fn require_attr(
    node: Node<'_, '_>,
    element: &'static str,
    name: &'static str,
) -> Result<String> {
    attr_owned(node, name).ok_or(Error::MissingAttribute {
        element,
        attr: name,
    })
}

pub(crate) fn require_child<'a, 'i>(
    node: Node<'a, 'i>,
    parent: &'static str,
    name: &'static str,
) -> Result<Node<'a, 'i>> {
    child(node, name).ok_or(Error::MissingElement {
        parent,
        child: name,
    })
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
    value
        .trim()
        .parse::<f64>()
        .map_err(|_| invalid(element, attr_name, value, "expected a decimal number"))
}

pub(crate) fn req_attr_u32(
    node: Node<'_, '_>,
    element: &'static str,
    name: &'static str,
) -> Result<u32> {
    parse_u32(element, name, &require_attr(node, element, name)?)
}

pub(crate) fn req_attr_u64(
    node: Node<'_, '_>,
    element: &'static str,
    name: &'static str,
) -> Result<u64> {
    parse_u64(element, name, &require_attr(node, element, name)?)
}

pub(crate) fn opt_attr_u32(
    node: Node<'_, '_>,
    element: &'static str,
    name: &'static str,
) -> Result<Option<u32>> {
    match attr(node, name) {
        Some(v) => Ok(Some(parse_u32(element, name, v)?)),
        None => Ok(None),
    }
}

pub(crate) fn opt_attr_u64(
    node: Node<'_, '_>,
    element: &'static str,
    name: &'static str,
) -> Result<Option<u64>> {
    match attr(node, name) {
        Some(v) => Ok(Some(parse_u64(element, name, v)?)),
        None => Ok(None),
    }
}

pub(crate) fn opt_attr_bool(
    node: Node<'_, '_>,
    element: &'static str,
    name: &'static str,
) -> Result<Option<bool>> {
    match attr(node, name) {
        Some(v) => Ok(Some(parse_bool(element, name, v)?)),
        None => Ok(None),
    }
}

pub(crate) fn opt_attr_f64(
    node: Node<'_, '_>,
    element: &'static str,
    name: &'static str,
) -> Result<Option<f64>> {
    match attr(node, name) {
        Some(v) => Ok(Some(parse_f64(element, name, v)?)),
        None => Ok(None),
    }
}
