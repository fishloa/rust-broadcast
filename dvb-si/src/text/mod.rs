//! DVB-SI text decoding — ETSI EN 300 468 Annex A.
//!
//! Covers the full Annex A Table A.3 selector set: the default Latin table
//! (Figure A.1, an ISO 6937 superset — see `iso_6937_single`), ISO 8859-n
//! (single-byte 0x01–0x0B and extended 0x10 forms), UCS-2 BE (0x11),
//! KS X 1001 Korean (0x12, decoded as EUC-KR), GB-2312 Simplified Chinese
//! (0x13, decoded via GBK which is a GB-2312 superset), Big5 Traditional
//! Chinese (0x14), UTF-8 (0x15), and the 0x1F `encoding_type_id` escape
//! (no ids are registered for broadcast use — yields U+FFFD). Reserved
//! selectors (0x08, 0x0C–0x0F, 0x16–0x1E) yield U+FFFD per byte.
//!
//! Glyph mappings are pinned to EN 300 468 V1.19.1 (2025-02) Figure A.1
//! "Character code table 00 - Latin alphabet with Unicode equivalents"
//! (PDF p. 159, vendored at `specs/etsi_en_300_468_v01.19.01_dvb_si.pdf`;
//! transcription in `dvb-si/docs/en_300_468.md`).
//!
//! [`DvbText`] wraps the raw wire bytes and decodes only on demand — parsing
//! stays zero-copy; decoding happens when you call [`DvbText::decode`], `Display`,
//! or serde:
//!
//! ```
//! use dvb_si::text::{DvbText, LangCode};
//!
//! // Leading 0x15 is the Annex A UTF-8 selector; "café" follows.
//! let name = DvbText::new(&[0x15, b'c', b'a', b'f', 0xC3, 0xA9]);
//! assert_eq!(name.decode(), "café");
//! assert_eq!(name.raw(), &[0x15, b'c', b'a', b'f', 0xC3, 0xA9]); // selector kept
//!
//! // A selector-less default-Latin (ISO 6937) sequence: combining acute + e → é.
//! assert_eq!(DvbText::new(&[0xC2, b'e']).decode(), "é");
//!
//! // LangCode is 3 raw bytes (ISO 639-2 / ISO 3166) decoded lossily on demand.
//! assert_eq!(LangCode(*b"fre").as_str(), "fre");
//! ```

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
        Charset::Ksx1001 => decode_with(encoding_rs::EUC_KR, body),
        Charset::Gb2312 => decode_with(encoding_rs::GBK, body),
        Charset::Big5 => decode_with(encoding_rs::BIG5, body),
        Charset::Unsupported(_indicator) => body.iter().map(|_| '\u{FFFD}').collect(),
    };

    // Annex A.1 control codes:
    //   single-byte tables: 0x86 emphasis on, 0x87 emphasis off, 0x8A CR/LF
    //   -> space; other C0/C1 controls are stripped.
    //   two-byte tables (Table A.2): the same functions live at U+E086 /
    //   U+E087 / U+E08A inside the ISO 10646 PUA; the rest of
    //   U+E080..U+E09F is reserved for control functions and stripped.
    decoded
        .chars()
        .filter_map(|c| match c as u32 {
            0x86 | 0x87 | 0xE086 | 0xE087 => None,
            0x8A | 0xE08A => Some(' '),
            0x0A => Some(' '),
            code if code < 0x20 => None,
            code if (0x80..0xA0).contains(&code) => None,
            code if (0xE080..0xE0A0).contains(&code) => None,
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

/// Borrowed DVB-encoded text (EN 300 468 Annex A). Wraps the raw selector +
/// body bytes; decoding happens only on [`DvbText::decode`] / `Display` /
/// serde — never in the parse hot path.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DvbText<'a>(&'a [u8]);

impl<'a> DvbText<'a> {
    /// Wrap raw Annex A bytes (charset selector included, if any).
    #[must_use]
    pub const fn new(raw: &'a [u8]) -> Self {
        Self(raw)
    }
    /// The raw wire bytes, selector included.
    #[must_use]
    pub const fn raw(&self) -> &'a [u8] {
        self.0
    }
    /// Decode per Annex A (Table A.3 selector + control codes). Borrows only
    /// for selector-less printable-ASCII input; any charset selector byte
    /// forces an owned decode.
    #[must_use]
    pub fn decode(&self) -> Cow<'a, str> {
        decode(self.0)
    }
}

