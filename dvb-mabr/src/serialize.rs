//! XML serialization helpers over [`quick_xml::Writer`].
//!
//! Every element is written with quick-xml's `BytesStart`/`BytesEnd`/
//! `BytesText` events through an indenting [`Writer`], so every attribute
//! value and text node is escaped by quick-xml (an attribute value containing
//! `\t`/`\n`/`\r` is written as a character reference so it survives
//! attribute-value normalization on re-parse). Output uses 2-space indentation
//! and self-closes elements that have no children; it is not byte-identical to
//! whatever produced the input (attribute order, whitespace, and comments are
//! not preserved), only structurally round-trippable.

extern crate alloc;

use alloc::string::{String, ToString};
use std::io;

use core::fmt::Display;

use broadcast_common::xml_chars::{XmlWriter, finish_document, new_writer, write_declaration};
use quick_xml::events::{BytesStart, BytesText, Event};

/// The writer every `write_xml` appends to (an in-memory `Vec`, so no write can
/// actually fail; the `io::Result`s exist only to let `?` chain).
pub(crate) type Out = XmlWriter;

/// Render a document: the `<?xml version="1.0" encoding="UTF-8"?>` declaration,
/// whatever `body` writes through the 2-space indenting writer, and a trailing
/// newline. Writing to the in-memory `Vec` cannot fail, so an `io::Error` (which
/// cannot occur) would only truncate the output.
pub(crate) fn document<F>(body: F) -> String
where
    F: FnOnce(&mut Out) -> io::Result<()>,
{
    let mut w = new_writer();
    write_declaration(&mut w, "UTF-8");
    let _ = body(&mut w);
    finish_document(w)
}

/// A start tag for `name`.
pub(crate) fn tag(name: &str) -> BytesStart<'_> {
    BytesStart::new(name)
}

/// Add a required `name="value"` attribute (value escaped by quick-xml).
pub(crate) fn attr(tag: &mut BytesStart<'_>, name: &str, value: &str) {
    tag.push_attribute((name, value));
}

/// Add an optional `name="value"` attribute, only if `Some`.
pub(crate) fn opt_attr(tag: &mut BytesStart<'_>, name: &str, value: Option<&str>) {
    if let Some(v) = value {
        attr(tag, name, v);
    }
}

/// Add a numeric attribute via its `Display` impl.
pub(crate) fn num_attr<T: Display>(tag: &mut BytesStart<'_>, name: &str, value: T) {
    attr(tag, name, &value.to_string());
}

/// Add an optional numeric attribute, only if `Some`.
pub(crate) fn opt_num_attr<T: Display>(tag: &mut BytesStart<'_>, name: &str, value: Option<T>) {
    if let Some(v) = value {
        num_attr(tag, name, v);
    }
}

/// Add an optional boolean attribute as `"true"`/`"false"` (xs:boolean).
pub(crate) fn opt_bool_attr(tag: &mut BytesStart<'_>, name: &str, value: Option<bool>) {
    if let Some(v) = value {
        attr(tag, name, if v { "true" } else { "false" });
    }
}

/// `<tag .../>`.
pub(crate) fn empty(w: &mut Out, tag: BytesStart<'_>) -> io::Result<()> {
    w.write_event(Event::Empty(tag))
}

/// `<tag ...>text</tag>` on one line, the text escaped by quick-xml.
pub(crate) fn text_element(w: &mut Out, tag: BytesStart<'_>, text: &str) -> io::Result<()> {
    let end = tag.to_end().into_owned();
    w.write_event(Event::Start(tag))?;
    w.write_event(Event::Text(BytesText::new(text)))?;
    w.write_event(Event::End(end))
}

/// `<tag ...>` + children (written by `body`) + `</tag>`, indented.
pub(crate) fn element<F>(w: &mut Out, tag: BytesStart<'_>, body: F) -> io::Result<()>
where
    F: FnOnce(&mut Out) -> io::Result<()>,
{
    let end = tag.to_end().into_owned();
    w.write_event(Event::Start(tag))?;
    body(w)?;
    w.write_event(Event::End(end))
}
