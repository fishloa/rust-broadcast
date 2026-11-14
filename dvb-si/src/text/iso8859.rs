//! ISO/IEC 8859 single-byte character sets for EN 300 468 Annex A Table A.3
//! (selectors 0x01–0x0B and the 0x10 0x00 nn form).
//!
//! Every part shares the same lower half: 0x00–0x7F is ASCII and 0x80–0x9F is
//! the C1 control set, so both decode to the code point of the same value and
//! the Annex A.1 control-code pass sees them. Only the GR area 0xA0–0xFF
//! differs between parts; it is tabulated here, one table per part, so these
//! charsets decode without any codec dependency (and under `no_std`). The
//! tables are pinned against `encoding_rs`'s WHATWG codecs byte for byte by
//! `tests::gr_tables_match_encoding_rs`.

use alloc::string::String;

/// First byte of the GR (right-hand graphic) area.
const GR_START: u8 = 0xA0;
/// Number of positions in the GR area `0xA0..=0xFF`.
const GR_LEN: usize = 96;

/// Decode `bytes` as ISO/IEC 8859-`part`; `None` when `part` is not a part
/// this module tabulates (8859-1 is decoded by the caller; 12 does not exist).
pub(super) fn decode(part: u8, bytes: &[u8]) -> Option<String> {
    let gr = gr_table(part)?;
    let glyph = |b: u8| match b.checked_sub(GR_START) {
        Some(i) => gr[usize::from(i)],
        None => char::from(b),
    };
    let len = bytes.iter().map(|&b| glyph(b).len_utf8()).sum();
    let mut out = String::with_capacity(len);
    out.extend(bytes.iter().map(|&b| glyph(b)));
    Some(out)
}

fn gr_table(part: u8) -> Option<&'static [char; GR_LEN]> {
    Some(match part {
        2 => &ISO_8859_2_GR,
        3 => &ISO_8859_3_GR,
        4 => &ISO_8859_4_GR,
        5 => &ISO_8859_5_GR,
        6 => &ISO_8859_6_GR,
        7 => &ISO_8859_7_GR,
        8 => &ISO_8859_8_GR,
        9 => &ISO_8859_9_GR,
        10 => &ISO_8859_10_GR,
        11 => &ISO_8859_11_GR,
        13 => &ISO_8859_13_GR,
        14 => &ISO_8859_14_GR,
        15 => &ISO_8859_15_GR,
        _ => return None,
    })
}

/// ISO/IEC 8859-2 positions `0xA0..=0xFF`; undefined positions are U+FFFD.
const ISO_8859_2_GR: [char; GR_LEN] = [
    '\u{00A0}', '\u{0104}', '\u{02D8}', '\u{0141}', '\u{00A4}', '\u{013D}', '\u{015A}', '\u{00A7}',
    '\u{00A8}', '\u{0160}', '\u{015E}', '\u{0164}', '\u{0179}', '\u{00AD}', '\u{017D}', '\u{017B}',
    '\u{00B0}', '\u{0105}', '\u{02DB}', '\u{0142}', '\u{00B4}', '\u{013E}', '\u{015B}', '\u{02C7}',
    '\u{00B8}', '\u{0161}', '\u{015F}', '\u{0165}', '\u{017A}', '\u{02DD}', '\u{017E}', '\u{017C}',
    '\u{0154}', '\u{00C1}', '\u{00C2}', '\u{0102}', '\u{00C4}', '\u{0139}', '\u{0106}', '\u{00C7}',
    '\u{010C}', '\u{00C9}', '\u{0118}', '\u{00CB}', '\u{011A}', '\u{00CD}', '\u{00CE}', '\u{010E}',
    '\u{0110}', '\u{0143}', '\u{0147}', '\u{00D3}', '\u{00D4}', '\u{0150}', '\u{00D6}', '\u{00D7}',
    '\u{0158}', '\u{016E}', '\u{00DA}', '\u{0170}', '\u{00DC}', '\u{00DD}', '\u{0162}', '\u{00DF}',
    '\u{0155}', '\u{00E1}', '\u{00E2}', '\u{0103}', '\u{00E4}', '\u{013A}', '\u{0107}', '\u{00E7}',
    '\u{010D}', '\u{00E9}', '\u{0119}', '\u{00EB}', '\u{011B}', '\u{00ED}', '\u{00EE}', '\u{010F}',
    '\u{0111}', '\u{0144}', '\u{0148}', '\u{00F3}', '\u{00F4}', '\u{0151}', '\u{00F6}', '\u{00F7}',
    '\u{0159}', '\u{016F}', '\u{00FA}', '\u{0171}', '\u{00FC}', '\u{00FD}', '\u{0163}', '\u{02D9}',
];

