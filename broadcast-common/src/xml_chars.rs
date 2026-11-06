//! XML 1.0 character-data validation over `quick-xml` events.
//!
//! Every crate that reads XML through `quick-xml` decodes character data here,
//! so a text run, CDATA section or entity/character reference is validated the
//! same way in a modelled element, between elements, and inside a subtree the
//! parser skips. Decoding is quick-xml's (`BytesText::xml10_content`,
//! `BytesRef::resolve_char_ref`, `escape::resolve_predefined_entity`); this
//! module enforces the XML 1.0 §2.2 `Char` production on the result.

use alloc::format;
use alloc::string::{String, ToString};

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;

/// Message for a character outside the XML 1.0 §2.2 `Char` production.
pub const BAD_CHAR: &str = "character not allowed in XML 1.0 (Char production, §2.2)";

/// XML 1.0 §2.2 `Char` production: the only characters a document may
/// contain, whether written literally or through a character reference.
pub fn is_xml_char(c: char) -> bool {
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
/// or a character outside the `Char` production ([`BAD_CHAR`]).
pub fn char_data(event: &Event<'_>) -> Result<Option<String>, String> {
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
        Err(BAD_CHAR.to_string())
    }
}

/// The namespace URI an element or attribute name resolved to: `None` when
/// unbound, `Err` with a description for an undeclared prefix.
pub fn namespace_of(result: ResolveResult<'_>) -> Result<Option<String>, String> {
    match result {
        ResolveResult::Unbound => Ok(None),
        ResolveResult::Bound(ns) => Ok(Some(ns.into_inner().to_string())),
        ResolveResult::Unknown(prefix) => Err(format!("unknown namespace prefix '{prefix}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quick_xml::NsReader;

    fn first_char_data(xml: &str) -> Result<Option<String>, String> {
        let mut reader = NsReader::from_str(xml);
        loop {
            let event = reader.read_event().map_err(|e| e.to_string())?;
            if matches!(event, Event::Eof) {
                return Ok(None);
            }
            if let Some(text) = char_data(&event)? {
                return Ok(Some(text));
            }
        }
    }

    #[test]
    fn char_production_boundaries() {
        assert!(is_xml_char('\t') && is_xml_char('\u{10FFFF}'));
        assert!(!is_xml_char('\u{0}') && !is_xml_char('\u{FFFE}') && !is_xml_char('\u{B}'));
    }

    #[test]
    fn resolves_predefined_and_numeric_references() {
        assert_eq!(
            first_char_data("<a>&amp;</a>").unwrap().as_deref(),
            Some("&")
        );
        assert_eq!(
            first_char_data("<a>&#x41;</a>").unwrap().as_deref(),
            Some("A")
        );
    }

    #[test]
    fn rejects_undefined_entity_and_forbidden_character_reference() {
        assert!(
            first_char_data("<a>&nope;</a>")
                .unwrap_err()
                .contains("undefined entity")
        );
        assert!(first_char_data("<a>&#1;</a>").is_err());
    }

    #[test]
    fn cdata_and_text_pass_through() {
        assert_eq!(
            first_char_data("<a><![CDATA[x<y]]></a>")
                .unwrap()
                .as_deref(),
            Some("x<y")
        );
    }

    #[test]
    fn unknown_prefix_is_an_error() {
        let mut reader = NsReader::from_str("<p:a/>");
        let (ns, _) = reader.read_resolved_event().unwrap();
        assert!(
            namespace_of(ns)
                .unwrap_err()
                .contains("unknown namespace prefix")
        );
    }
}
