//! EBU Teletext (ETSI EN 300 706 V1.2.1) subtitle page decode.
//!
//! Cite: `docs/teletext-subtitles.md` (curated transcription of the sections
//! used here: §7.1 packet structure, §8.1/§8.2 FEC, §9.3.1 page header,
//! Table 2 control bits, §12.2 spacing attributes, §15.1-15.2 national option
//! selection, Table 35/36 Latin G0 character set; the Table 36 reading is in
//! `private/specs-verified/dvb/timed-metadata/teletext-national-options.md`).
//!
//! `dvb-vbi` carries the EN 301 775 VBI PES framing (`TeletextDataField`, a
//! 42-byte opaque `txt_data_block`) but — by its own module docs — does not
//! decode EN 300 706 (a large, separate spec covering FEC, character sets and
//! page composition, not carriage). This module owns that decode, consuming
//! only `dvb_vbi::TeletextDataField`'s raw bytes:
//!
//! - [`decode_hamming_8_4`] / [`encode_hamming_8_4`] — the 4-data-bit +
//!   4-parity-bit code protecting packet addresses and page header fields
//!   (§8.2): single-bit errors corrected, double-bit errors rejected.
//! - [`decode_odd_parity`] / [`encode_odd_parity`] — the 7-data-bit + 1-parity
//!   code protecting displayable row bytes (§8.1): errors only *detected*
//!   (odd parity carries no correction capability), rendered as
//!   `'\u{FFFD}'`.
//! - [`NationalOption`] — the C12/C13/C14 national option sub-set selector
//!   (§15.1, Table 32's first ["Latin 0", Level-1-ambiguous] group, the
//!   interpretation clause 15.1 mandates at presentation Level 1). Every
//!   Table 36 row is a variant; the header can select the first seven.
//! - [`latin_g0_char`] — Table 35 (Latin G0 Primary Set, identical to 7-bit
//!   ASCII `0x20`-`0x7F` outside 13 reserved positions) + the Table 36
//!   substitutions at those positions for the page's national option.
//! - [`PacketAddress`] / [`decode_packet_address`] — the magazine + row
//!   number prefix common to every packet (§7.1.2).
//! - [`PageHeader`] / [`PageHeader::parse`] — the page header packet (`Y=0`,
//!   §9.3.1): page number, sub-code, and all eleven control bits (Table 2).
//! - `PageAssembler` (crate-private) — accumulates a single tracked
//!   `(magazine, page)`'s row packets (`Y=1..=24`) into display text,
//!   applying the erase-page (C4) and inhibit-display (C10) control bits;
//!   drives [`crate::webvtt::TeletextCueExtractor`].
//!
//! # Documented losses (first pass, matching this crate's `cc-data`
//! extractors' lossy-by-design philosophy)
//!
//! - **No enhancement packets**: `X/26` (character/attribute overwrite),
//!   `X/27`/`X/28`/`M/29` (page linking, character-set re-designation, side
//!   panels, CLUTs) are not processed. Only basic Level-1 page composition
//!   (`X/0` header + `X/1`-`X/24` display rows) is decoded.
//! - **No styling**: spacing-attribute control codes (`0x00`-`0x1F`, clause
//!   12.2 — colour, flash, double-height, etc.) are rendered as a space, not
//!   carried into the WebVTT payload as cue styling. Start Box / End Box are
//!   interpreted: a C5/C6 page displays only the boxed text.
//! - **Sub-code ignored for page matching**: a page is matched by magazine +
//!   page number only; multi-subpage rotation (e.g. multi-language subtitle
//!   variants sharing one page number) is not distinguished.
//! - **National options**: only the Level 1 reading of C12-C14 (Table 32's
//!   first group); X/28 and M/29 designations are not processed.
use alloc::string::String;
use alloc::vec::Vec;

/// Decode a Hamming-8/4 protected byte (ETSI EN 300 706 §8.2): bits 1,3,5,7
/// (transmission order, i.e. the LSB-first bit positions `0x01,0x04,0x10,0x40`)
/// are the protection bits P1-P4, bits 2,4,6,8 (`0x02,0x08,0x20,0x80`) carry
/// the 4 data bits D1-D4.
///
/// Implemented as a brute-force nearest-codeword search against
/// [`encode_hamming_8_4`] rather than a hand-derived syndrome table: the code
/// has minimum distance 4 (a (7,4) Hamming code extended with an overall
/// parity bit), so a 0-bit-error byte matches its own re-encoding exactly, a
/// single-bit error matches after flipping exactly one bit, and a
/// (rejected) double-bit error matches no single flip — this is
/// mathematically equivalent to the spec's "four odd parity tests A-D"
/// procedure and was cross-checked against it (see the crate tests below).
///
/// Returns the corrected 4-bit data nibble (`D1` in bit 0 .. `D4` in bit 3),
/// or `None` if the byte is not within Hamming distance 1 of any valid
/// codeword (an uncorrectable, "double error", byte per §8.2's decode table).
#[must_use]
pub fn decode_hamming_8_4(byte: u8) -> Option<u8> {
    let candidate =
        (byte >> 1) & 1 | ((byte >> 3) & 1) << 1 | ((byte >> 5) & 1) << 2 | ((byte >> 7) & 1) << 3;
    if encode_hamming_8_4(candidate) == byte {
        return Some(candidate);
    }
    for bit in 0..8u8 {
        let flipped = byte ^ (1 << bit);
        let candidate = (flipped >> 1) & 1
            | ((flipped >> 3) & 1) << 1
            | ((flipped >> 5) & 1) << 2
            | ((flipped >> 7) & 1) << 3;
        if encode_hamming_8_4(candidate) == flipped {
            return Some(candidate);
        }
    }
    None
}