/// ISO/IEC 8859-3 positions `0xA0..=0xFF`; undefined positions are U+FFFD.
const ISO_8859_3_GR: [char; GR_LEN] = [
    '\u{00A0}', '\u{0126}', '\u{02D8}', '\u{00A3}', '\u{00A4}', '\u{FFFD}', '\u{0124}', '\u{00A7}',
    '\u{00A8}', '\u{0130}', '\u{015E}', '\u{011E}', '\u{0134}', '\u{00AD}', '\u{FFFD}', '\u{017B}',
    '\u{00B0}', '\u{0127}', '\u{00B2}', '\u{00B3}', '\u{00B4}', '\u{00B5}', '\u{0125}', '\u{00B7}',
    '\u{00B8}', '\u{0131}', '\u{015F}', '\u{011F}', '\u{0135}', '\u{00BD}', '\u{FFFD}', '\u{017C}',
    '\u{00C0}', '\u{00C1}', '\u{00C2}', '\u{FFFD}', '\u{00C4}', '\u{010A}', '\u{0108}', '\u{00C7}',
    '\u{00C8}', '\u{00C9}', '\u{00CA}', '\u{00CB}', '\u{00CC}', '\u{00CD}', '\u{00CE}', '\u{00CF}',
    '\u{FFFD}', '\u{00D1}', '\u{00D2}', '\u{00D3}', '\u{00D4}', '\u{0120}', '\u{00D6}', '\u{00D7}',
    '\u{011C}', '\u{00D9}', '\u{00DA}', '\u{00DB}', '\u{00DC}', '\u{016C}', '\u{015C}', '\u{00DF}',
    '\u{00E0}', '\u{00E1}', '\u{00E2}', '\u{FFFD}', '\u{00E4}', '\u{010B}', '\u{0109}', '\u{00E7}',
    '\u{00E8}', '\u{00E9}', '\u{00EA}', '\u{00EB}', '\u{00EC}', '\u{00ED}', '\u{00EE}', '\u{00EF}',
    '\u{FFFD}', '\u{00F1}', '\u{00F2}', '\u{00F3}', '\u{00F4}', '\u{0121}', '\u{00F6}', '\u{00F7}',
    '\u{011D}', '\u{00F9}', '\u{00FA}', '\u{00FB}', '\u{00FC}', '\u{016D}', '\u{015D}', '\u{02D9}',
];

/// ISO/IEC 8859-4 positions `0xA0..=0xFF`; undefined positions are U+FFFD.
const ISO_8859_4_GR: [char; GR_LEN] = [
    '\u{00A0}', '\u{0104}', '\u{0138}', '\u{0156}', '\u{00A4}', '\u{0128}', '\u{013B}', '\u{00A7}',
    '\u{00A8}', '\u{0160}', '\u{0112}', '\u{0122}', '\u{0166}', '\u{00AD}', '\u{017D}', '\u{00AF}',
    '\u{00B0}', '\u{0105}', '\u{02DB}', '\u{0157}', '\u{00B4}', '\u{0129}', '\u{013C}', '\u{02C7}',
    '\u{00B8}', '\u{0161}', '\u{0113}', '\u{0123}', '\u{0167}', '\u{014A}', '\u{017E}', '\u{014B}',
    '\u{0100}', '\u{00C1}', '\u{00C2}', '\u{00C3}', '\u{00C4}', '\u{00C5}', '\u{00C6}', '\u{012E}',
    '\u{010C}', '\u{00C9}', '\u{0118}', '\u{00CB}', '\u{0116}', '\u{00CD}', '\u{00CE}', '\u{012A}',
    '\u{0110}', '\u{0145}', '\u{014C}', '\u{0136}', '\u{00D4}', '\u{00D5}', '\u{00D6}', '\u{00D7}',
    '\u{00D8}', '\u{0172}', '\u{00DA}', '\u{00DB}', '\u{00DC}', '\u{0168}', '\u{016A}', '\u{00DF}',
    '\u{0101}', '\u{00E1}', '\u{00E2}', '\u{00E3}', '\u{00E4}', '\u{00E5}', '\u{00E6}', '\u{012F}',
    '\u{010D}', '\u{00E9}', '\u{0119}', '\u{00EB}', '\u{0117}', '\u{00ED}', '\u{00EE}', '\u{012B}',
    '\u{0111}', '\u{0146}', '\u{014D}', '\u{0137}', '\u{00F4}', '\u{00F5}', '\u{00F6}', '\u{00F7}',
    '\u{00F8}', '\u{0173}', '\u{00FA}', '\u{00FB}', '\u{00FC}', '\u{0169}', '\u{016B}', '\u{02D9}',
];

