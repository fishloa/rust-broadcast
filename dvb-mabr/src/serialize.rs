//! Manual XML serialization helpers.
//!
//! No dependency on an XML-writer crate — every element type builds its own
//! fragment with `String`/`write!` (the same approach as `ttml-subtitle`).
//! Output uses 2-space indentation and self-closes elements with no
//! children/attributes-only content; it is not byte-identical to whatever
//! produced the input (attribute order, whitespace, and comments are not
//! preserved), only structurally round-trippable.

extern crate alloc;

use alloc::string::String;
use core::fmt::Write as _;

/// Push `level` two-space indents onto `out`.
pub(crate) fn push_indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("  ");
    }
}

/// Escape the five XML 1.0 predefined entities. Used for both text content
/// and attribute values (over-escaping `'`/`"` in text content is harmless).
pub(crate) fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Write a required `name="value"` attribute.
pub(crate) fn write_attr(out: &mut String, name: &str, value: &str) {
    let _ = write!(out, " {name}=\"{}\"", xml_escape(value));
}

/// Write an optional `name="value"` attribute, only if `Some`.
pub(crate) fn write_opt_attr(out: &mut String, name: &str, value: Option<&str>) {
    if let Some(v) = value {
        write_attr(out, name, v);
    }
}

/// Write an optional numeric attribute via its `Display` impl (no escaping
/// needed — digits only).
pub(crate) fn write_opt_num_attr<T: core::fmt::Display>(
    out: &mut String,
    name: &str,
    value: Option<T>,
) {
    if let Some(v) = value {
        let _ = write!(out, " {name}=\"{v}\"");
    }
}

/// Write a required numeric attribute via its `Display` impl.
pub(crate) fn write_num_attr<T: core::fmt::Display>(out: &mut String, name: &str, value: T) {
    let _ = write!(out, " {name}=\"{value}\"");
}

/// Write an optional boolean attribute as `"true"`/`"false"` (xs:boolean).
pub(crate) fn write_opt_bool_attr(out: &mut String, name: &str, value: Option<bool>) {
    if let Some(v) = value {
        let _ = write!(out, " {name}=\"{}\"", if v { "true" } else { "false" });
    }
}