/// Encode a 4-bit data nibble (`D1` in bit 0 .. `D4` in bit 3) as a
/// Hamming-8/4 protected byte, per the ETSI EN 300 706 §8.2 encoding
/// equations:
///
/// ```text
/// P1 = 1 ⊕ D1 ⊕ D3 ⊕ D4
/// P2 = 1 ⊕ D1 ⊕ D2 ⊕ D4
/// P3 = 1 ⊕ D1 ⊕ D2 ⊕ D3
/// P4 = 1 ⊕ P1 ⊕ D1 ⊕ P2 ⊕ D2 ⊕ P3 ⊕ D3 ⊕ D4
/// ```
///
/// with wire bit order (transmission order, LSB first) `P1 D1 P2 D2 P3 D3 P4 D4`.
/// Only the low 4 bits of `nibble` are used.
#[must_use]
pub fn encode_hamming_8_4(nibble: u8) -> u8 {
    let d1 = nibble & 1;
    let d2 = (nibble >> 1) & 1;
    let d3 = (nibble >> 2) & 1;
    let d4 = (nibble >> 3) & 1;
    let p1 = 1 ^ d1 ^ d3 ^ d4;
    let p2 = 1 ^ d1 ^ d2 ^ d4;
    let p3 = 1 ^ d1 ^ d2 ^ d3;
    let p4 = 1 ^ p1 ^ d1 ^ p2 ^ d2 ^ p3 ^ d3 ^ d4;
    p1 | (d1 << 1) | (p2 << 2) | (d2 << 3) | (p3 << 4) | (d3 << 5) | (p4 << 6) | (d4 << 7)
}

/// Decode an odd-parity protected byte (ETSI EN 300 706 §8.1): bit 8 (the
/// MSB, `0x80`) is the parity bit, bits 1-7 (`0x7F`) carry the 7 data bits.
/// Odd parity **detects but cannot correct** single-bit errors (unlike
/// Hamming-8/4): returns `None` on any parity mismatch.
///
/// Returns the 7-bit data value, or `None` if the byte does not have odd
/// parity (an even count of set bits).
#[must_use]
pub fn decode_odd_parity(byte: u8) -> Option<u8> {
    broadcast_common::parity::strip_parity(byte)
}

/// Encode a 7-bit data value with odd parity in bit 8 (MSB), per ETSI
/// EN 300 706 §8.1. Only the low 7 bits of `data7` are used.
#[must_use]
pub fn encode_odd_parity(data7: u8) -> u8 {
    broadcast_common::parity::set_odd_parity(data7)
}

/// The C12/C13/C14 "National Option Character Subset" selector (ETSI
/// EN 300 706 Table 2, page header control bits) — decoded per Table 32's
/// first group (`0000XXX`, "Latin 0"), which clause 15.1 states is the
/// interpretation at presentation Level 1 ("the national option sub-set in
/// use on the page is defined by the C12, C13 and C14 control bits in the
/// page header alone").
///
/// [`latin_g0_char`] applies the Table 36 substitutions of the option in
/// force; a [`NationalOption::Reserved`] value substitutes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum NationalOption {
    /// `000` — English.
    English,
    /// `001` — German.
    German,
    /// `010` — Swedish/Finnish/Hungarian.
    SwedishFinnishHungarian,
    /// `011` — Italian.
    Italian,
    /// `100` — French.
    French,
    /// `101` — Portuguese/Spanish.
    PortugueseSpanish,
    /// `110` — Czech/Slovak.
    CzechSlovak,
    /// Polish (Table 36). Not selectable by C12-C14 at Level 1 (Table 32
    /// places it in a later group), kept so every Table 36 row is addressable.
    Polish,
    /// Rumanian (Table 36); not selectable by C12-C14 at Level 1.
    Rumanian,
    /// Turkish (Table 36); not selectable by C12-C14 at Level 1.
    Turkish,
    /// Serbian/Croatian/Slovenian (Table 36); not selectable by C12-C14 at
    /// Level 1.
    SerbianCroatianSlovenian,
    /// Estonian (Table 36); not selectable by C12-C14 at Level 1.
    Estonian,
    /// Lettish/Lithuanian (Table 36); not selectable by C12-C14 at Level 1.
    LettishLithuanian,
    /// `111` — reserved (Table 32's first group defines no option here); the
    /// raw 3-bit value is retained.
    Reserved(u8),
}

impl NationalOption {
    /// Decode from the packed 3-bit value `(c12 << 2) | (c13 << 1) | c14`.
    #[must_use]
    pub fn from_bits(v: u8) -> Self {
        match v & 0x7 {
            0 => NationalOption::English,
            1 => NationalOption::German,
            2 => NationalOption::SwedishFinnishHungarian,
            3 => NationalOption::Italian,
            4 => NationalOption::French,
            5 => NationalOption::PortugueseSpanish,
            6 => NationalOption::CzechSlovak,
            other => NationalOption::Reserved(other),
        }
    }