/// ISO/IEC 8859-5 positions `0xA0..=0xFF`; undefined positions are U+FFFD.
const ISO_8859_5_GR: [char; GR_LEN] = [
    '\u{00A0}', '\u{0401}', '\u{0402}', '\u{0403}', '\u{0404}', '\u{0405}', '\u{0406}', '\u{0407}',
    '\u{0408}', '\u{0409}', '\u{040A}', '\u{040B}', '\u{040C}', '\u{00AD}', '\u{040E}', '\u{040F}',
    '\u{0410}', '\u{0411}', '\u{0412}', '\u{0413}', '\u{0414}', '\u{0415}', '\u{0416}', '\u{0417}',
    '\u{0418}', '\u{0419}', '\u{041A}', '\u{041B}', '\u{041C}', '\u{041D}', '\u{041E}', '\u{041F}',
    '\u{0420}', '\u{0421}', '\u{0422}', '\u{0423}', '\u{0424}', '\u{0425}', '\u{0426}', '\u{0427}',
    '\u{0428}', '\u{0429}', '\u{042A}', '\u{042B}', '\u{042C}', '\u{042D}', '\u{042E}', '\u{042F}',
    '\u{0430}', '\u{0431}', '\u{0432}', '\u{0433}', '\u{0434}', '\u{0435}', '\u{0436}', '\u{0437}',
    '\u{0438}', '\u{0439}', '\u{043A}', '\u{043B}', '\u{043C}', '\u{043D}', '\u{043E}', '\u{043F}',
    '\u{0440}', '\u{0441}', '\u{0442}', '\u{0443}', '\u{0444}', '\u{0445}', '\u{0446}', '\u{0447}',
    '\u{0448}', '\u{0449}', '\u{044A}', '\u{044B}', '\u{044C}', '\u{044D}', '\u{044E}', '\u{044F}',
    '\u{2116}', '\u{0451}', '\u{0452}', '\u{0453}', '\u{0454}', '\u{0455}', '\u{0456}', '\u{0457}',
    '\u{0458}', '\u{0459}', '\u{045A}', '\u{045B}', '\u{045C}', '\u{00A7}', '\u{045E}', '\u{045F}',
];

/// ISO/IEC 8859-6 positions `0xA0..=0xFF`; undefined positions are U+FFFD.
const ISO_8859_6_GR: [char; GR_LEN] = [
    '\u{00A0}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{00A4}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}',
    '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{060C}', '\u{00AD}', '\u{FFFD}', '\u{FFFD}',
    '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}',
    '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{061B}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{061F}',
    '\u{FFFD}', '\u{0621}', '\u{0622}', '\u{0623}', '\u{0624}', '\u{0625}', '\u{0626}', '\u{0627}',
    '\u{0628}', '\u{0629}', '\u{062A}', '\u{062B}', '\u{062C}', '\u{062D}', '\u{062E}', '\u{062F}',
    '\u{0630}', '\u{0631}', '\u{0632}', '\u{0633}', '\u{0634}', '\u{0635}', '\u{0636}', '\u{0637}',
    '\u{0638}', '\u{0639}', '\u{063A}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}',
    '\u{0640}', '\u{0641}', '\u{0642}', '\u{0643}', '\u{0644}', '\u{0645}', '\u{0646}', '\u{0647}',
    '\u{0648}', '\u{0649}', '\u{064A}', '\u{064B}', '\u{064C}', '\u{064D}', '\u{064E}', '\u{064F}',
    '\u{0650}', '\u{0651}', '\u{0652}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}',
    '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}',
];

