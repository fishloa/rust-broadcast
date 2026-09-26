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

/// The first baseline-namespaced element child with the given local name, if
/// any.
pub(crate) fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|n| {
        n.is_element()
            && n.tag_name().name() == name
            && is_baseline_namespace(n.tag_name().namespace())
    })
}

/// All baseline-namespaced element children with the given local name, in
/// document order.
pub(crate) fn children<'a, 'i>(
    node: Node<'a, 'i>,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children().filter(move |n| {
        n.is_element()
            && n.tag_name().name() == name
            && is_baseline_namespace(n.tag_name().namespace())
    })
}

/// Trimmed text content of a named child element, if that child is present.
pub(crate) fn child_text(node: Node<'_, '_>, name: &str) -> Option<String> {
    child(node, name).map(own_text)
}

/// Trimmed text content of this element itself (its element content, not
/// its attributes) — used for leaf elements whose value is a URI/string
/// (`PresentationManifestLocator`, `ReportingLocator`, `ResourceLocator`,
/// `BaseURL`, the macro elements). An empty element yields `""`.
///
/// Concatenates every direct text-node child rather than using
/// [`Node::text`], which returns only the *first* one: a comment or CDATA
/// section between two text runs (e.g. `<BaseURL>a<!-- x -->b</BaseURL>`)
/// would otherwise silently truncate the value at the comment (MABR-W4,
/// #1121).
pub(crate) fn own_text(node: Node<'_, '_>) -> String {
    let mut s = String::new();
    for child in node.children() {
        if child.is_text()
            && let Some(t) = child.text()
        {
            s.push_str(t);
        }
    }
    s.trim().to_string()
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