    /// Spec token (issue #204 label convention).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            NationalOption::English => "English",
            NationalOption::German => "German",
            NationalOption::SwedishFinnishHungarian => "Swedish/Finnish/Hungarian",
            NationalOption::Italian => "Italian",
            NationalOption::French => "French",
            NationalOption::PortugueseSpanish => "Portuguese/Spanish",
            NationalOption::CzechSlovak => "Czech/Slovak",
            NationalOption::Polish => "Polish",
            NationalOption::Rumanian => "Rumanian",
            NationalOption::Turkish => "Turkish",
            NationalOption::SerbianCroatianSlovenian => "Serbian/Croatian/Slovenian",
            NationalOption::Estonian => "Estonian",
            NationalOption::LettishLithuanian => "Lettish/Lithuanian",
            NationalOption::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(NationalOption, Reserved);

/// Decode one Latin G0 (Table 35) code position (`0x20`-`0x7F`) to its
/// display character, applying the Table 36 national-option substitution of
/// `option` at the 13 reserved positions. The glyph chart is an image in the
/// spec; the reading is recorded in
/// `private/specs-verified/dvb/timed-metadata/teletext-national-options.md`.
///
/// Codes below `0x20` are Level-1 spacing attributes (clause 12.2); each
/// occupies a character cell displayed as a SPACE, so they decode to a space
/// here (the box attributes are interpreted by the row renderer, not here).
/// Code `0x7F` is the Level-1 "reserved position" full block (Table 35
/// note 4), rendered as `'\u{2588}'` (FULL BLOCK). A [`NationalOption::Reserved`]
/// option substitutes nothing and yields the base ASCII glyph.
#[must_use]
pub fn latin_g0_char(code: u8, option: NationalOption) -> char {
    let code = code & CODE_MASK_7BIT;
    if code < FIRST_GRAPHIC_CODE {
        return ' ';
    }
    if code == FULL_BLOCK_CODE {
        return '\u{2588}';
    }
    if let Some(subset) = national_subset(option)
        && let Some(slot) = NATIONAL_POSITIONS.iter().position(|&p| p == code)
    {
        return subset[slot];
    }
    code as char
}

const CODE_MASK_7BIT: u8 = 0x7F;
const FIRST_GRAPHIC_CODE: u8 = 0x20;
const FULL_BLOCK_CODE: u8 = 0x7F;

/// Spacing attribute 0/A, End Box (EN 300 706 clause 12.2, Table 26).
const END_BOX: u8 = 0x0A;
/// Spacing attribute 0/B, Start Box (EN 300 706 clause 12.2, Table 26).
const START_BOX: u8 = 0x0B;

/// The 13 Table 35 code positions a national option replaces, in the column
/// order of Table 36 (2/3 2/4 4/0 5/B 5/C 5/D 5/E 5/F 6/0 7/B 7/C 7/D 7/E).
const NATIONAL_POSITIONS: [u8; 13] = [
    0x23, 0x24, 0x40, 0x5B, 0x5C, 0x5D, 0x5E, 0x5F, 0x60, 0x7B, 0x7C, 0x7D, 0x7E,
];

type NationalSubset = [char; 13];

const CZECH_SLOVAK: NationalSubset = [
    '#', 'ů', 'č', 'ť', 'ž', 'ý', 'í', 'ř', 'é', 'á', 'ě', 'ú', 'š',
];
const ENGLISH: NationalSubset = [
    '£', '$', '@', '←', '½', '→', '↑', '#', '\u{2015}', '¼', '\u{2016}', '¾', '÷',
];
const ESTONIAN: NationalSubset = [
    '#', 'õ', 'Š', 'Ä', 'Ö', 'Ž', 'Ü', 'Õ', 'š', 'ä', 'ö', 'ž', 'ü',
];
const FRENCH: NationalSubset = [
    'é', 'ï', 'à', 'ë', 'ê', 'ù', 'î', '#', 'è', 'â', 'ô', 'û', 'ç',
];
const GERMAN: NationalSubset = [
    '#', '$', '§', 'Ä', 'Ö', 'Ü', '^', '_', '°', 'ä', 'ö', 'ü', 'ß',
];
const ITALIAN: NationalSubset = [
    '£', '$', 'é', '°', 'ç', '→', '↑', '#', 'ù', 'à', 'ò', 'è', 'ì',
];
const LETTISH_LITHUANIAN: NationalSubset = [
    '#', '$', 'Š', 'ė', 'ę', 'Ž', 'č', 'ū', 'š', 'ą', 'ų', 'ž', 'į',
];
const POLISH: NationalSubset = [
    '#', 'ń', 'ą', 'Ż', 'Ś', 'Ł', 'ć', 'ó', 'ę', 'ż', 'ś', 'ł', 'ź',
];
const PORTUGUESE_SPANISH: NationalSubset = [
    'ç', '$', '¡', 'á', 'é', 'í', 'ó', 'ú', '¿', 'ü', 'ñ', 'è', 'à',
];
const RUMANIAN: NationalSubset = [
    '#', '¤', 'Ţ', 'Â', 'Ş', 'Ă', 'Î', 'ı', 'ţ', 'â', 'ş', 'ă', 'î',
];
const SERBIAN_CROATIAN_SLOVENIAN: NationalSubset = [
    '#', 'Ë', 'Č', 'Ć', 'Ž', 'Đ', 'Š', 'ë', 'č', 'ć', 'ž', 'đ', 'š',
];
const SWEDISH_FINNISH_HUNGARIAN: NationalSubset = [
    '#', '¤', 'É', 'Ä', 'Ö', 'Å', 'Ü', '_', 'é', 'ä', 'ö', 'å', 'ü',
];
const TURKISH: NationalSubset = [
    '\u{20BA}', 'ğ', 'İ', 'Ş', 'Ö', 'Ç', 'Ü', 'Ğ', 'ı', 'ş', 'ö', 'ç', 'ü',
];

fn national_subset(option: NationalOption) -> Option<&'static NationalSubset> {
    Some(match option {
        NationalOption::English => &ENGLISH,
        NationalOption::German => &GERMAN,
        NationalOption::SwedishFinnishHungarian => &SWEDISH_FINNISH_HUNGARIAN,
        NationalOption::Italian => &ITALIAN,
        NationalOption::French => &FRENCH,
        NationalOption::PortugueseSpanish => &PORTUGUESE_SPANISH,
        NationalOption::CzechSlovak => &CZECH_SLOVAK,
        NationalOption::Polish => &POLISH,
        NationalOption::Rumanian => &RUMANIAN,
        NationalOption::Turkish => &TURKISH,
        NationalOption::SerbianCroatianSlovenian => &SERBIAN_CROATIAN_SLOVENIAN,
        NationalOption::Estonian => &ESTONIAN,
        NationalOption::LettishLithuanian => &LETTISH_LITHUANIAN,
        NationalOption::Reserved(_) => return None,
    })
}

