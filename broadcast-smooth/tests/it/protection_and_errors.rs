//! Protection child selection, malformed-document reporting and the
//! user-facing error text (MS-SSTR §2.2.2.2).

use broadcast_common::xml_chars::XmlLimit;
use broadcast_smooth::{SmoothManifest, SmoothParseError};

#[test]
fn the_first_protection_element_wins() {
    let xml = r#"<SmoothStreamingMedia>
        <Protection><ProtectionHeader SystemID="first">AAA</ProtectionHeader></Protection>
        <Protection><ProtectionHeader SystemID="second">BBB</ProtectionHeader></Protection>
    </SmoothStreamingMedia>"#;
    let m = SmoothManifest::parse(xml).unwrap();
    assert_eq!(m.protection.unwrap().header.unwrap(), "AAA");
}

#[test]
fn a_non_protection_sibling_does_not_replace_the_protection() {
    let xml = r#"<SmoothStreamingMedia>
        <Protection><ProtectionHeader SystemID="only">AAA</ProtectionHeader></Protection>
        <StreamIndex Type="video" Url="u"><QualityLevel Index="0" Bitrate="1" FourCC="H264"/></StreamIndex>
    </SmoothStreamingMedia>"#;
    let m = SmoothManifest::parse(xml).unwrap();
    assert_eq!(m.protection.unwrap().header.unwrap(), "AAA");
    assert_eq!(m.streams.len(), 1);
}

#[test]
fn the_first_protection_header_child_wins() {
    let xml = r#"<SmoothStreamingMedia>
        <Protection>
            <ProtectionHeader SystemID="first">AAA</ProtectionHeader>
            <ProtectionHeader SystemID="second">BBB</ProtectionHeader>
        </Protection>
    </SmoothStreamingMedia>"#;
    let m = SmoothManifest::parse(xml).unwrap();
    assert_eq!(m.protection.unwrap().header.unwrap(), "AAA");
}

#[test]
fn malformed_xml_reports_the_reader_position_and_message() {
    let err =
        SmoothManifest::parse("<SmoothStreamingMedia><a></SmoothStreamingMedia>").unwrap_err();
    assert_eq!(
        err,
        SmoothParseError::Xml {
            pos: 48,
            message:
                "ill-formed document: expected `</a>`, but `</SmoothStreamingMedia>` was found"
                    .into(),
        }
    );
}

#[test]
fn parse_error_display_is_exact() {
    assert_eq!(
        SmoothParseError::UnexpectedElement {
            expected: "SmoothStreamingMedia",
            found: "x".into(),
        }
        .to_string(),
        "expected element <SmoothStreamingMedia>, found <x>"
    );
    assert_eq!(
        SmoothParseError::UnexpectedElement {
            expected: "SmoothStreamingMedia",
            found: String::new(),
        }
        .to_string(),
        "expected element <SmoothStreamingMedia>, found none"
    );
    assert_eq!(
        SmoothParseError::InvalidHex { value: "zz".into() }.to_string(),
        "invalid CodecPrivateData hex \"zz\""
    );
    assert_eq!(
        SmoothParseError::Xml {
            pos: 7,
            message: "boom".into(),
        }
        .to_string(),
        "XML error at byte offset 7: boom"
    );
}

#[test]
fn parse_error_display_pins_every_remaining_variant() {
    assert_eq!(
        SmoothParseError::MissingAttribute {
            element: "QualityLevel",
            attr: "Bitrate",
        }
        .to_string(),
        "<QualityLevel> is missing required attribute @Bitrate"
    );
    assert_eq!(
        SmoothParseError::InvalidAttributeValue {
            element: "StreamIndex",
            attr: "Type",
            value: "x".into(),
        }
        .to_string(),
        "<StreamIndex>@Type has invalid value \"x\""
    );
    assert_eq!(
        SmoothParseError::InvalidRepeatCount { r: 0 }.to_string(),
        "<c>@r has invalid value 0 (must be >= 1)"
    );
    assert_eq!(
        SmoothParseError::MissingDuration { index: 3 }.to_string(),
        "<c> at index 3 duration cannot be derived"
    );
    assert_eq!(
        SmoothParseError::IndivisibleRun { index: 4 }.to_string(),
        "<c> at index 4 run does not divide evenly"
    );
    assert_eq!(
        SmoothParseError::NonMonotonicTime { index: 5 }.to_string(),
        "<c> at index 5 has non-monotonic time"
    );
    assert_eq!(
        SmoothParseError::ChunkRunTooLong {
            count_hint: 100_001
        }
        .to_string(),
        "c@r repeat run exceeded max chunk count (100001 > 100000)"
    );
    assert_eq!(
        SmoothParseError::CodecPrivateDataTooLong { len: 70_000 }.to_string(),
        "CodecPrivateData hex length 70000 exceeds max 65536"
    );
    assert_eq!(
        SmoothParseError::TooLarge {
            limit: XmlLimit::Depth,
            max: 32,
        }
        .to_string(),
        "manifest exceeds the depth limit of 32"
    );
}
