use ttml_subtitle::{Document, Error};

#[test]
fn a_root_without_a_tt_element_is_not_a_ttml_root() {
    assert_eq!(
        Document::parse_str("<html xmlns=\"http://www.w3.org/1999/xhtml\"/>").unwrap_err(),
        Error::NotTtmlRoot("html".to_string())
    );
}

#[test]
fn a_tt_element_in_a_foreign_namespace_is_not_a_ttml_root() {
    assert_eq!(
        Document::parse_str("<tt xmlns=\"urn:example:other\"/>").unwrap_err(),
        Error::NotTtmlRoot("namespace \"urn:example:other\"".to_string())
    );
}