/// The magazine + row number packet address prefix common to every Teletext
/// packet (ETSI EN 300 706 §7.1.2): 2 bytes, both Hamming-8/4 coded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketAddress {
    /// Magazine number, `1..=8` (a packet address magazine field of `0`
    /// denotes magazine 8 — §3, "magazine number 8").
    pub magazine: u8,
    /// Packet number `Y`, `0..=31` (`0` = page header, `1..=25` = display
    /// rows, `26..=31` = non-displayable enhancement packets).
    pub row: u8,
}

/// Decode the 2-byte packet address prefix (`txt_data_block[0..2]` of a
/// [`dvb_vbi::TeletextDataField`] — EN 300 706 bytes 4-5, since
/// `txt_data_block` starts after the clock-run-in/framing-code).
///
/// Returns `None` if either byte is an uncorrectable (double-bit-error)
/// Hamming-8/4 byte; the packet is then silently ignored by
/// `PageAssembler` (a robustness choice, not a spec requirement).
#[must_use]
pub fn decode_packet_address(b4: u8, b5: u8) -> Option<PacketAddress> {
    let n4 = decode_hamming_8_4(b4)?;
    let n5 = decode_hamming_8_4(b5)?;
    let mag_field = n4 & 0x7;
    let magazine = if mag_field == 0 { 8 } else { mag_field };
    let y0 = (n4 >> 3) & 1;
    let row = y0 | (n5 << 1);
    Some(PacketAddress { magazine, row })
}

/// A decoded page header packet (`Y = 0`, ETSI EN 300 706 §9.3.1): page
/// address, sub-code, and all eleven control bits (Table 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageHeader {
    /// Page number, `Pt << 4 | Pu` (§9.3.1.1; both nibbles `0x0`-`0xF`).
    pub page: u8,
    /// Page sub-code element S1 (§9.3.1.2, byte 8; `0x0`-`0xF`).
    pub s1: u8,
    /// Page sub-code element S2 (byte 9 bits 2/4/6; `0x0`-`0x7`).
    pub s2: u8,
    /// Page sub-code element S3 (byte 10; `0x0`-`0xF`).
    pub s3: u8,
    /// Page sub-code element S4 (byte 11 bits 2/4; `0x0`-`0x3`).
    pub s4: u8,
    /// C4 Erase Page.
    pub erase_page: bool,
    /// C5 Newsflash.
    pub newsflash: bool,
    /// C6 Subtitle.
    pub subtitle: bool,
    /// C7 Suppress Header (row 0 not displayed).
    pub suppress_header: bool,
    /// C8 Update Indicator.
    pub update_indicator: bool,
    /// C9 Interrupted Sequence.
    pub interrupted_sequence: bool,
    /// C10 Inhibit Display (rows 1-24 not displayed).
    pub inhibit_display: bool,
    /// C11 Magazine Serial (`true` = serial mode, `false` = parallel mode).
    pub magazine_serial: bool,
    /// C12/C13/C14 National Option Character Subset.
    pub national_option: NationalOption,
}

impl PageHeader {
    /// Decode a page header from the 8 Hamming-8/4 bytes at
    /// `txt_data_block[2..10]` (EN 300 706 bytes 6-13 — the 2 bytes of
    /// packet address precede these). Returns `None` if any of the 8 bytes
    /// is an uncorrectable Hamming-8/4 byte.
    #[must_use]
    pub fn parse(txt_data_block: &[u8; 42]) -> Option<PageHeader> {
        let page_units = decode_hamming_8_4(txt_data_block[2])?;
        let page_tens = decode_hamming_8_4(txt_data_block[3])?;
        let s1 = decode_hamming_8_4(txt_data_block[4])?;
        let n_s2_c4 = decode_hamming_8_4(txt_data_block[5])?;
        let s3 = decode_hamming_8_4(txt_data_block[6])?;
        let n_s4_c5_c6 = decode_hamming_8_4(txt_data_block[7])?;
        let n_c7_c10 = decode_hamming_8_4(txt_data_block[8])?;
        let n_c11_c14 = decode_hamming_8_4(txt_data_block[9])?;

        let s2 = n_s2_c4 & 0x7;
        let c4 = (n_s2_c4 >> 3) & 1;
        let s4 = n_s4_c5_c6 & 0x3;
        let c5 = (n_s4_c5_c6 >> 2) & 1;
        let c6 = (n_s4_c5_c6 >> 3) & 1;
        let c7 = n_c7_c10 & 1;
        let c8 = (n_c7_c10 >> 1) & 1;
        let c9 = (n_c7_c10 >> 2) & 1;
        let c10 = (n_c7_c10 >> 3) & 1;
        let c11 = n_c11_c14 & 1;
        let c12 = (n_c11_c14 >> 1) & 1;
        let c13 = (n_c11_c14 >> 2) & 1;
        let c14 = (n_c11_c14 >> 3) & 1;

        Some(PageHeader {
            page: (page_tens << 4) | page_units,
            s1,
            s2,
            s3,
            s4,
            erase_page: c4 != 0,
            newsflash: c5 != 0,
            subtitle: c6 != 0,
            suppress_header: c7 != 0,
            update_indicator: c8 != 0,
            interrupted_sequence: c9 != 0,
            inhibit_display: c10 != 0,
            magazine_serial: c11 != 0,
            national_option: NationalOption::from_bits((c12 << 2) | (c13 << 1) | c14),
        })
    }
}

const ROW_LEN: usize = 40;
const DISPLAY_ROWS: usize = 24;
const ROW_PAYLOAD_START: usize = 2;

/// One display row's 40 columns after odd-parity decode (§8.1): `Some` holds
/// the 7-bit code, `None` a byte that failed its parity check.
type RowCodes = [Option<u8>; ROW_LEN];

