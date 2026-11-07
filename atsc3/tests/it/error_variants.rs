use atsc3::{Error, Slt};

#[test]
fn malformed_xml_is_an_xml_parse_error_carrying_the_parser_message() {
    let xml = "<SLT bsid=\"1\"";
    let reason = roxmltree::Document::parse(xml).unwrap_err().to_string();
    assert_eq!(
        Slt::parse(xml).unwrap_err(),
        Error::XmlParse { what: "SLT", reason }
    );
}