/// ISO/IEC 8859-7 positions `0xA0..=0xFF`; undefined positions are U+FFFD.
const ISO_8859_7_GR: [char; GR_LEN] = [
    '\u{00A0}', '\u{2018}', '\u{2019}', '\u{00A3}', '\u{20AC}', '\u{20AF}', '\u{00A6}', '\u{00A7}',
    '\u{00A8}', '\u{00A9}', '\u{037A}', '\u{00AB}', '\u{00AC}', '\u{00AD}', '\u{FFFD}', '\u{2015}',
    '\u{00B0}', '\u{00B1}', '\u{00B2}', '\u{00B3}', '\u{0384}', '\u{0385}', '\u{0386}', '\u{00B7}',
    '\u{0388}', '\u{0389}', '\u{038A}', '\u{00BB}', '\u{038C}', '\u{00BD}', '\u{038E}', '\u{038F}',
    '\u{0390}', '\u{0391}', '\u{0392}', '\u{0393}', '\u{0394}', '\u{0395}', '\u{0396}', '\u{0397}',
    '\u{0398}', '\u{0399}', '\u{039A}', '\u{039B}', '\u{039C}', '\u{039D}', '\u{039E}', '\u{039F}',
    '\u{03A0}', '\u{03A1}', '\u{FFFD}', '\u{03A3}', '\u{03A4}', '\u{03A5}', '\u{03A6}', '\u{03A7}',
    '\u{03A8}', '\u{03A9}', '\u{03AA}', '\u{03AB}', '\u{03AC}', '\u{03AD}', '\u{03AE}', '\u{03AF}',
    '\u{03B0}', '\u{03B1}', '\u{03B2}', '\u{03B3}', '\u{03B4}', '\u{03B5}', '\u{03B6}', '\u{03B7}',
    '\u{03B8}', '\u{03B9}', '\u{03BA}', '\u{03BB}', '\u{03BC}', '\u{03BD}', '\u{03BE}', '\u{03BF}',
    '\u{03C0}', '\u{03C1}', '\u{03C2}', '\u{03C3}', '\u{03C4}', '\u{03C5}', '\u{03C6}', '\u{03C7}',
    '\u{03C8}', '\u{03C9}', '\u{03CA}', '\u{03CB}', '\u{03CC}', '\u{03CD}', '\u{03CE}', '\u{FFFD}',
];

/// ISO/IEC 8859-8 positions `0xA0..=0xFF`; undefined positions are U+FFFD.
const ISO_8859_8_GR: [char; GR_LEN] = [
    '\u{00A0}', '\u{FFFD}', '\u{00A2}', '\u{00A3}', '\u{00A4}', '\u{00A5}', '\u{00A6}', '\u{00A7}',
    '\u{00A8}', '\u{00A9}', '\u{00D7}', '\u{00AB}', '\u{00AC}', '\u{00AD}', '\u{00AE}', '\u{00AF}',
    '\u{00B0}', '\u{00B1}', '\u{00B2}', '\u{00B3}', '\u{00B4}', '\u{00B5}', '\u{00B6}', '\u{00B7}',
    '\u{00B8}', '\u{00B9}', '\u{00F7}', '\u{00BB}', '\u{00BC}', '\u{00BD}', '\u{00BE}', '\u{FFFD}',
    '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}',
    '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}',
    '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}',
    '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{2017}',
    '\u{05D0}', '\u{05D1}', '\u{05D2}', '\u{05D3}', '\u{05D4}', '\u{05D5}', '\u{05D6}', '\u{05D7}',
    '\u{05D8}', '\u{05D9}', '\u{05DA}', '\u{05DB}', '\u{05DC}', '\u{05DD}', '\u{05DE}', '\u{05DF}',
    '\u{05E0}', '\u{05E1}', '\u{05E2}', '\u{05E3}', '\u{05E4}', '\u{05E5}', '\u{05E6}', '\u{05E7}',
    '\u{05E8}', '\u{05E9}', '\u{05EA}', '\u{FFFD}', '\u{FFFD}', '\u{200E}', '\u{200F}', '\u{FFFD}',
];

