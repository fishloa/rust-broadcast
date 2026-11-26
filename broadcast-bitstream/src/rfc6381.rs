//! RFC 6381 codec strings built from a parameter set (or the codec facts one
//! carries).
//!
//! `avc1.PPCCLL` for H.264, the HEVC registration's `hvc1.…` form
//! (ISO/IEC 14496-15:2017 Annex E.3, restated in ETSI TS 103 285 §5.2.2) and
//! the VVC registration's `vvc1.…` form (ISO/IEC 14496-15:2022 §11.3.4) each
//! follow their codec's feature; `mp4a.40.<AOT>` describes an audio
//! configuration and has none.
//!
//! Each helper is a pure formatting function: the caller passes the fields it
//! already decoded, so the string never disagrees with the facts.

#[cfg(feature = "hevc")]
use crate::HevcSpsInfo;
use alloc::string::String;

#[cfg(feature = "hevc")]
/// Byte length of the `general_constraint_indicator_flags` tail of the HEVC
/// 48-bit constraint flags (H.265 §7.3.3).
const HEVC_CONSTRAINT_BYTES_LEN: usize = 6;

#[cfg(feature = "h264")]
/// Capacity of the `avc1.PPCCLL` codec string.
const AVC1_CODEC_STRING_CAPACITY: usize = 12;
#[cfg(feature = "vvc")]
/// Capacity of the `vvc1.…` codec string.
const VVC1_CODEC_STRING_CAPACITY: usize = 24;
/// Capacity of the `mp4a.40.…` codec string.
const MP4A_CODEC_STRING_CAPACITY: usize = 16;
/// Decimal digits of a `u8` (0..=255).
const U8_DECIMAL_DIGITS: usize = 3;
/// Radix of the decimal conversion.
const DECIMAL_RADIX: u8 = 10;

// ---------------------------------------------------------------------------
// RFC 6381 helpers
// ---------------------------------------------------------------------------

#[cfg(feature = "h264")]
/// Build RFC 6381 `avc1.PPCCLL` codec string.
///
/// UPPERCASE hex: `avc1.<profile_idc><constraint_flags><level_idc>`,
/// each as two hex digits (e.g. `avc1.4D400D`).
pub fn rfc6381_avc1(profile_idc: u8, constraint_flags: u8, level_idc: u8) -> String {
    let mut s = String::with_capacity(AVC1_CODEC_STRING_CAPACITY);
    s.push_str("avc1.");
    write_hex_byte(&mut s, profile_idc);
    write_hex_byte(&mut s, constraint_flags);
    write_hex_byte(&mut s, level_idc);
    s
}

#[cfg(feature = "hevc")]
const HVC1_CODEC_STRING_CAPACITY: usize = 48;

#[cfg(feature = "hevc")]
/// `general_profile_space` 1..=3 as the codec-string prefix letters.
const HEVC_PROFILE_SPACE_LETTERS: [char; 3] = ['A', 'B', 'C'];

#[cfg(feature = "hevc")]
/// Build the `hvc1.…` codec string of ISO/IEC 14496-15:2017 Annex E.3 (the HEVC registration
/// under RFC 6381): `hvc1.<space letter><profile_idc>.<compat>.<tier><level>
/// [.<constraint byte>…]`, where `general_profile_idc` and `general_level_idc`
/// are decimal, `<compat>` is the bit-reversed 32-bit compatibility word in hex
/// without leading zeros, and trailing zero constraint bytes are dropped (the
/// grammar, with the corrigendum applied, as restated in ETSI TS 103 285
/// §5.2.2, `private/specs-verified/dash/docs-specs/etsi-ts-103-285/ch05-video.md`).
pub fn rfc6381_hvc1(info: &HevcSpsInfo) -> String {
    let mut s = String::with_capacity(HVC1_CODEC_STRING_CAPACITY);
    s.push_str("hvc1.");
    if let Some(letter) = usize::from(info.general_profile_space())
        .checked_sub(1)
        .and_then(|i| HEVC_PROFILE_SPACE_LETTERS.get(i))
    {
        s.push(*letter);
    }
    s.push_str(&alloc::format!("{}", info.general_profile_idc()));
    s.push('.');
    s.push_str(&alloc::format!(
        "{:X}",
        info.general_profile_compatibility_flags().reverse_bits()
    ));
    s.push('.');
    s.push(if info.general_tier_flag() { 'H' } else { 'L' });
    s.push_str(&alloc::format!("{}", info.general_level_idc()));

    let constraints = info.general_constraint_indicator_flags();
    let be = constraints.to_be_bytes();
    let bytes = &be[be.len() - HEVC_CONSTRAINT_BYTES_LEN..];

    let last_nonzero = bytes.iter().rposition(|&b| b != 0);
    if let Some(end) = last_nonzero {
        for (i, &b) in bytes.iter().enumerate() {
            if i > end {
                break;
            }
            s.push('.');
            write_hex_byte(&mut s, b);
        }
    }

    s
}

