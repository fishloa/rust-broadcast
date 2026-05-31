//! DVB-SI text decoding — ETSI EN 300 468 Annex A.
//!
//! Lift-and-shift port of zenith's `src/si/dvb_text.rs`. Full Annex A
//! table coverage (all 17 charsets, emphasis pairs, extended language
//! blocks) lands in Phase 1; this subset matches zenith's current
//! behaviour byte-for-byte so migration carries zero regression.

use std::borrow::Cow;

/// Decode a DVB text payload (e.g. short_event_descriptor event_name_char)
/// into an owned UTF-8 `String`. The first byte may be a charset indicator
/// per ETSI EN 300 468 Annex A Table A.3.
#[must_use]
pub fn decode_dvb_string(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    let (charset, body) = split_charset(bytes);
    let decoded = match charset {
        Charset::Iso6937 => decode_iso_6937(body),
        Charset::Iso8859(n) => decode_iso_8859(n, body),
        Charset::Utf8 => String::from_utf8_lossy(body).into_owned(),
        Charset::Ucs2Be => decode_ucs2_be(body),
        Charset::Unsupported(_indicator) => body.iter().map(|_| '\u{FFFD}').collect(),
    };

    // Annex A.2 control codes:
    //   0x86 emphasis on, 0x87 emphasis off, 0x8A CR/LF -> space.
    //   Other C0/C1 controls are stripped per zenith's pre-existing behaviour.
    decoded
        .chars()
        .filter_map(|c| match c as u32 {
            0x86 | 0x87 => None,
            0x8A => Some(' '),
            0x0A => Some(' '),
            code if code < 0x20 => None,
            code if (0x80..0xA0).contains(&code) => None,
            _ => Some(c),
        })
        .collect()
}

/// Convenience wrapper returning `Cow::Borrowed` for pure-ASCII input,
/// `Cow::Owned` otherwise.
#[must_use]
pub fn decode(bytes: &[u8]) -> Cow<'_, str> {
    if bytes.iter().all(|&b| b.is_ascii() && b >= 0x20) {
        return Cow::Borrowed(std::str::from_utf8(bytes).unwrap_or(""));
    }
    Cow::Owned(decode_dvb_string(bytes))
}

#[derive(Debug)]
enum Charset {
    Iso6937,
    Iso8859(u8),
    Utf8,
    Ucs2Be,
    Unsupported(u8),
}

fn split_charset(bytes: &[u8]) -> (Charset, &[u8]) {
    match bytes[0] {
        b if b >= 0x20 => (Charset::Iso6937, bytes),
        0x00 => (Charset::Iso6937, &bytes[1..]),
        0x01..=0x0B => (Charset::Iso8859(bytes[0] + 4), &bytes[1..]),
        0x10 if bytes.len() >= 3 && bytes[1] == 0x00 => {
            (Charset::Iso8859(bytes[2]), &bytes[3..])
        }
        0x11 => (Charset::Ucs2Be, &bytes[1..]),
        0x15 => (Charset::Utf8, &bytes[1..]),
        other => (Charset::Unsupported(other), &bytes[1..]),
    }
}

fn decode_iso_6937(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if (0xC0..=0xCF).contains(&b) && i + 1 < bytes.len() {
            let base = bytes[i + 1];
            if let Some(c) = combine(b, base) {
                out.push(c);
                i += 2;
                continue;
            }
        }
        out.push(iso_6937_single(b));
        i += 1;
    }
    out
}

fn iso_6937_single(b: u8) -> char {
    match b {
        0x00..=0x7F => b as char,
        // Preserve ETSI Annex A.2 C1 control codes so the post-filter can act on them.
        0x86 | 0x87 | 0x8A => b as char,
        0x80..=0xA0 => '\u{FFFD}',
        0xA4 => '¤',
        0xA8 => '¤',
        0xFB => 'ß',
        0xFC => 'œ',
        0xFD => 'ŕ',
        0xFE => '\u{FFFD}',
        0xFF => '\u{FFFD}',
        other => other as char,
    }
}