impl std::ops::Deref for DvbText<'_> {
    /// Derefs to the raw wire bytes (selector included) — `len()`/indexing are
    /// byte counts for serialization, not decoded character counts.
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.0
    }
}

impl std::fmt::Display for DvbText<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.decode())
    }
}

impl std::fmt::Debug for DvbText<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DvbText({:?})", self.decode())
    }
}

impl<'a> From<&'a [u8]> for DvbText<'a> {
    fn from(raw: &'a [u8]) -> Self {
        Self(raw)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for DvbText<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.decode())
    }
}
// Serialize-only: re-encoding decoded text into DVB charset bytes is lossy.
// Structs holding DvbText derive Serialize only; re-parse from wire bytes.

/// ISO 639-2 language code or ISO 3166 country code — 3 raw bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct LangCode(pub [u8; 3]);

impl LangCode {
    /// The code as a string; lossy (U+FFFD) for non-ASCII garbage.
    #[must_use]
    pub fn as_str(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.0)
    }
}

impl std::ops::Deref for LangCode {
    type Target = [u8; 3];
    fn deref(&self) -> &[u8; 3] {
        &self.0
    }
}

impl std::fmt::Display for LangCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.as_str())
    }
}

impl std::fmt::Debug for LangCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LangCode({})", self.as_str())
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for LangCode {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.as_str())
    }
}

#[derive(Debug)]
enum Charset {
    Iso6937,
    Iso8859(u8),
    Utf8,
    Ucs2Be,
    /// KS X 1001 (selector 0x12), decoded as EUC-KR.
    Ksx1001,
    /// GB-2312 (selector 0x13), decoded via GBK (a GB-2312 superset).
    Gb2312,
    /// Big5 (selector 0x14).
    Big5,
    Unsupported(u8),
}

fn split_charset(bytes: &[u8]) -> (Charset, &[u8]) {
    match bytes[0] {
        b if b >= 0x20 => (Charset::Iso6937, bytes),
        0x00 => (Charset::Iso6937, &bytes[1..]),
        // Table A.3: 0x01..=0x0B map to ISO 8859-5..-15, EXCEPT 0x08 which is
        // "reserved for future use" (there is no ISO 8859-12).
        0x08 => (Charset::Unsupported(0x08), &bytes[1..]),
        0x01..=0x0B => (Charset::Iso8859(bytes[0] + 4), &bytes[1..]),
        0x10 if bytes.len() >= 3 && bytes[1] == 0x00 => (Charset::Iso8859(bytes[2]), &bytes[3..]),
        0x11 => (Charset::Ucs2Be, &bytes[1..]),
        0x12 => (Charset::Ksx1001, &bytes[1..]),
        0x13 => (Charset::Gb2312, &bytes[1..]),
        0x14 => (Charset::Big5, &bytes[1..]),
        0x15 => (Charset::Utf8, &bytes[1..]),
        // 0x1F: an 8-bit encoding_type_id follows (Table A.4 area); no ids are
        // registered for broadcast text — treat the body as undecodable.
        0x1F if bytes.len() >= 2 => (Charset::Unsupported(0x1F), &bytes[2..]),
        other => (Charset::Unsupported(other), &bytes[1..]),
    }
}

