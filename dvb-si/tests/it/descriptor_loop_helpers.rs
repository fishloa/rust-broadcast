//! Structural descriptor-loop scans and the DVB text/`LangCode` formatting
//! helpers used by `dvb-tools` output.

use dvb_si::descriptors::DescriptorLoop;
use dvb_si::text::{DvbText, LangCode};

/// Two descriptors, the last exactly filling the loop: the zero-length final
/// entry must still be yielded, and `contains_tag` must reach it.
#[test]
fn raw_tags_walks_a_descriptor_that_ends_exactly_at_the_loop_end() {
    let raw = [0x48u8, 0x03, b'a', b'b', b'c', 0x54, 0x00];
    let loop_ = DescriptorLoop::new(&raw);

    let tags: Vec<(u8, &[u8])> = loop_.raw_tags().collect();
    assert_eq!(tags.len(), 2);
    assert_eq!(tags[0], (0x48, &b"abc"[..]));
    assert_eq!(tags[1], (0x54, &[][..]));

    assert!(loop_.contains_tag(0x54));
    assert!(!loop_.contains_tag(0x55));
    // A one-byte loop is just a tag header: checking it must not read a length.
    assert!(!DescriptorLoop::new(&[0x54]).contains_tag(0x55));
    assert!(DescriptorLoop::new(&[0x54]).contains_tag(0x54));
}

/// A trailing descriptor whose declared length runs past the buffer ends the
/// structural walk without panicking, but its tag byte still counts as
/// present.
#[test]
fn a_truncated_trailing_descriptor_does_not_panic() {
    let raw = [0x48u8, 0x03, b'a', b'b', b'c', 0x54, 0x09, b'x'];
    let loop_ = DescriptorLoop::new(&raw);
    let tags: Vec<(u8, &[u8])> = loop_.raw_tags().collect();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0], (0x48, &b"abc"[..]));
    assert!(loop_.contains_tag(0x54), "the header byte is still present");
}

/// `LangCode` is its three wire bytes in every rendering (`dvb-tools` prints
/// ISO 639 audio/subtitle languages through all three).
#[test]
fn lang_code_renders_its_three_bytes() {
    let lang = LangCode(*b"fra");
    assert_eq!(&*lang, b"fra");
    assert_eq!(lang.as_str(), "fra");
    assert_eq!(format!("{lang}"), "fra");
    assert_eq!(format!("{lang:?}"), "LangCode(fra)");
}

/// `DvbText`'s Debug is the decoded string, not the raw selector bytes.
#[test]
fn dvb_text_debug_shows_the_decoded_string() {
    assert_eq!(format!("{:?}", DvbText::new(b"abc")), "DvbText(\"abc\")");
    assert_eq!(
        format!("{:?}", DvbText::new(&[0xC2, b'e'])),
        "DvbText(\"é\")"
    );
}
