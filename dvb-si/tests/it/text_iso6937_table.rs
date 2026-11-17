//! EN 300 468 Annex A Figure A.1 default-Latin (ISO/IEC 6937) combining row.
//!
//! The oracle is the private tier's `text/iso6937_combining.tsv`: one row per
//! defined non-spacing prefix (0xC1..=0xCF) and printable base byte, with the
//! expected Unicode code points computed from Unicode NFC (see that topic's
//! `PROVENANCE.md`), never from the table under test. It is spec-derived, so it
//! lives in the private repo; without that checkout the table test skips loudly
//! and the literal-letter tests still run.

use dvb_si::text::{decode_dvb_string, try_decode_dvb_string};

fn code_points(s: &str) -> Vec<u32> {
    s.chars().map(u32::from).collect()
}

fn parse_code_points(field: &str) -> Vec<u32> {
    field
        .split(' ')
        .map(|cp| u32::from_str_radix(cp.trim_start_matches("U+"), 16).unwrap())
        .collect()
}

/// The private oracle table, or `None` (after a loud notice) without the
/// private checkout. A present checkout that lacks the table is a hard failure.
fn oracle_table() -> Option<String> {
    let Some(root) = test_fixtures::private_root() else {
        eprintln!(
            "SKIP every_combining_pair_matches_the_unicode_oracle: private fixtures absent \
             (the `private` submodule is not checked out); \
             `git submodule update --init private` (maintainers only)"
        );
        return None;
    };
    let path = root.join("fixtures/text/iso6937_combining.tsv");
    assert!(
        path.is_file(),
        "private checkout present but {} is missing",
        path.display()
    );
    Some(std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

/// Every (prefix, base) pair of the figure's non-spacing row decodes exactly
/// as the independent Unicode oracle says: a precomposed letter where ISO 6937
/// defines one, otherwise the base letter followed by the combining mark.
#[test]
fn every_combining_pair_matches_the_unicode_oracle() {
    let Some(tsv) = oracle_table() else {
        return;
    };
    let mut precomposed = 0usize;
    let mut fallback = 0usize;
    for line in tsv.lines().skip(1) {
        let mut fields = line.split('\t');
        let prefix = u8::from_str_radix(fields.next().unwrap(), 16).unwrap();
        let base = u8::from_str_radix(fields.next().unwrap(), 16).unwrap();
        let want = parse_code_points(fields.next().unwrap());
        let got = code_points(&decode_dvb_string(&[prefix, base]));
        assert_eq!(got, want, "prefix {prefix:#04X} base {base:#04X}");
        if want.len() == 1 {
            precomposed += 1;
        } else {
            fallback += 1;
        }
    }
    assert_eq!(precomposed, 141, "precomposed pairs in Figure A.1");
    assert_eq!(fallback, 13 * 95 - 141, "base + combining mark pairs");
}

/// Named accented letters a DVB service name or EPG title carries, pinned as
/// literals (one per prefix row).
#[test]
fn named_accented_letters_decode_literally() {
    assert_eq!(decode_dvb_string(&[0x00, 0xC1, b'e']), "è");
    assert_eq!(decode_dvb_string(&[0x00, 0xC2, b'A']), "Á");
    assert_eq!(decode_dvb_string(&[0x00, 0xC3, b'A']), "Â");
    assert_eq!(decode_dvb_string(&[0x00, 0xC4, b'n']), "ñ");
    assert_eq!(decode_dvb_string(&[0x00, 0xC5, b'a']), "ā");
    assert_eq!(decode_dvb_string(&[0x00, 0xC6, b'g']), "ğ");
    assert_eq!(decode_dvb_string(&[0x00, 0xC7, b'z']), "ż");
    assert_eq!(decode_dvb_string(&[0x00, 0xC8, b'o']), "ö");
    assert_eq!(decode_dvb_string(&[0x00, 0xCA, b'a']), "å");
    assert_eq!(decode_dvb_string(&[0x00, 0xCB, b'c']), "ç");
    assert_eq!(decode_dvb_string(&[0x00, 0xCD, b'o']), "ő");
    assert_eq!(decode_dvb_string(&[0x00, 0xCE, b'a']), "ą");
    assert_eq!(decode_dvb_string(&[0x00, 0xCF, b's']), "š");
}

/// The three grey (undefined) prefixes are not combining marks: each decodes
/// to U+FFFD and the following letter stands alone.
#[test]
fn undefined_combining_prefixes_decode_to_replacement_then_base() {
    for prefix in [0xC0u8, 0xC9, 0xCC] {
        assert_eq!(
            decode_dvb_string(&[0x00, prefix, b'a']),
            "\u{FFFD}a",
            "prefix {prefix:#04X}"
        );
    }
    // A defined mark with no base letter left is also a replacement.
    assert_eq!(decode_dvb_string(&[0x00, 0xC8]), "\u{FFFD}");
}

/// Annex A.2 emphasis on/off (0x86/0x87) is stripped from the text.
#[test]
fn emphasis_control_codes_are_stripped_between_letters() {
    assert_eq!(decode_dvb_string(&[0x00, 0x86, b'a', 0x87]), "a");
    assert_eq!(decode_dvb_string(&[0x00, b'a', 0x86, b'b', 0x87]), "ab");
}

/// The `0x10 0x00 nn` three-byte form selects ISO 8859-nn; anything else
/// starting 0x10 is unsupported and must not index past the buffer.
#[test]
fn extended_selector_names_the_iso8859_part() {
    // 0x05 = ISO/IEC 8859-5 (Cyrillic): 0xB0 is CYRILLIC CAPITAL LETTER A.
    assert_eq!(decode_dvb_string(&[0x10, 0x00, 0x05, b'x', 0xB0]), "xА");
    // Second byte not 0x00: not the extended form. Both body bytes are U+FFFD.
    assert_eq!(decode_dvb_string(&[0x10, 0x05, b'x']), "\u{FFFD}\u{FFFD}");
    // Too short for the three-byte form: no panic, one U+FFFD body byte.
    assert_eq!(decode_dvb_string(&[0x10, 0x00]), "\u{FFFD}");
    assert_eq!(try_decode_dvb_string(&[0x10, 0x00]).unwrap(), "\u{FFFD}");
}

/// Two-character samples of each `encoding_rs`-backed legacy charset.
#[cfg(feature = "legacy-charsets")]
#[test]
fn two_character_legacy_samples() {
    // EUC-KR: HANGUL SYLLABLE GA, NA.
    assert_eq!(decode_dvb_string(&[0x12, 0xB0, 0xA1, 0xB3, 0xAA]), "가나");
    // GB-2312/GBK: ni, hao.
    assert_eq!(decode_dvb_string(&[0x13, 0xC4, 0xE3, 0xBA, 0xC3]), "你好");
    // Big5: zhong, wen.
    assert_eq!(decode_dvb_string(&[0x14, 0xA4, 0xA4, 0xA4, 0xE5]), "中文");
}
