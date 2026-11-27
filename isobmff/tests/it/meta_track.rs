//! Family 1 of issue #4167: timed-metadata and text-track boxes.
//!
//! Boxes no installed tool writes are assembled from the clause and labelled
//! here; the ones a real file carries are asserted against that file in
//! `tier34_fixtures.rs`. Every parse is serialized back and compared with the
//! bytes it came from.

use broadcast_core::{Parse, Serialize};
use isobmff::init_segment::SampleEntryVariant;
use isobmff::meta_track::{
    BinaryXmlBox, BoxedMetaDataSampleEntry, CopyrightBox, MetadataKeyBox, MetadataKeyChild,
    MetadataKeyDeclarationBox, MetadataKeyTableBox, MetadataLocaleBox, MetadataSetupBox,
    TextConfigBox, TextMetaDataSampleEntry, UriBox, UriInitBox, UriMetaDataSampleEntry, XmlBox,
    XmlMetaDataSampleEntry,
};

fn boxed(fourcc: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
    out.extend_from_slice(&fourcc);
    out.extend_from_slice(body);
    out
}

fn full(version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + body.len());
    out.push(version);
    out.extend_from_slice(&flags.to_be_bytes()[1..]);
    out.extend_from_slice(body);
    out
}

fn sample_entry(fourcc: [u8; 4], fields: &[u8], children: &[u8]) -> Vec<u8> {
    let mut body = vec![0u8; 6];
    body.extend_from_slice(&1u16.to_be_bytes());
    body.extend_from_slice(fields);
    body.extend_from_slice(children);
    boxed(fourcc, &body)
}

#[test]
fn uri_and_init_data_round_trip() {
    let uri_bytes = boxed(*b"uri ", &full(0, 0, b"urn:mpeg:test\0"));
    let uri = UriBox::parse(&uri_bytes).expect("uri");
    assert_eq!(uri.uri_str(), Some("urn:mpeg:test"));
    assert_eq!(uri.try_to_bytes().expect("bytes"), uri_bytes);

    let init = boxed(*b"uriI", &full(0, 0, &[0xDE, 0xAD, 0xBE, 0xEF]));
    let parsed = UriInitBox::parse(&init).expect("uriI");
    assert_eq!(parsed.data(), &[0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), init);
}