fn decode_row_codes(txt_data_block: &[u8; 42]) -> RowCodes {
    let mut codes = [None; ROW_LEN];
    for (slot, &b) in codes
        .iter_mut()
        .zip(&txt_data_block[ROW_PAYLOAD_START..ROW_PAYLOAD_START + ROW_LEN])
    {
        *slot = decode_odd_parity(b);
    }
    codes
}

fn push_code(out: &mut String, code: Option<u8>, option: NationalOption) {
    match code {
        Some(code) => out.push(latin_g0_char(code, option)),
        None => out.push('\u{FFFD}'),
    }
}

/// Render one row. On a boxed (C5 newsflash / C6 subtitle) page only the
/// columns after a Start Box and before the next End Box are displayed
/// (clause 12.2, Table 26, attributes 0/A and 0/B: "characters outside this
/// area are not displayed"; both reset at the start of a row); the two
/// attribute cells and the padding at the box edges are not text and are
/// trimmed. On any other page the whole row is rendered, trailing spaces
/// trimmed.
fn render_row(codes: &RowCodes, option: NationalOption, boxed_page: bool) -> String {
    let mut out = String::with_capacity(ROW_LEN);
    if boxed_page {
        let mut in_box = false;
        for &code in codes {
            match code {
                Some(START_BOX) => in_box = true,
                Some(END_BOX) => in_box = false,
                _ if in_box => push_code(&mut out, code, option),
                _ => {}
            }
        }
        out.trim().into()
    } else {
        for &code in codes {
            push_code(&mut out, code, option);
        }
        out.trim_end().into()
    }
}

/// Accumulates one tracked `(magazine, page)`'s row packets into display
/// text (ETSI EN 300 706 §7.2: a page's body is its header packet plus all
/// subsequent `Y=1..=24` packets in the same magazine, up to the next
/// header). Crate-private: the public surface is
/// [`crate::webvtt::TeletextCueExtractor`].
pub(crate) struct PageAssembler {
    magazine: u8,
    page: u8,
    /// Whether the magazine's most-recently-seen header packet matched
    /// `(magazine, page)` — row packets in a magazine belong to whichever
    /// page was last headed, per §7.2.1.
    active: bool,
    inhibited: bool,
    boxed_page: bool,
    national_option: NationalOption,
    /// Index `0` = row 1 .. index `23` = row 24.
    rows: [Option<RowCodes>; DISPLAY_ROWS],
}

impl PageAssembler {
    pub(crate) fn new(magazine: u8, page: u8) -> Self {
        PageAssembler {
            magazine,
            page,
            active: false,
            inhibited: false,
            boxed_page: false,
            national_option: NationalOption::English,
            rows: [None; DISPLAY_ROWS],
        }
    }

    pub(crate) fn push(&mut self, field: &dvb_vbi::TeletextDataField) {
        // EN 300 706 transmits each byte of the magazine/packet address and
        // data block LSB-first (§7.1.2/§8.1/§8.2's "bslbf" fields, EN 301 775
        // §4.5) — `txt_data_block_logical()` undoes that wire order; run
        // Hamming-8/4 / odd-parity decode (which assume the spec's bit order
        // directly) against its output, not the raw `txt_data_block`.
        let block: [u8; 42] = field.txt_data_block_logical();
        let block = &block;
        let Some(addr) = decode_packet_address(block[0], block[1]) else {
            return;
        };
        if addr.magazine != self.magazine {
            return;
        }
        if addr.row == 0 {
            let Some(header) = PageHeader::parse(block) else {
                return;
            };
            self.active = header.page == self.page;
            if self.active {
                if header.erase_page {
                    self.rows = [None; DISPLAY_ROWS];
                }
                self.inhibited = header.inhibit_display;
                self.boxed_page = header.subtitle || header.newsflash;
                self.national_option = header.national_option;
            }
            return;
        }
        if !self.active || usize::from(addr.row) > DISPLAY_ROWS {
            return;
        }
        self.rows[usize::from(addr.row) - 1] = Some(decode_row_codes(block));
    }