/// ISO/IEC 8859-9 positions `0xA0..=0xFF`; undefined positions are U+FFFD.
const ISO_8859_9_GR: [char; GR_LEN] = [
    '\u{00A0}', '\u{00A1}', '\u{00A2}', '\u{00A3}', '\u{00A4}', '\u{00A5}', '\u{00A6}', '\u{00A7}',
    '\u{00A8}', '\u{00A9}', '\u{00AA}', '\u{00AB}', '\u{00AC}', '\u{00AD}', '\u{00AE}', '\u{00AF}',
    '\u{00B0}', '\u{00B1}', '\u{00B2}', '\u{00B3}', '\u{00B4}', '\u{00B5}', '\u{00B6}', '\u{00B7}',
    '\u{00B8}', '\u{00B9}', '\u{00BA}', '\u{00BB}', '\u{00BC}', '\u{00BD}', '\u{00BE}', '\u{00BF}',
    '\u{00C0}', '\u{00C1}', '\u{00C2}', '\u{00C3}', '\u{00C4}', '\u{00C5}', '\u{00C6}', '\u{00C7}',
    '\u{00C8}', '\u{00C9}', '\u{00CA}', '\u{00CB}', '\u{00CC}', '\u{00CD}', '\u{00CE}', '\u{00CF}',
    '\u{011E}', '\u{00D1}', '\u{00D2}', '\u{00D3}', '\u{00D4}', '\u{00D5}', '\u{00D6}', '\u{00D7}',
    '\u{00D8}', '\u{00D9}', '\u{00DA}', '\u{00DB}', '\u{00DC}', '\u{0130}', '\u{015E}', '\u{00DF}',
    '\u{00E0}', '\u{00E1}', '\u{00E2}', '\u{00E3}', '\u{00E4}', '\u{00E5}', '\u{00E6}', '\u{00E7}',
    '\u{00E8}', '\u{00E9}', '\u{00EA}', '\u{00EB}', '\u{00EC}', '\u{00ED}', '\u{00EE}', '\u{00EF}',
    '\u{011F}', '\u{00F1}', '\u{00F2}', '\u{00F3}', '\u{00F4}', '\u{00F5}', '\u{00F6}', '\u{00F7}',
    '\u{00F8}', '\u{00F9}', '\u{00FA}', '\u{00FB}', '\u{00FC}', '\u{0131}', '\u{015F}', '\u{00FF}',
];

/// ISO/IEC 8859-10 positions `0xA0..=0xFF`; undefined positions are U+FFFD.
const ISO_8859_10_GR: [char; GR_LEN] = [
    '\u{00A0}', '\u{0104}', '\u{0112}', '\u{0122}', '\u{012A}', '\u{0128}', '\u{0136}', '\u{00A7}',
    '\u{013B}', '\u{0110}', '\u{0160}', '\u{0166}', '\u{017D}', '\u{00AD}', '\u{016A}', '\u{014A}',
    '\u{00B0}', '\u{0105}', '\u{0113}', '\u{0123}', '\u{012B}', '\u{0129}', '\u{0137}', '\u{00B7}',
    '\u{013C}', '\u{0111}', '\u{0161}', '\u{0167}', '\u{017E}', '\u{2015}', '\u{016B}', '\u{014B}',
    '\u{0100}', '\u{00C1}', '\u{00C2}', '\u{00C3}', '\u{00C4}', '\u{00C5}', '\u{00C6}', '\u{012E}',
    '\u{010C}', '\u{00C9}', '\u{0118}', '\u{00CB}', '\u{0116}', '\u{00CD}', '\u{00CE}', '\u{00CF}',
    '\u{00D0}', '\u{0145}', '\u{014C}', '\u{00D3}', '\u{00D4}', '\u{00D5}', '\u{00D6}', '\u{0168}',
    '\u{00D8}', '\u{0172}', '\u{00DA}', '\u{00DB}', '\u{00DC}', '\u{00DD}', '\u{00DE}', '\u{00DF}',
    '\u{0101}', '\u{00E1}', '\u{00E2}', '\u{00E3}', '\u{00E4}', '\u{00E5}', '\u{00E6}', '\u{012F}',
    '\u{010D}', '\u{00E9}', '\u{0119}', '\u{00EB}', '\u{0117}', '\u{00ED}', '\u{00EE}', '\u{00EF}',
    '\u{00F0}', '\u{0146}', '\u{014D}', '\u{00F3}', '\u{00F4}', '\u{00F5}', '\u{00F6}', '\u{0169}',
    '\u{00F8}', '\u{0173}', '\u{00FA}', '\u{00FB}', '\u{00FC}', '\u{00FD}', '\u{00FE}', '\u{0138}',
];