#[test]
fn xml_box_round_trips_with_and_without_the_terminator() {
    let text = b"<root/>";
    let mut body = text.to_vec();
    body.push(0);
    let terminated = boxed(*b"xml ", &full(0, 0, &body));
    let parsed = XmlBox::parse(&terminated).expect("xml");
    assert_eq!(parsed.xml(), text);
    assert!(parsed.is_null_terminated());
    assert_eq!(parsed.try_to_bytes().expect("bytes"), terminated);

    let unterminated = boxed(*b"xml ", &full(0, 0, text));
    let parsed = XmlBox::parse(&unterminated).expect("xml without terminator");
    assert_eq!(parsed.xml(), text);
    assert!(!parsed.is_null_terminated());
    assert_eq!(parsed.try_to_bytes().expect("bytes"), unterminated);

    let mut utf16 = vec![0xFF, 0xFE, b'<', 0, b'/', 0, b'>', 0];
    utf16.extend_from_slice(&[0, 0]);
    let bytes = boxed(*b"xml ", &full(0, 0, &utf16));
    let parsed = XmlBox::parse(&bytes).expect("utf16 xml");
    assert_eq!(parsed.xml(), &utf16[..utf16.len() - 2]);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn binary_xml_round_trips() {
    let bytes = boxed(*b"bxml", &full(0, 0, &[1, 2, 3, 4]));
    let parsed = BinaryXmlBox::parse(&bytes).expect("bxml");
    assert_eq!(parsed.data(), &[1, 2, 3, 4]);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn copyright_box_packs_its_language() {
    let bytes = boxed(*b"cprt", &full(0, 0, &[0x55, 0xC4, b'(', b'c', b')', 0]));
    let parsed = CopyrightBox::parse(&bytes).expect("cprt");
    assert_eq!(parsed.language(), 0x55C4);
    assert_eq!(parsed.notice_str(), Some("(c)"));
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn key_declaration_locale_and_setup_round_trip() {
    let mut body = b"mdta".to_vec();
    body.extend_from_slice(b"com.example.key");
    let keyd_bytes = boxed(*b"keyd", &body);
    let keyd = MetadataKeyDeclarationBox::parse(&keyd_bytes).expect("keyd");
    assert_eq!(keyd.key_namespace(), u32::from_be_bytes(*b"mdta"));
    assert_eq!(keyd.key_value_str(), Some("com.example.key"));
    assert_eq!(keyd.try_to_bytes().expect("bytes"), keyd_bytes);

    let loca_bytes = boxed(*b"loca", b"en-US\0");
    let loca = MetadataLocaleBox::parse(&loca_bytes).expect("loca");
    assert_eq!(loca.locale_str(), Some("en-US"));
    assert_eq!(loca.try_to_bytes().expect("bytes"), loca_bytes);

    let setu_bytes = boxed(*b"setu", &[0, 1, 2, 3, 4, 5]);
    let setu = MetadataSetupBox::parse(&setu_bytes).expect("setu");
    assert_eq!(setu.data(), &[0, 1, 2, 3, 4, 5]);
    assert_eq!(setu.try_to_bytes().expect("bytes"), setu_bytes);
}

#[test]
fn a_local_key_box_keeps_its_children_in_order() {
    let mut children = boxed(*b"keyd", b"mdtahello");
    children.extend_from_slice(&boxed(*b"loca", b"fr-FR\0"));
    children.extend_from_slice(&boxed(*b"setu", &[9, 9]));
    let bytes = boxed(7u32.to_be_bytes(), &children);
    let key = MetadataKeyBox::parse(&bytes).expect("local key");
    assert_eq!(key.local_key_id(), 7);
    assert_eq!(key.children().len(), 3);
    assert!(matches!(
        key.children()[0],
        MetadataKeyChild::Declaration(_)
    ));
    assert_eq!(
        key.declaration().and_then(|d| d.key_value_str()),
        Some("hello")
    );
    assert_eq!(key.locale().and_then(|l| l.locale_str()), Some("fr-FR"));
    assert_eq!(key.setup().map(|s| s.data()), Some(&[9u8, 9][..]));
    assert_eq!(key.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn a_key_table_holds_local_key_boxes() {
    let key = boxed(1u32.to_be_bytes(), &boxed(*b"keyd", b"mdtaone"));
    let bytes = boxed(*b"keys", &key);
    let table = MetadataKeyTableBox::parse(&bytes).expect("keys");
    assert_eq!(table.keys().len(), 1);
    assert_eq!(table.keys()[0].local_key_id(), 1);
    assert_eq!(table.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn xml_metadata_sample_entry_round_trips() {
    let mut fields = b"gzip\0urn:example:one urn:example:two\0".to_vec();
    fields.extend_from_slice(b"https://example.test/schema\0");
    let bytes = sample_entry(*b"metx", &fields, &[]);
    let entry = XmlMetaDataSampleEntry::parse(&bytes).expect("metx");
    assert_eq!(entry.content_encoding(), b"gzip");
    assert_eq!(entry.namespace(), b"urn:example:one urn:example:two");
    assert_eq!(
        entry.schema_location(),
        Some(&b"https://example.test/schema"[..])
    );
    assert_eq!(entry.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn text_metadata_sample_entry_round_trips() {
    let txtc = boxed(*b"txtC", &full(0, 0, b"header\0"));
    let bytes = sample_entry(*b"mett", b"utf-8\0text/plain\0", &txtc);
    let entry = TextMetaDataSampleEntry::parse(&bytes).expect("mett");
    assert_eq!(entry.codec_type(), *b"mett");
    assert_eq!(entry.content_encoding(), b"utf-8");
    assert_eq!(entry.mime_format(), b"text/plain");
    assert_eq!(
        entry.text_config().and_then(|c| c.text_config_str()),
        Some("header")
    );
    assert_eq!(entry.try_to_bytes().expect("bytes"), bytes);

    let stxt = sample_entry(*b"stxt", b"\0text/plain\0", &[]);
    let entry = TextMetaDataSampleEntry::parse(&stxt).expect("stxt");
    assert_eq!(entry.codec_type(), *b"stxt");
    assert_eq!(entry.try_to_bytes().expect("bytes"), stxt);
}

#[test]
fn uri_metadata_sample_entry_round_trips() {
    let uri = boxed(*b"uri ", &full(0, 0, b"urn:example:form\0"));
    let init = boxed(*b"uriI", &full(0, 0, &[0xAA]));
    let bytes = sample_entry(*b"urim", &[], &[uri, init].concat());
    let entry = UriMetaDataSampleEntry::parse(&bytes).expect("urim");
    assert_eq!(entry.uri().uri_str(), Some("urn:example:form"));
    assert_eq!(entry.uri_init().map(|i| i.data()), Some(&[0xAAu8][..]));
    assert_eq!(entry.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn boxed_metadata_sample_entry_round_trips() {
    let key = boxed(2u32.to_be_bytes(), &boxed(*b"keyd", b"mdta.two"));
    let keys = boxed(*b"keys", &key);
    let btrt = boxed(*b"btrt", &[0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3]);
    let bytes = sample_entry(*b"mebx", &[], &[keys, btrt].concat());
    let entry = BoxedMetaDataSampleEntry::parse(&bytes).expect("mebx");
    assert_eq!(entry.key_table().keys().len(), 1);
    assert_eq!(entry.other_boxes().len(), 1);
    assert_eq!(entry.try_to_bytes().expect("bytes"), bytes);

    let variant = isobmff::init_segment::sample_entry_variant(&bytes).expect("stsd dispatch");
    let SampleEntryVariant::Mebx(parsed) = &variant else {
        panic!("mebx dispatched as another variant");
    };
    assert_eq!(parsed.key_table().keys()[0].local_key_id(), 2);
}

#[test]
fn empty_string_fields_are_kept_so_the_bytes_round_trip() {
    let bytes = sample_entry(*b"metx", b"\0\0", &[]);
    let entry = XmlMetaDataSampleEntry::parse(&bytes).expect("metx");
    assert!(entry.content_encoding().is_empty());
    assert!(entry.namespace().is_empty());
    assert_eq!(entry.schema_location(), None);
    assert_eq!(entry.try_to_bytes().expect("bytes"), bytes);

    let config = TextConfigBox::parse(&boxed(*b"txtC", &full(0, 0, b"\0"))).expect("txtC");
    assert_eq!(config.text_config_str(), Some(""));
}
