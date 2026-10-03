//! The single place the manifest parsers decode and validate XML character data.
//!
//! [`crate::dash_parse`] and [`crate::smooth_parse`] read `quick_xml` events;
//! every text run, CDATA section and entity/character reference they see —
//! whether in a modelled element, between elements, or inside a subtree the
//! parser skips — goes through [`char_data`], so nothing is validated in one
//! path and waved through in another. Decoding is quick-xml's
//! (`BytesText::xml10_content`, `BytesRef::resolve_char_ref`,
//! `escape::resolve_predefined_entity`); this module only enforces XML 1.0 §2.2.

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;

/// XML 1.0 §2.2 `Char` production: the only characters a document may
/// contain, whether written literally or through a character reference.
pub(crate) fn is_xml_char(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\r'
            | '\u{20}'..='\u{D7FF}'
            | '\u{E000}'..='\u{FFFD}'
            | '\u{10000}'..='\u{10FFFF}'
    )
}

/// `Ok(Some(text))` for a text run, CDATA section or entity/character
/// reference (references resolved); `Ok(None)` for every other event; `Err`
/// with a description for an undefined entity, an invalid character reference,
/// or a character outside the `Char` production.
pub(crate) fn char_data(event: &Event<'_>) -> Result<Option<String>, String> {
    let content = match event {
        Event::Text(t) => t.xml10_content().into_owned(),
        Event::CData(c) => c.xml10_content().into_owned(),
        Event::GeneralRef(r) => match r.resolve_char_ref() {
            Ok(Some(c)) => c.to_string(),
            Ok(None) => match resolve_predefined_entity(r) {
                Some(s) => s.to_string(),
                None => return Err(format!("undefined entity reference &{};", &**r)),
            },
            Err(e) => return Err(e.to_string()),
        },
        _ => return Ok(None),
    };
    if content.chars().all(is_xml_char) {
        Ok(Some(content))
    } else {
        Err("character not allowed in XML 1.0 (Char production, §2.2)".to_string())
    }
}