/// ISO/IEC 8859-11 positions `0xA0..=0xFF`; undefined positions are U+FFFD.
const ISO_8859_11_GR: [char; GR_LEN] = [
    '\u{00A0}', '\u{0E01}', '\u{0E02}', '\u{0E03}', '\u{0E04}', '\u{0E05}', '\u{0E06}', '\u{0E07}',
    '\u{0E08}', '\u{0E09}', '\u{0E0A}', '\u{0E0B}', '\u{0E0C}', '\u{0E0D}', '\u{0E0E}', '\u{0E0F}',
    '\u{0E10}', '\u{0E11}', '\u{0E12}', '\u{0E13}', '\u{0E14}', '\u{0E15}', '\u{0E16}', '\u{0E17}',
    '\u{0E18}', '\u{0E19}', '\u{0E1A}', '\u{0E1B}', '\u{0E1C}', '\u{0E1D}', '\u{0E1E}', '\u{0E1F}',
    '\u{0E20}', '\u{0E21}', '\u{0E22}', '\u{0E23}', '\u{0E24}', '\u{0E25}', '\u{0E26}', '\u{0E27}',
    '\u{0E28}', '\u{0E29}', '\u{0E2A}', '\u{0E2B}', '\u{0E2C}', '\u{0E2D}', '\u{0E2E}', '\u{0E2F}',
    '\u{0E30}', '\u{0E31}', '\u{0E32}', '\u{0E33}', '\u{0E34}', '\u{0E35}', '\u{0E36}', '\u{0E37}',
    '\u{0E38}', '\u{0E39}', '\u{0E3A}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{0E3F}',
    '\u{0E40}', '\u{0E41}', '\u{0E42}', '\u{0E43}', '\u{0E44}', '\u{0E45}', '\u{0E46}', '\u{0E47}',
    '\u{0E48}', '\u{0E49}', '\u{0E4A}', '\u{0E4B}', '\u{0E4C}', '\u{0E4D}', '\u{0E4E}', '\u{0E4F}',
    '\u{0E50}', '\u{0E51}', '\u{0E52}', '\u{0E53}', '\u{0E54}', '\u{0E55}', '\u{0E56}', '\u{0E57}',
    '\u{0E58}', '\u{0E59}', '\u{0E5A}', '\u{0E5B}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}',
];

/// ISO/IEC 8859-13 positions `0xA0..=0xFF`; undefined positions are U+FFFD.
const ISO_8859_13_GR: [char; GR_LEN] = [
    '\u{00A0}', '\u{201D}', '\u{00A2}', '\u{00A3}', '\u{00A4}', '\u{201E}', '\u{00A6}', '\u{00A7}',
    '\u{00D8}', '\u{00A9}', '\u{0156}', '\u{00AB}', '\u{00AC}', '\u{00AD}', '\u{00AE}', '\u{00C6}',
    '\u{00B0}', '\u{00B1}', '\u{00B2}', '\u{00B3}', '\u{201C}', '\u{00B5}', '\u{00B6}', '\u{00B7}',
    '\u{00F8}', '\u{00B9}', '\u{0157}', '\u{00BB}', '\u{00BC}', '\u{00BD}', '\u{00BE}', '\u{00E6}',
    '\u{0104}', '\u{012E}', '\u{0100}', '\u{0106}', '\u{00C4}', '\u{00C5}', '\u{0118}', '\u{0112}',
    '\u{010C}', '\u{00C9}', '\u{0179}', '\u{0116}', '\u{0122}', '\u{0136}', '\u{012A}', '\u{013B}',
    '\u{0160}', '\u{0143}', '\u{0145}', '\u{00D3}', '\u{014C}', '\u{00D5}', '\u{00D6}', '\u{00D7}',
    '\u{0172}', '\u{0141}', '\u{015A}', '\u{016A}', '\u{00DC}', '\u{017B}', '\u{017D}', '\u{00DF}',
    '\u{0105}', '\u{012F}', '\u{0101}', '\u{0107}', '\u{00E4}', '\u{00E5}', '\u{0119}', '\u{0113}',
    '\u{010D}', '\u{00E9}', '\u{017A}', '\u{0117}', '\u{0123}', '\u{0137}', '\u{012B}', '\u{013C}',
    '\u{0161}', '\u{0144}', '\u{0146}', '\u{00F3}', '\u{014D}', '\u{00F5}', '\u{00F6}', '\u{00F7}',
    '\u{0173}', '\u{0142}', '\u{015B}', '\u{016B}', '\u{00FC}', '\u{017C}', '\u{017E}', '\u{2019}',
];