fn decode_iso_6937(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        // 0xC0..=0xCF is the Figure A.1 non-spacing (combining-prefix) row.
        if (0xC0..=0xCF).contains(&b) {
            match combining_mark(b) {
                Some(mark) if i + 1 < bytes.len() => {
                    let base = bytes[i + 1];
                    if let Some(c) = combine(b, base) {
                        out.push(c);
                    } else {
                        // No precomposed form — emit base + Unicode combining
                        // mark, which is canonically equivalent.
                        out.push(iso_6937_single(base));
                        out.push(mark);
                    }
                    i += 2;
                }
                // Undefined prefix (0xC0/0xC9/0xCC) or dangling prefix at end.
                _ => {
                    out.push('\u{FFFD}');
                    i += 1;
                }
            }
            continue;
        }
        out.push(iso_6937_single(b));
        i += 1;
    }
    out
}

/// Decode a single (non-combining) byte of the default Latin table.
///
/// Source: ETSI EN 300 468 V1.19.1 (2025-02) Figure A.1 — "Character code
/// table 00 - Latin alphabet with Unicode equivalents" (PDF p. 159). Per the
/// note under the figure, the table is a superset of ISO/IEC 6937 with the
/// Euro symbol (U+20AC) added at position 0xA4. Grey (undefined) positions
/// decode to U+FFFD.
fn iso_6937_single(b: u8) -> char {
    match b {
        0x00..=0x7F => b as char,
        // Preserve ETSI Annex A.2 C1 control codes so the post-filter can act on them.
        0x86 | 0x87 | 0x8A => b as char,
        0x80..=0x9F => '\u{FFFD}',
        0xA0 => '\u{00A0}', // NBSP
        0xA1 => '¡',
        0xA2 => '¢',
        0xA3 => '£',
        0xA4 => '\u{20AC}', // € — DVB addition (note under Figure A.1)
        0xA5 => '¥',
        0xA6 => '\u{FFFD}', // undefined
        0xA7 => '§',
        0xA8 => '\u{00A4}', // ¤ general currency sign
        0xA9 => '\u{2018}', // ' left single quotation mark
        0xAA => '\u{201C}', // " left double quotation mark
        0xAB => '«',
        0xAC => '\u{2190}', // ←
        0xAD => '\u{2191}', // ↑
        0xAE => '\u{2192}', // →
        0xAF => '\u{2193}', // ↓
        0xB0 => '°',
        0xB1 => '±',
        0xB2 => '²',
        0xB3 => '³',
        0xB4 => '\u{00D7}', // ×
        0xB5 => 'µ',
        0xB6 => '¶',
        0xB7 => '·',
        0xB8 => '\u{00F7}', // ÷
        0xB9 => '\u{2019}', // ' right single quotation mark
        0xBA => '\u{201D}', // " right double quotation mark
        0xBB => '»',
        0xBC => '¼',
        0xBD => '½',
        0xBE => '¾',
        0xBF => '¿',
        // Combining-prefix row; reached only for a dangling/undefined prefix.
        0xC0..=0xCF => '\u{FFFD}',
        0xD0 => '\u{2015}', // ― horizontal bar
        0xD1 => '¹',
        0xD2 => '®',
        0xD3 => '©',
        0xD4 => '\u{2122}', // ™
        0xD5 => '\u{266A}', // ♪ eighth note
        0xD6 => '¬',
        0xD7 => '\u{00A6}',        // ¦ broken bar
        0xD8..=0xDB => '\u{FFFD}', // undefined
        0xDC => '\u{215B}',        // ⅛
        0xDD => '\u{215C}',        // ⅜
        0xDE => '\u{215D}',        // ⅝
        0xDF => '\u{215E}',        // ⅞
        0xE0 => '\u{2126}',        // Ω OHM SIGN
        0xE1 => 'Æ',
        0xE2 => '\u{0110}', // Đ
        0xE3 => 'ª',
        0xE4 => '\u{0126}', // Ħ
        0xE5 => '\u{FFFD}', // undefined
        0xE6 => '\u{0132}', // Ĳ
        0xE7 => '\u{013F}', // Ŀ
        0xE8 => '\u{0141}', // Ł
        0xE9 => 'Ø',
        0xEA => '\u{0152}', // Œ
        0xEB => 'º',
        0xEC => 'Þ',
        0xED => '\u{0166}', // Ŧ
        0xEE => '\u{014A}', // Ŋ
        0xEF => '\u{0149}', // ŉ
        0xF0 => '\u{0138}', // ĸ
        0xF1 => 'æ',
        0xF2 => '\u{0111}', // đ
        0xF3 => 'ð',
        0xF4 => '\u{0127}', // ħ
        0xF5 => '\u{0131}', // ı dotless i
        0xF6 => '\u{0133}', // ĳ
        0xF7 => '\u{0140}', // ŀ
        0xF8 => '\u{0142}', // ł
        0xF9 => 'ø',
        0xFA => '\u{0153}', // œ
        0xFB => 'ß',
        0xFC => '\u{00FE}', // þ
        0xFD => '\u{0167}', // ŧ
        0xFE => '\u{014B}', // ŋ
        0xFF => '\u{00AD}', // SHY soft hyphen
    }
}