fn combine(prefix: u8, base: u8) -> Option<char> {
    Some(match (prefix, base) {
        (0xC1, b'A') => 'À', (0xC1, b'E') => 'È', (0xC1, b'I') => 'Ì',
        (0xC1, b'O') => 'Ò', (0xC1, b'U') => 'Ù',
        (0xC1, b'a') => 'à', (0xC1, b'e') => 'è', (0xC1, b'i') => 'ì',
        (0xC1, b'o') => 'ò', (0xC1, b'u') => 'ù',
        (0xC2, b'A') => 'Á', (0xC2, b'E') => 'É', (0xC2, b'I') => 'Í',
        (0xC2, b'O') => 'Ó', (0xC2, b'U') => 'Ú', (0xC2, b'Y') => 'Ý',
        (0xC2, b'a') => 'á', (0xC2, b'e') => 'é', (0xC2, b'i') => 'í',
        (0xC2, b'o') => 'ó', (0xC2, b'u') => 'ú', (0xC2, b'y') => 'ý',
        (0xC3, b'A') => 'Â', (0xC3, b'E') => 'Ê', (0xC3, b'I') => 'Î',
        (0xC3, b'O') => 'Ô', (0xC3, b'U') => 'Û',
        (0xC3, b'a') => 'â', (0xC3, b'e') => 'ê', (0xC3, b'i') => 'î',
        (0xC3, b'o') => 'ô', (0xC3, b'u') => 'û',
        (0xC4, b'A') => 'Ã', (0xC4, b'N') => 'Ñ', (0xC4, b'O') => 'Õ',
        (0xC4, b'a') => 'ã', (0xC4, b'n') => 'ñ', (0xC4, b'o') => 'õ',
        (0xC8, b'A') => 'Ä', (0xC8, b'E') => 'Ë', (0xC8, b'I') => 'Ï',
        (0xC8, b'O') => 'Ö', (0xC8, b'U') => 'Ü', (0xC8, b'Y') => 'Ÿ',
        (0xC8, b'a') => 'ä', (0xC8, b'e') => 'ë', (0xC8, b'i') => 'ï',
        (0xC8, b'o') => 'ö', (0xC8, b'u') => 'ü', (0xC8, b'y') => 'ÿ',
        (0xCB, b'C') => 'Ç', (0xCB, b'c') => 'ç',
        _ => return None,
    })
}

fn decode_iso_8859(n: u8, bytes: &[u8]) -> String {
    use encoding_rs::*;
    let encoding: &'static Encoding = match n {
        2 => ISO_8859_2,
        3 => ISO_8859_3,
        4 => ISO_8859_4,
        5 => ISO_8859_5,
        6 => ISO_8859_6,
        7 => ISO_8859_7,
        8 => ISO_8859_8,
        9 => WINDOWS_1254,
        10 => ISO_8859_10,
        11 => WINDOWS_874,
        13 => ISO_8859_13,
        14 => ISO_8859_14,
        15 => ISO_8859_15,
        _ => return bytes.iter().map(|&b| b as char).collect(),
    };
    let (cow, _, _) = encoding.decode(bytes);
    cow.into_owned()
}

fn decode_ucs2_be(bytes: &[u8]) -> String {
    let code_units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16_lossy(&code_units)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_empty_input_returns_empty_string() {
        assert_eq!(decode_dvb_string(&[]), "");
    }

    #[test]
    fn decode_plain_ascii_is_borrowed() {
        let cow = decode(b"HELLO");
        assert!(matches!(cow, Cow::Borrowed(_)));
        assert_eq!(cow, "HELLO");
    }

    #[test]
    fn decode_iso6937_latin_accent_chars() {
        assert_eq!(decode_dvb_string(&[0x00, 0xC2, b'A']), "Á");
        assert_eq!(decode_dvb_string(&[0x00, 0xC1, b'e']), "è");
        assert_eq!(decode_dvb_string(&[0x00, 0xC8, b'o']), "ö");
    }

    #[test]
    fn decode_selector_0x01_yields_iso8859_5_cyrillic() {
        let s = decode_dvb_string(&[0x01, 0xB0, 0xB1]);
        assert!(s.chars().all(|c| c != '\u{FFFD}'), "got: {s:?}");
        assert!(!s.is_empty());
    }

    #[test]
    fn decode_selector_0x10_extended_yields_iso8859_nn() {
        let s = decode_dvb_string(&[0x10, 0x00, 0x09, b'A', b'B']);
        assert_eq!(s, "AB");
    }

    #[test]
    fn decode_selector_0x11_ucs2_be() {
        let s = decode_dvb_string(&[0x11, 0x00, 0x41, 0x00, 0x42]);
        assert_eq!(s, "AB");
    }

    #[test]
    fn decode_selector_0x15_utf8_passthrough() {
        let s = decode_dvb_string(&[0x15, 0xC3, 0xA9, 0xC3, 0xA9]);
        assert_eq!(s, "éé");
    }

    #[test]
    fn decode_control_chars_stripped_linefeed_becomes_space() {
        let s = decode_dvb_string(b"A\x01B\nC");
        assert_eq!(s, "AB C");
    }

    #[test]
    fn emphasis_on_off_markers_stripped_per_annex_a2() {
        // 0x86 and 0x87 are emphasis on/off markers per ETSI Annex A.2 — not
        // representable in plain text, strip silently.
        let s = decode_dvb_string(&[0x00, b'A', 0x86, b'B', 0x87, b'C']);
        assert_eq!(s, "ABC");
    }

    #[test]
    fn decode_annex_a2_crlf_0x8a_becomes_space() {
        // 0x8A in DVB text maps to CR/LF per Annex A.2 — render as space.
        let s = decode_dvb_string(&[0x00, b'A', 0x8A, b'B']);
        assert_eq!(s, "A B");
    }

    #[test]
    fn unknown_selector_returns_replacement_characters() {
        // Selector 0x1F is reserved/unsupported — each byte becomes U+FFFD.
        let s = decode_dvb_string(&[0x1F, 0xAA, 0xBB, 0xCC]);
        assert_eq!(s.chars().count(), 3);
        assert!(s.chars().all(|c| c == '\u{FFFD}'));
    }
}