/// ISO/IEC 8859-14 positions `0xA0..=0xFF`; undefined positions are U+FFFD.
const ISO_8859_14_GR: [char; GR_LEN] = [
    '\u{00A0}', '\u{1E02}', '\u{1E03}', '\u{00A3}', '\u{010A}', '\u{010B}', '\u{1E0A}', '\u{00A7}',
    '\u{1E80}', '\u{00A9}', '\u{1E82}', '\u{1E0B}', '\u{1EF2}', '\u{00AD}', '\u{00AE}', '\u{0178}',
    '\u{1E1E}', '\u{1E1F}', '\u{0120}', '\u{0121}', '\u{1E40}', '\u{1E41}', '\u{00B6}', '\u{1E56}',
    '\u{1E81}', '\u{1E57}', '\u{1E83}', '\u{1E60}', '\u{1EF3}', '\u{1E84}', '\u{1E85}', '\u{1E61}',
    '\u{00C0}', '\u{00C1}', '\u{00C2}', '\u{00C3}', '\u{00C4}', '\u{00C5}', '\u{00C6}', '\u{00C7}',
    '\u{00C8}', '\u{00C9}', '\u{00CA}', '\u{00CB}', '\u{00CC}', '\u{00CD}', '\u{00CE}', '\u{00CF}',
    '\u{0174}', '\u{00D1}', '\u{00D2}', '\u{00D3}', '\u{00D4}', '\u{00D5}', '\u{00D6}', '\u{1E6A}',
    '\u{00D8}', '\u{00D9}', '\u{00DA}', '\u{00DB}', '\u{00DC}', '\u{00DD}', '\u{0176}', '\u{00DF}',
    '\u{00E0}', '\u{00E1}', '\u{00E2}', '\u{00E3}', '\u{00E4}', '\u{00E5}', '\u{00E6}', '\u{00E7}',
    '\u{00E8}', '\u{00E9}', '\u{00EA}', '\u{00EB}', '\u{00EC}', '\u{00ED}', '\u{00EE}', '\u{00EF}',
    '\u{0175}', '\u{00F1}', '\u{00F2}', '\u{00F3}', '\u{00F4}', '\u{00F5}', '\u{00F6}', '\u{1E6B}',
    '\u{00F8}', '\u{00F9}', '\u{00FA}', '\u{00FB}', '\u{00FC}', '\u{00FD}', '\u{0177}', '\u{00FF}',
];