/// Unicode combining mark for a Figure A.1 non-spacing prefix byte
/// (row 0xC0..=0xCF). `None` for the undefined positions 0xC0/0xC9/0xCC.
fn combining_mark(prefix: u8) -> Option<char> {
    Some(match prefix {
        0xC1 => '\u{0300}', // grave
        0xC2 => '\u{0301}', // acute
        0xC3 => '\u{0302}', // circumflex
        0xC4 => '\u{0303}', // tilde
        0xC5 => '\u{0304}', // macron
        0xC6 => '\u{0306}', // breve
        0xC7 => '\u{0307}', // dot above
        0xC8 => '\u{0308}', // diaeresis
        0xCA => '\u{030A}', // ring above
        0xCB => '\u{0327}', // cedilla
        0xCD => '\u{030B}', // double acute
        0xCE => '\u{0328}', // ogonek
        0xCF => '\u{030C}', // caron
        _ => return None,
    })
}

fn combine(prefix: u8, base: u8) -> Option<char> {
    Some(match (prefix, base) {
        (0xC1, b'A') => 'À',
        (0xC1, b'E') => 'È',
        (0xC1, b'I') => 'Ì',
        (0xC1, b'O') => 'Ò',
        (0xC1, b'U') => 'Ù',
        (0xC1, b'a') => 'à',
        (0xC1, b'e') => 'è',
        (0xC1, b'i') => 'ì',
        (0xC1, b'o') => 'ò',
        (0xC1, b'u') => 'ù',
        (0xC2, b'A') => 'Á',
        (0xC2, b'E') => 'É',
        (0xC2, b'I') => 'Í',
        (0xC2, b'O') => 'Ó',
        (0xC2, b'U') => 'Ú',
        (0xC2, b'Y') => 'Ý',
        (0xC2, b'a') => 'á',
        (0xC2, b'e') => 'é',
        (0xC2, b'i') => 'í',
        (0xC2, b'o') => 'ó',
        (0xC2, b'u') => 'ú',
        (0xC2, b'y') => 'ý',
        (0xC2, b'C') => 'Ć',
        (0xC2, b'c') => 'ć',
        (0xC2, b'L') => 'Ĺ',
        (0xC2, b'l') => 'ĺ',
        (0xC2, b'N') => 'Ń',
        (0xC2, b'n') => 'ń',
        (0xC2, b'R') => 'Ŕ',
        (0xC2, b'r') => 'ŕ',
        (0xC2, b'S') => 'Ś',
        (0xC2, b's') => 'ś',
        (0xC2, b'Z') => 'Ź',
        (0xC2, b'z') => 'ź',
        (0xC3, b'A') => 'Â',
        (0xC3, b'E') => 'Ê',
        (0xC3, b'I') => 'Î',
        (0xC3, b'O') => 'Ô',
        (0xC3, b'U') => 'Û',
        (0xC3, b'a') => 'â',
        (0xC3, b'e') => 'ê',
        (0xC3, b'i') => 'î',
        (0xC3, b'o') => 'ô',
        (0xC3, b'u') => 'û',
        (0xC4, b'A') => 'Ã',
        (0xC4, b'N') => 'Ñ',
        (0xC4, b'O') => 'Õ',
        (0xC4, b'a') => 'ã',
        (0xC4, b'n') => 'ñ',
        (0xC4, b'o') => 'õ',
        (0xC4, b'I') => 'Ĩ',
        (0xC4, b'i') => 'ĩ',
        (0xC4, b'U') => 'Ũ',
        (0xC4, b'u') => 'ũ',
        // macron
        (0xC5, b'A') => 'Ā',
        (0xC5, b'a') => 'ā',
        (0xC5, b'E') => 'Ē',
        (0xC5, b'e') => 'ē',
        (0xC5, b'I') => 'Ī',
        (0xC5, b'i') => 'ī',
        (0xC5, b'O') => 'Ō',
        (0xC5, b'o') => 'ō',
        (0xC5, b'U') => 'Ū',
        (0xC5, b'u') => 'ū',
        // breve
        (0xC6, b'A') => 'Ă',
        (0xC6, b'a') => 'ă',
        (0xC6, b'G') => 'Ğ',
        (0xC6, b'g') => 'ğ',
        (0xC6, b'U') => 'Ŭ',
        (0xC6, b'u') => 'ŭ',
        // dot above
        (0xC7, b'C') => 'Ċ',
        (0xC7, b'c') => 'ċ',
        (0xC7, b'E') => 'Ė',
        (0xC7, b'e') => 'ė',
        (0xC7, b'G') => 'Ġ',
        (0xC7, b'g') => 'ġ',
        (0xC7, b'I') => 'İ',
        (0xC7, b'Z') => 'Ż',
        (0xC7, b'z') => 'ż',
        (0xC8, b'A') => 'Ä',
        (0xC8, b'E') => 'Ë',
        (0xC8, b'I') => 'Ï',
        (0xC8, b'O') => 'Ö',
        (0xC8, b'U') => 'Ü',
        (0xC8, b'Y') => 'Ÿ',
        (0xC8, b'a') => 'ä',
        (0xC8, b'e') => 'ë',
        (0xC8, b'i') => 'ï',
        (0xC8, b'o') => 'ö',
        (0xC8, b'u') => 'ü',
        (0xC8, b'y') => 'ÿ',
        // ring above
        (0xCA, b'A') => 'Å',
        (0xCA, b'a') => 'å',
        (0xCA, b'U') => 'Ů',
        (0xCA, b'u') => 'ů',
        (0xCB, b'C') => 'Ç',
        (0xCB, b'c') => 'ç',
        (0xCB, b'G') => 'Ģ',
        (0xCB, b'g') => 'ģ',
        (0xCB, b'K') => 'Ķ',
        (0xCB, b'k') => 'ķ',
        (0xCB, b'L') => 'Ļ',
        (0xCB, b'l') => 'ļ',
        (0xCB, b'N') => 'Ņ',
        (0xCB, b'n') => 'ņ',
        (0xCB, b'R') => 'Ŗ',
        (0xCB, b'r') => 'ŗ',
        (0xCB, b'S') => 'Ş',
        (0xCB, b's') => 'ş',
        (0xCB, b'T') => 'Ţ',
        (0xCB, b't') => 'ţ',
        // double acute
        (0xCD, b'O') => 'Ő',
        (0xCD, b'o') => 'ő',
        (0xCD, b'U') => 'Ű',
        (0xCD, b'u') => 'ű',
        // ogonek
        (0xCE, b'A') => 'Ą',
        (0xCE, b'a') => 'ą',
        (0xCE, b'E') => 'Ę',
        (0xCE, b'e') => 'ę',
        (0xCE, b'I') => 'Į',
        (0xCE, b'i') => 'į',
        (0xCE, b'U') => 'Ų',
        (0xCE, b'u') => 'ų',
        // caron
        (0xCF, b'C') => 'Č',
        (0xCF, b'c') => 'č',
        (0xCF, b'D') => 'Ď',
        (0xCF, b'd') => 'ď',
        (0xCF, b'E') => 'Ě',
        (0xCF, b'e') => 'ě',
        (0xCF, b'L') => 'Ľ',
        (0xCF, b'l') => 'ľ',
        (0xCF, b'N') => 'Ň',
        (0xCF, b'n') => 'ň',
        (0xCF, b'R') => 'Ř',
        (0xCF, b'r') => 'ř',
        (0xCF, b'S') => 'Š',
        (0xCF, b's') => 'š',
        (0xCF, b'T') => 'Ť',
        (0xCF, b't') => 'ť',
        (0xCF, b'Z') => 'Ž',
        (0xCF, b'z') => 'ž',
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

fn decode_with(encoding: &'static encoding_rs::Encoding, bytes: &[u8]) -> String {
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
    fn decode_selector_0x12_ksx1001_euc_kr() {
        // EUC-KR 0xB0A1 = '가' (HANGUL SYLLABLE GA).
        assert_eq!(decode_dvb_string(&[0x12, 0xB0, 0xA1]), "가");
    }

    #[test]
    fn decode_selector_0x13_gb2312() {
        // GB-2312/GBK 0xC4E3 = '你'.
        assert_eq!(decode_dvb_string(&[0x13, 0xC4, 0xE3]), "你");
    }

    #[test]
    fn decode_selector_0x14_big5() {
        // Big5 0xA4A4 = '中'.
        assert_eq!(decode_dvb_string(&[0x14, 0xA4, 0xA4]), "中");
    }

    /// A multi-byte trail byte in 0x80–0x9F must survive: the C1 control
    /// filter operates on decoded code points, never on raw trail bytes.
    /// GBK 0x8180 = '亐' (U+4E90, trail byte in the C1 range).
    #[test]
    fn decode_selector_0x13_gbk_trail_byte_in_c1_range() {
        assert_eq!(decode_dvb_string(&[0x13, 0x81, 0x80]), "亐");
    }

    /// Annex A.1 two-byte control codes live at U+E080–U+E09F in the PUA
    /// (Table A.2): U+E08A is CR/LF → space; the reserved rest is stripped.
    /// GBK 0xABCD decodes to U+E08A; GBK 0xABC3 decodes to U+E080.
    #[test]
    fn two_byte_control_codes_filtered() {
        assert_eq!(decode_dvb_string(&[0x13, 0xAB, 0xCD]), " ");
        assert_eq!(decode_dvb_string(&[0x13, 0xAB, 0xC3]), "");
    }

    /// 0x1F consumes its 8-bit encoding_type_id; the body is undecodable
    /// (no registered broadcast ids) and yields U+FFFD per byte.
    #[test]
    fn decode_selector_0x1f_encoding_type_id() {
        let s = decode_dvb_string(&[0x1F, 0x01, 0x41, 0x42]);
        assert_eq!(s.chars().count(), 2);
        assert!(s.chars().all(|c| c == '\u{FFFD}'));
    }

    /// Table A.3 marks single-byte selector 0x08 reserved (no ISO 8859-12).
    #[test]
    fn reserved_selector_0x08_is_unsupported() {
        let s = decode_dvb_string(&[0x08, 0x41, 0x42]);
        assert!(s.chars().all(|c| c == '\u{FFFD}'));
        assert_eq!(s.chars().count(), 2);
    }

    #[test]
    fn unknown_selector_returns_replacement_characters() {
        // Selector 0x16 is reserved for future use — each byte becomes U+FFFD.
        let s = decode_dvb_string(&[0x16, 0xAA, 0xBB, 0xCC]);
        assert_eq!(s.chars().count(), 3);
        assert!(s.chars().all(|c| c == '\u{FFFD}'));
    }

    /// Pins the GR-area single-byte mappings to ETSI EN 300 468 V1.19.1
    /// (2025-02) Figure A.1 — "Character code table 00 - Latin alphabet with
    /// Unicode equivalents" (PDF p. 159; vendored at
    /// `specs/etsi_en_300_468_v01.19.01_dvb_si.pdf`).
    #[test]
    fn figure_a1_gr_area_single_byte_mappings() {
        let pins: &[(u8, char)] = &[
            (0xA0, '\u{00A0}'), // NBSP
            (0xA1, '¡'),
            (0xA2, '¢'),
            (0xA3, '£'),
            (0xA4, '\u{20AC}'), // € — DVB addition (note under Figure A.1)
            (0xA5, '¥'),
            (0xA7, '§'),
            (0xA8, '\u{00A4}'), // ¤ general currency sign
            (0xA9, '\u{2018}'), // '
            (0xAA, '\u{201C}'), // "
            (0xAB, '«'),
            (0xAC, '\u{2190}'), // ←
            (0xAD, '\u{2191}'), // ↑
            (0xAE, '\u{2192}'), // →
            (0xAF, '\u{2193}'), // ↓
            (0xB0, '°'),
            (0xB1, '±'),
            (0xB2, '²'),
            (0xB3, '³'),
            (0xB4, '\u{00D7}'), // ×
            (0xB5, 'µ'),
            (0xB6, '¶'),
            (0xB7, '·'),
            (0xB8, '\u{00F7}'), // ÷
            (0xB9, '\u{2019}'), // '
            (0xBA, '\u{201D}'), // "
            (0xBB, '»'),
            (0xBC, '¼'),
            (0xBD, '½'),
            (0xBE, '¾'),
            (0xBF, '¿'),
            (0xD0, '\u{2015}'), // ―
            (0xD1, '¹'),
            (0xD2, '®'),
            (0xD3, '©'),
            (0xD4, '\u{2122}'), // ™
            (0xD5, '\u{266A}'), // ♪
            (0xD6, '¬'),
            (0xD7, '\u{00A6}'), // ¦
            (0xDC, '\u{215B}'), // ⅛
            (0xDD, '\u{215C}'), // ⅜
            (0xDE, '\u{215D}'), // ⅝
            (0xDF, '\u{215E}'), // ⅞
            (0xE0, '\u{2126}'), // Ω OHM SIGN
            (0xE1, 'Æ'),
            (0xE2, '\u{0110}'), // Đ
            (0xE3, 'ª'),
            (0xE4, '\u{0126}'), // Ħ
            (0xE6, '\u{0132}'), // Ĳ
            (0xE7, '\u{013F}'), // Ŀ
            (0xE8, '\u{0141}'), // Ł
            (0xE9, 'Ø'),
            (0xEA, '\u{0152}'), // Œ
            (0xEB, 'º'),
            (0xEC, 'Þ'),
            (0xED, '\u{0166}'), // Ŧ
            (0xEE, '\u{014A}'), // Ŋ
            (0xEF, '\u{0149}'), // ŉ
            (0xF0, '\u{0138}'), // ĸ
            (0xF1, 'æ'),
            (0xF2, '\u{0111}'), // đ
            (0xF3, 'ð'),
            (0xF4, '\u{0127}'), // ħ
            (0xF5, '\u{0131}'), // ı
            (0xF6, '\u{0133}'), // ĳ
            (0xF7, '\u{0140}'), // ŀ
            (0xF8, '\u{0142}'), // ł
            (0xF9, 'ø'),
            (0xFA, '\u{0153}'), // œ
            (0xFB, 'ß'),
            (0xFC, '\u{00FE}'), // þ
            (0xFD, '\u{0167}'), // ŧ
            (0xFE, '\u{014B}'), // ŋ
            (0xFF, '\u{00AD}'), // SHY soft hyphen
        ];
        for &(byte, want) in pins {
            let got = decode_dvb_string(&[0x00, byte]);
            assert_eq!(
                got,
                want.to_string(),
                "byte {byte:#04x}: want {want:?} (U+{:04X}), got {got:?}",
                want as u32
            );
        }
    }

    /// Bytes undefined (grey) in Figure A.1 decode to U+FFFD.
    #[test]
    fn figure_a1_undefined_positions_are_replacement() {
        for byte in [0xA6u8, 0xD8, 0xD9, 0xDA, 0xDB, 0xE5] {
            let got = decode_dvb_string(&[0x00, byte]);
            assert_eq!(got, "\u{FFFD}", "byte {byte:#04x} should be U+FFFD");
        }
    }

    /// C-row prefixes with precomposed entries (Figure A.1 non-spacing row).
    #[test]
    fn figure_a1_combining_precomposed() {
        assert_eq!(decode_dvb_string(&[0x00, 0xCA, b'a']), "å"); // ring U+030A
        assert_eq!(decode_dvb_string(&[0x00, 0xCA, b'A']), "Å");
        assert_eq!(decode_dvb_string(&[0x00, 0xCF, b's']), "š"); // caron U+030C
        assert_eq!(decode_dvb_string(&[0x00, 0xCF, b'Z']), "Ž");
        assert_eq!(decode_dvb_string(&[0x00, 0xCE, b'e']), "ę"); // ogonek U+0328
        assert_eq!(decode_dvb_string(&[0x00, 0xCD, b'o']), "ő"); // double acute U+030B
        assert_eq!(decode_dvb_string(&[0x00, 0xC7, b'z']), "ż"); // dot above U+0307
        assert_eq!(decode_dvb_string(&[0x00, 0xC5, b'a']), "ā"); // macron U+0304
        assert_eq!(decode_dvb_string(&[0x00, 0xC6, b'g']), "ğ"); // breve U+0306
    }

    /// A defined prefix with no precomposed form falls back to
    /// base + Unicode combining mark (canonically equivalent).
    #[test]
    fn figure_a1_combining_fallback_emits_base_plus_mark() {
        assert_eq!(decode_dvb_string(&[0x00, 0xC5, b'x']), "x\u{0304}");
    }

    /// Undefined C-row prefixes (0xC0, 0xC9, 0xCC) and a dangling prefix at
    /// end of input decode to U+FFFD.
    #[test]
    fn figure_a1_combining_undefined_or_dangling_prefix() {
        assert_eq!(decode_dvb_string(&[0x00, 0xC0, b'a']), "\u{FFFD}a");
        assert_eq!(decode_dvb_string(&[0x00, 0xC9, b'a']), "\u{FFFD}a");
        assert_eq!(decode_dvb_string(&[0x00, 0xCC, b'a']), "\u{FFFD}a");
        assert_eq!(decode_dvb_string(&[0x00, 0xC2]), "\u{FFFD}");
    }

    #[test]
    fn dvb_text_decodes_with_charset_selector() {
        let t = DvbText::new(&[0x15, 0xC3, 0xA9]); // UTF-8 selector + é
        assert_eq!(t.decode(), "é");
        assert_eq!(t.raw(), &[0x15, 0xC3, 0xA9]);
        assert_eq!(&t[..], &[0x15, 0xC3, 0xA9]); // Deref
        assert_eq!(format!("{t}"), "é");
    }

    #[test]
    fn lang_code_as_str() {
        assert_eq!(LangCode(*b"fre").as_str(), "fre");
        assert_eq!(LangCode([0xFF, b'r', b'e']).as_str(), "\u{FFFD}re"); // lossy, no panic
    }

    #[cfg(feature = "serde")]
    #[test]
    fn dvb_text_serializes_decoded() {
        let t = DvbText::new(&[0x15, 0xC3, 0xA9]);
        assert_eq!(serde_json::to_string(&t).unwrap(), "\"é\"");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn lang_code_serializes_as_string() {
        // Serialize-only: LangCode renders as its decoded string. Parsing FROM
        // JSON is deliberately unsupported (re-parse from wire bytes instead).
        let lc = LangCode(*b"FRA");
        assert_eq!(serde_json::to_string(&lc).unwrap(), "\"FRA\"");
    }
}