    /// The currently displayed subtitle text: non-empty rows 1-24 as rendered
    /// by [`render_row`], in row order, joined with `\n`. Empty if the page
    /// is currently inhibited (C10) or no row is non-empty.
    pub(crate) fn display_text(&self) -> String {
        if self.inhibited {
            return String::new();
        }
        let lines: Vec<String> = self
            .rows
            .iter()
            .flatten()
            .map(|codes| render_row(codes, self.national_option, self.boxed_page))
            .filter(|r| !r.is_empty())
            .collect();
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn hamming_8_4_round_trips_all_16_nibbles() {
        for nibble in 0u8..16 {
            let byte = encode_hamming_8_4(nibble);
            assert_eq!(
                decode_hamming_8_4(byte),
                Some(nibble),
                "nibble {nibble:#X} round-trip"
            );
        }
    }

    #[test]
    fn hamming_8_4_corrects_every_single_bit_error() {
        for nibble in 0u8..16 {
            let byte = encode_hamming_8_4(nibble);
            for bit in 0..8u8 {
                let corrupted = byte ^ (1 << bit);
                assert_eq!(
                    decode_hamming_8_4(corrupted),
                    Some(nibble),
                    "nibble {nibble:#X}, single-bit error at bit {bit} must be corrected"
                );
            }
        }
    }

    #[test]
    fn hamming_8_4_rejects_double_bit_errors() {
        // Flip 2 bits of a known-good codeword: per §8.2, "double bit errors
        // can be detected" (rejected, not silently accepted as some other
        // nibble).
        let byte = encode_hamming_8_4(0b0110);
        let corrupted = byte ^ 0b0000_0011; // flip bits 0 and 1
        assert_eq!(
            decode_hamming_8_4(corrupted),
            None,
            "a double-bit error must be rejected, not miscorrected"
        );
    }

    #[test]
    fn hamming_manual_cross_check_against_spec_encoding_equations() {
        // Manually compute nibble 0b0110 (D1=0,D2=1,D3=1,D4=0) by hand from
        // §8.2's stated equations, cross-checking `encode_hamming_8_4`:
        // P1 = 1^D1^D3^D4 = 1^0^1^0 = 0
        // P2 = 1^D1^D2^D4 = 1^0^1^0 = 0
        // P3 = 1^D1^D2^D3 = 1^0^1^1 = 1
        // P4 = 1^P1^D1^P2^D2^P3^D3^D4 = 1^0^0^0^1^1^1^0 = 0
        // wire bits (transmission order P1 D1 P2 D2 P3 D3 P4 D4) = 0 0 0 1 1 1 0 0
        // packed LSB-first: bit1=P1=0,bit2=D1=0,bit3=P2=0,bit4=D2=1,
        //                    bit5=P3=1,bit6=D3=1,bit7=P4=0,bit8=D4=0
        // byte = 0b0011_1000 = 0x38
        assert_eq!(encode_hamming_8_4(0b0110), 0x38);
        assert_eq!(decode_hamming_8_4(0x38), Some(0b0110));
    }

    #[test]
    fn odd_parity_round_trips_all_128_values() {
        for data7 in 0u8..128 {
            let byte = encode_odd_parity(data7);
            assert_eq!(
                byte.count_ones() % 2,
                1,
                "encoded byte must have odd parity"
            );
            assert_eq!(decode_odd_parity(byte), Some(data7));
        }
    }

    #[test]
    fn odd_parity_detects_but_does_not_correct() {
        let byte = encode_odd_parity(b'H' & 0x7F);
        let corrupted = byte ^ 0x01; // flip one data bit -> even parity
        assert_eq!(
            decode_odd_parity(corrupted),
            None,
            "odd parity only detects errors; a corrupted byte must not decode"
        );
    }

    #[test]
    fn latin_g0_base_matches_ascii_outside_reserved_positions() {
        for code in 0x20u8..0x7F {
            let is_reserved = matches!(
                code,
                0x23 | 0x24
                    | 0x40
                    | 0x5B
                    | 0x5C
                    | 0x5D
                    | 0x5E
                    | 0x5F
                    | 0x60
                    | 0x7B
                    | 0x7C
                    | 0x7D
                    | 0x7E
            );
            if !is_reserved {
                assert_eq!(
                    latin_g0_char(code, NationalOption::English),
                    code as char,
                    "non-reserved position {code:#04X} must match base ASCII"
                );
            }
        }
    }

    #[test]
    fn latin_g0_english_substitutions_verified_against_table_36() {
        // Values read directly from ETSI EN 300 706 Table 36's "English" row
        // (bitmap glyph chart, visually verified — see docs/teletext-subtitles.md).
        assert_eq!(latin_g0_char(0x23, NationalOption::English), '£');
        assert_eq!(latin_g0_char(0x24, NationalOption::English), '$');
        assert_eq!(latin_g0_char(0x40, NationalOption::English), '@');
        assert_eq!(latin_g0_char(0x5B, NationalOption::English), '←');
        assert_eq!(latin_g0_char(0x5C, NationalOption::English), '½');
        assert_eq!(latin_g0_char(0x5D, NationalOption::English), '→');
        assert_eq!(latin_g0_char(0x5E, NationalOption::English), '↑');
        assert_eq!(latin_g0_char(0x5F, NationalOption::English), '#');
        assert_eq!(latin_g0_char(0x7B, NationalOption::English), '¼');
        assert_eq!(latin_g0_char(0x7C, NationalOption::English), '‖');
        assert_eq!(latin_g0_char(0x7D, NationalOption::English), '¾');
        assert_eq!(latin_g0_char(0x7E, NationalOption::English), '÷');
    }

    #[test]
    fn latin_g0_reserved_option_substitutes_nothing() {
        assert_eq!(latin_g0_char(0x24, NationalOption::Reserved(7)), '$');
        assert_eq!(latin_g0_char(0x5B, NationalOption::Reserved(7)), '[');
    }

    #[test]
    fn latin_g0_other_rows_of_table_36() {
        let row = |option| -> String {
            NATIONAL_POSITIONS
                .iter()
                .map(|&c| latin_g0_char(c, option))
                .collect()
        };
        assert_eq!(row(NationalOption::German), "#$§ÄÖÜ^_°äöüß");
        assert_eq!(row(NationalOption::Italian), "£$é°ç→↑#ùàòèì");
        assert_eq!(row(NationalOption::PortugueseSpanish), "ç$¡áéíóú¿üñèà");
        assert_eq!(row(NationalOption::Turkish), "\u{20BA}ğİŞÖÇÜĞışöçü");
    }

    #[test]
    fn latin_g0_french_row_of_table_36() {
        let got: String = NATIONAL_POSITIONS
            .iter()
            .map(|&c| latin_g0_char(c, NationalOption::French))
            .collect();
        assert_eq!(got, "éïàëêùî#èâôûç");
    }

    #[test]
    fn page_assembler_applies_the_header_national_option() {
        let mut asm = PageAssembler::new(8, 0x88);
        asm.push(&field_from_block(
            build_header_block(8, 0x88, true, true, false, NationalOption::French),
            0,
        ));
        let row = [
            START_BOX, START_BOX, b'd', 0x23, b'p', b'a', b's', b's', b'e', b'r', END_BOX,
        ];
        asm.push(&field_from_block(build_row_bytes(8, 20, &row), 20));
        assert_eq!(asm.display_text(), "dépasser");
    }

    #[test]
    fn boxed_page_shows_only_text_between_start_box_and_end_box() {
        let mut asm = PageAssembler::new(8, 0x88);
        asm.push(&field_from_block(
            build_header_block(8, 0x88, true, true, false, NationalOption::English),
            0,
        ));
        let mut row = vec![b' ', b' ', b' ', b'x', START_BOX, START_BOX, b' '];
        row.extend_from_slice(b"Hi there");
        row.extend_from_slice(&[b' ', END_BOX, END_BOX, b'y', b'z']);
        asm.push(&field_from_block(build_row_bytes(8, 20, &row), 20));
        assert_eq!(asm.display_text(), "Hi there");
    }

    #[test]
    fn row_without_start_box_is_not_displayed_on_a_boxed_page() {
        let mut asm = PageAssembler::new(8, 0x88);
        asm.push(&field_from_block(
            build_header_block(8, 0x88, true, true, false, NationalOption::English),
            0,
        ));
        asm.push(&field_from_block(build_row_block(8, 20, "outside"), 20));
        assert_eq!(asm.display_text(), "");
    }

    #[test]
    fn box_attributes_are_ignored_on_a_page_that_is_not_boxed() {
        let mut asm = PageAssembler::new(8, 0x88);
        asm.push(&field_from_block(
            build_header_block(8, 0x88, true, false, false, NationalOption::English),
            0,
        ));
        asm.push(&field_from_block(build_row_block(8, 20, "plain"), 20));
        assert_eq!(asm.display_text(), "plain");
    }

    #[test]
    fn control_codes_render_as_space_and_0x7f_as_full_block() {
        assert_eq!(latin_g0_char(0x00, NationalOption::English), ' ');
        assert_eq!(latin_g0_char(0x1F, NationalOption::English), ' ');
        assert_eq!(latin_g0_char(0x7F, NationalOption::English), '\u{2588}');
    }

    #[test]
    fn national_option_decodes_all_8_values() {
        assert_eq!(NationalOption::from_bits(0), NationalOption::English);
        assert_eq!(NationalOption::from_bits(1), NationalOption::German);
        assert_eq!(
            NationalOption::from_bits(2),
            NationalOption::SwedishFinnishHungarian
        );
        assert_eq!(NationalOption::from_bits(3), NationalOption::Italian);
        assert_eq!(NationalOption::from_bits(4), NationalOption::French);
        assert_eq!(
            NationalOption::from_bits(5),
            NationalOption::PortugueseSpanish
        );
        assert_eq!(NationalOption::from_bits(6), NationalOption::CzechSlovak);
        assert_eq!(NationalOption::from_bits(7), NationalOption::Reserved(7));
    }

    #[test]
    fn packet_address_decodes_magazine_0_as_magazine_8() {
        // magazine field = 0 (nibble low 3 bits = 0), row = 0.
        let b4 = encode_hamming_8_4(0); // magazine field 0, Y bit0 = 0
        let b5 = encode_hamming_8_4(0); // Y bits 1-4 = 0
        let addr = decode_packet_address(b4, b5).unwrap();
        assert_eq!(
            addr.magazine, 8,
            "magazine field 0 must decode as magazine 8"
        );
        assert_eq!(addr.row, 0);
    }

    #[test]
    fn packet_address_decodes_magazine_and_row() {
        // magazine field = 3, Y = 17 (0b10001): Y bit0=1 (goes in byte4 D4),
        // Y bits1-4 = 0b1000 (goes in byte5 nibble).
        let b4 = encode_hamming_8_4(0b1_011); // D1..D3=011(mag=3), D4=1(Y bit0)
        let b5 = encode_hamming_8_4(0b1000); // Y bits1..4 = 1000
        let addr = decode_packet_address(b4, b5).unwrap();
        assert_eq!(addr.magazine, 3);
        assert_eq!(addr.row, 17);
    }

    #[test]
    fn page_assembler_tracks_target_page_and_ignores_others() {
        let mut asm = PageAssembler::new(8, 0x88);

        // Header for magazine 8, page 0x88, C6 subtitle set, C4 erase set.
        let header_block = build_header_block(8, 0x88, true, true, false, NationalOption::English);
        asm.push(&field_from_block(header_block, 0));
        assert!(asm.active);

        // Row 20: "HI"
        let row_block = build_boxed_row(8, 20, "HI");
        asm.push(&field_from_block(row_block, 20));
        assert_eq!(asm.display_text(), "HI");

        // A row for a DIFFERENT magazine must be ignored.
        let other_mag_row = build_boxed_row(1, 21, "IGNORED");
        asm.push(&field_from_block(other_mag_row, 21));
        assert_eq!(
            asm.display_text(),
            "HI",
            "other magazine's row must be ignored"
        );

        // A header for a different page in the same magazine deactivates us.
        let other_page_header =
            build_header_block(8, 0x01, true, false, false, NationalOption::English);
        asm.push(&field_from_block(other_page_header, 0));
        assert!(!asm.active);
        let row_after_switch = build_boxed_row(8, 21, "SHOULD NOT APPEAR");
        asm.push(&field_from_block(row_after_switch, 21));
        assert_eq!(
            asm.display_text(),
            "HI",
            "rows while another page is active must not be stored"
        );

        // Switch back with erase -> old row content is cleared.
        let back_header = build_header_block(8, 0x88, true, true, false, NationalOption::English);
        asm.push(&field_from_block(back_header, 0));
        assert_eq!(asm.display_text(), "", "erase_page must clear old rows");
    }

    #[test]
    fn page_assembler_inhibit_display_blanks_text() {
        let mut asm = PageAssembler::new(8, 0x88);
        asm.push(&field_from_block(
            build_header_block(8, 0x88, true, true, false, NationalOption::English),
            0,
        ));
        asm.push(&field_from_block(build_boxed_row(8, 20, "HELLO"), 20));
        assert_eq!(asm.display_text(), "HELLO");

        // A header with C10 (inhibit_display) set, but C4 (erase) NOT set,
        // must blank the display while leaving the row buffer itself
        // populated (proves inhibit is a display-time gate, not a clear).
        asm.push(&field_from_block(
            build_header_block(8, 0x88, false, false, true, NationalOption::English),
            0,
        ));
        assert_eq!(
            asm.display_text(),
            "",
            "inhibit_display must blank the text"
        );
        assert!(
            asm.rows[19].is_some(),
            "inhibit_display must not clear the underlying row buffer"
        );
    }

    /// Test-only helper: build a 42-byte `txt_data_block` for a page header
    /// packet (`Y=0`) at `(magazine, page)` with the given C4/C6/C10 bits.
    /// Uses [`encode_hamming_8_4`] — the same function proven correct by the
    /// round-trip and manual-cross-check tests above — to build spec-valid
    /// wire bytes (this project's established "construct from verified spec
    /// encode rules" fixture fallback; see `docs/teletext-subtitles.md`).
    pub(super) fn build_header_block(
        magazine: u8,
        page: u8,
        erase_page: bool,
        subtitle: bool,
        inhibit_display: bool,
        option: NationalOption,
    ) -> [u8; 42] {
        let mut b = [0u8; 42];
        let mag_field = if magazine == 8 { 0 } else { magazine };
        b[0] = encode_hamming_8_4(mag_field); // Y bit0 = 0 (row 0)
        b[1] = encode_hamming_8_4(0); // Y bits1..4 = 0
        b[2] = encode_hamming_8_4(page & 0xF); // page units
        b[3] = encode_hamming_8_4((page >> 4) & 0xF); // page tens
        b[4] = encode_hamming_8_4(0); // S1 = 0
        let c4 = u8::from(erase_page);
        b[5] = encode_hamming_8_4(c4 << 3); // S2=0, C4
        b[6] = encode_hamming_8_4(0); // S3 = 0
        let c6 = u8::from(subtitle);
        b[7] = encode_hamming_8_4(c6 << 3); // S4=0, C5=0, C6
        let c10 = u8::from(inhibit_display);
        b[8] = encode_hamming_8_4(c10 << 3); // C7=C8=C9=0, C10
        let opt_bits = match option {
            NationalOption::English => 0u8,
            NationalOption::German => 1,
            NationalOption::SwedishFinnishHungarian => 2,
            NationalOption::Italian => 3,
            NationalOption::French => 4,
            NationalOption::PortugueseSpanish => 5,
            NationalOption::CzechSlovak => 6,
            NationalOption::Reserved(v) => v,
            other => panic!("{other} has no Level 1 header encoding"),
        };
        let c12 = (opt_bits >> 2) & 1;
        let c13 = (opt_bits >> 1) & 1;
        let c14 = opt_bits & 1;
        b[9] = encode_hamming_8_4((c14 << 3) | (c13 << 2) | (c12 << 1)); // C11=0
        for byte in b.iter_mut().skip(10) {
            *byte = encode_odd_parity(0x20); // row-0 text: spaces
        }
        b
    }

    /// Test-only helper: build a 42-byte `txt_data_block` for a display row
    /// packet (`Y=1..=24`) carrying `text` (ASCII only, right-padded with
    /// spaces to 40 columns), odd-parity encoded per §8.1.
    pub(super) fn build_row_block(magazine: u8, row: u8, text: &str) -> [u8; 42] {
        assert!((1..=24).contains(&row));
        let mut b = [0u8; 42];
        let mag_field = if magazine == 8 { 0 } else { magazine };
        let y0 = row & 1;
        b[0] = encode_hamming_8_4(mag_field | (y0 << 3));
        b[1] = encode_hamming_8_4(row >> 1);
        let bytes = text.as_bytes();
        for i in 0..40usize {
            let ch = bytes.get(i).copied().unwrap_or(b' ');
            b[2 + i] = encode_odd_parity(ch & 0x7F);
        }
        b
    }

    /// A row whose text is wrapped in the doubled Start Box / End Box pair a
    /// subtitle page carries.
    pub(super) fn build_boxed_row(magazine: u8, row: u8, text: &str) -> [u8; 42] {
        let mut codes = vec![START_BOX, START_BOX];
        codes.extend_from_slice(text.as_bytes());
        codes.extend_from_slice(&[END_BOX, END_BOX]);
        build_row_bytes(magazine, row, &codes)
    }

    /// Like [`build_row_block`] but from raw 7-bit codes, so spacing
    /// attributes can be placed in the row.
    pub(super) fn build_row_bytes(magazine: u8, row: u8, codes: &[u8]) -> [u8; 42] {
        assert!((1..=24).contains(&row));
        let mut b = [0u8; 42];
        let mag_field = if magazine == 8 { 0 } else { magazine };
        b[0] = encode_hamming_8_4(mag_field | ((row & 1) << 3));
        b[1] = encode_hamming_8_4(row >> 1);
        for i in 0..40usize {
            let code = codes.get(i).copied().unwrap_or(b' ');
            b[2 + i] = encode_odd_parity(code & 0x7F);
        }
        b
    }

    pub(super) fn field_from_block(block: [u8; 42], line: u8) -> dvb_vbi::TeletextDataField {
        // `build_header_block`/`build_row_block` produce the spec's own
        // (canonical) byte values via `encode_hamming_8_4`/`encode_odd_parity`;
        // reverse each bit to emulate the LSB-first wire bytes `push` now
        // un-reverses, so these tests exercise the real wire path.
        dvb_vbi::TeletextDataField {
            header: dvb_vbi::LineHeader::new(true, line % 24),
            framing_code: dvb_vbi::FRAMING_CODE_EBU,
            txt_data_block: block.map(u8::reverse_bits),
        }
    }
}