/// ISO/IEC 8859-15 positions `0xA0..=0xFF`; undefined positions are U+FFFD.
const ISO_8859_15_GR: [char; GR_LEN] = [
    '\u{00A0}', '\u{00A1}', '\u{00A2}', '\u{00A3}', '\u{20AC}', '\u{00A5}', '\u{0160}', '\u{00A7}',
    '\u{0161}', '\u{00A9}', '\u{00AA}', '\u{00AB}', '\u{00AC}', '\u{00AD}', '\u{00AE}', '\u{00AF}',
    '\u{00B0}', '\u{00B1}', '\u{00B2}', '\u{00B3}', '\u{017D}', '\u{00B5}', '\u{00B6}', '\u{00B7}',
    '\u{017E}', '\u{00B9}', '\u{00BA}', '\u{00BB}', '\u{0152}', '\u{0153}', '\u{0178}', '\u{00BF}',
    '\u{00C0}', '\u{00C1}', '\u{00C2}', '\u{00C3}', '\u{00C4}', '\u{00C5}', '\u{00C6}', '\u{00C7}',
    '\u{00C8}', '\u{00C9}', '\u{00CA}', '\u{00CB}', '\u{00CC}', '\u{00CD}', '\u{00CE}', '\u{00CF}',
    '\u{00D0}', '\u{00D1}', '\u{00D2}', '\u{00D3}', '\u{00D4}', '\u{00D5}', '\u{00D6}', '\u{00D7}',
    '\u{00D8}', '\u{00D9}', '\u{00DA}', '\u{00DB}', '\u{00DC}', '\u{00DD}', '\u{00DE}', '\u{00DF}',
    '\u{00E0}', '\u{00E1}', '\u{00E2}', '\u{00E3}', '\u{00E4}', '\u{00E5}', '\u{00E6}', '\u{00E7}',
    '\u{00E8}', '\u{00E9}', '\u{00EA}', '\u{00EB}', '\u{00EC}', '\u{00ED}', '\u{00EE}', '\u{00EF}',
    '\u{00F0}', '\u{00F1}', '\u{00F2}', '\u{00F3}', '\u{00F4}', '\u{00F5}', '\u{00F6}', '\u{00F7}',
    '\u{00F8}', '\u{00F9}', '\u{00FA}', '\u{00FB}', '\u{00FC}', '\u{00FD}', '\u{00FE}', '\u{00FF}',
];

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// The tabulated parts against `encoding_rs` (WHATWG) over all 256 bytes.
    /// WHATWG has no pure 8859-9 / 8859-11: its labels route to windows-1254 /
    /// windows-874, which agree on the GR area but put printable glyphs in
    /// 0x80–0x9F, so for those two only the GR area is compared.
    #[test]
    fn gr_tables_match_encoding_rs() {
        use encoding_rs::*;
        let parts: [(u8, &'static Encoding, bool); 13] = [
            (2, ISO_8859_2, true),
            (3, ISO_8859_3, true),
            (4, ISO_8859_4, true),
            (5, ISO_8859_5, true),
            (6, ISO_8859_6, true),
            (7, ISO_8859_7, true),
            (8, ISO_8859_8, true),
            (9, WINDOWS_1254, false),
            (10, ISO_8859_10, true),
            (11, WINDOWS_874, false),
            (13, ISO_8859_13, true),
            (14, ISO_8859_14, true),
            (15, ISO_8859_15, true),
        ];
        let all: Vec<u8> = (0..=u8::MAX).collect();
        for (part, encoding, whole) in parts {
            let ours: Vec<char> = decode(part, &all).unwrap().chars().collect();
            let (theirs, _) = encoding.decode_without_bom_handling(&all);
            let theirs: Vec<char> = theirs.chars().collect();
            assert_eq!(ours.len(), 256, "8859-{part}");
            assert_eq!(theirs.len(), 256, "8859-{part}");
            let from = if whole { 0 } else { usize::from(GR_START) };
            assert_eq!(ours[from..], theirs[from..], "8859-{part}");
            for b in 0..usize::from(GR_START) {
                assert_eq!(u32::from(ours[b]), b as u32, "8859-{part} byte {b:#04x}");
            }
        }
    }

    #[test]
    fn untabulated_parts_are_none() {
        for part in [0u8, 1, 12, 16, 17, 0xFF] {
            assert_eq!(decode(part, b"A"), None, "8859-{part}");
        }
    }

    #[test]
    fn known_glyphs() {
        assert_eq!(decode(2, &[0xA1]).unwrap(), "Ą");
        assert_eq!(decode(5, &[0xB0, 0xB1]).unwrap(), "АБ");
        assert_eq!(decode(9, &[0xDD]).unwrap(), "İ");
        assert_eq!(decode(15, &[0xA4]).unwrap(), "€");
        assert_eq!(decode(3, &[0xA5]).unwrap(), "\u{FFFD}");
    }
}