#[cfg(feature = "vvc")]
/// Build the RFC 6381 `vvc1.…` codec string for H.266/VVC.
///
/// Per the VVC file-format registration (ISO/IEC 14496-15:2022 §11.3.4, the
/// VVC analogue of the RFC 6381 §3.3 HEVC form):
/// `vvc1.<general_profile_idc>.<tier><general_level_idc>[.CTA-<constraint…>]`
/// where the tier prefix is `L` for main tier and `H` for high tier, the level
/// is the decimal `general_level_idc`, and the constraint suffix (`CTA-` +
/// hex, dropping trailing zero bytes) is emitted only when constraint bytes are
/// present. `general_profile_idc` and the level are written in decimal.
pub fn rfc6381_vvc1(
    general_profile_idc: u8,
    general_tier_flag: bool,
    general_level_idc: u8,
    general_constraint_info: &[u8],
    num_bytes_constraint_info: u8,
) -> String {
    let mut s = String::with_capacity(VVC1_CODEC_STRING_CAPACITY);
    s.push_str("vvc1.");
    write_decimal(&mut s, general_profile_idc);
    s.push('.');
    s.push(if general_tier_flag { 'H' } else { 'L' });
    write_decimal(&mut s, general_level_idc);

    // Constraint suffix: the general_constraint_info payload, already
    // MSB-aligned into its `num_bytes_constraint_info`-byte block, emitted as
    // `CTA-` + hex with trailing zero bytes dropped.
    if num_bytes_constraint_info > 0 {
        let n = num_bytes_constraint_info as usize;
        // Only the bytes actually read are meaningful; `n` is the block width.
        let block = &general_constraint_info[..general_constraint_info.len().min(n)];
        if let Some(end) = block.iter().rposition(|&b| b != 0) {
            s.push_str(".CTA-");
            for &b in &block[..=end] {
                write_hex_byte(&mut s, b);
            }
        }
    }
    s
}

/// Build RFC 6381 `mp4a.40.<AOT>` codec string from AAC `AudioObjectType`.
pub fn rfc6381_mp4a(aot: u8) -> String {
    let mut s = String::with_capacity(MP4A_CODEC_STRING_CAPACITY);
    s.push_str("mp4a.40.");
    write_decimal(&mut s, aot);
    s
}

#[cfg(any(feature = "h264", feature = "hevc", feature = "vvc"))]
fn write_hex_byte(s: &mut String, byte: u8) {
    // Writing to a `String` cannot fail.
    let _ = core::fmt::Write::write_fmt(s, format_args!("{byte:02X}"));
}

fn write_decimal(s: &mut String, mut n: u8) {
    if n == 0 {
        s.push('0');
        return;
    }
    let mut buf = [0u8; U8_DECIMAL_DIGITS + 1];
    let mut len = 0;
    while n > 0 {
        buf[len] = b'0' + (n % DECIMAL_RADIX);
        n /= DECIMAL_RADIX;
        len += 1;
    }
    for i in (0..len).rev() {
        s.push(buf[i] as char);
    }
}
