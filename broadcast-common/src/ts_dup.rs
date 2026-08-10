//! Legal-duplicate transport-stream-packet detection — ITU-T H.222.0
//! (08/2023) / ISO/IEC 13818-1 §2.4.3.3.
//!
//! Cited via `mpeg-ts/docs/README.md` (§2.4.3.3 — duplicate packets), which
//! carries the verbatim clause (Rec. ITU-T H.222.0 (08/2023) §2.4.3.3, PDF
//! p. 48):
//!
//! > In transport streams, duplicate packets may be sent as two, and only
//! > two, consecutive transport stream packets of the same PID. The
//! > duplicate packets shall have the same continuity_counter value as the
//! > original packet and the adaptation_field_control field shall be equal
//! > to '01' or '11'. In duplicate packets each byte of the original packet
//! > shall be duplicated, with the exception that in the program clock
//! > reference fields, if present, a valid value shall be encoded.
//!
//! Three independent properties fall out of that sentence:
//!
//! 1. **"each byte of the original packet"**, with the PCR fields the
//!    *sole* exception — a payload-only comparison is too lenient. A packet
//!    differing in, say, `splice_countdown` or OPCR is NOT a legal
//!    duplicate. [`is_legal_duplicate_pair`] implements exactly this byte
//!    comparison.
//! 2. **`adaptation_field_control` must be `01` or `11`** — a byte-identical
//!    repeat of a non-payload-bearing packet (`00`/`10`) is not a "duplicate"
//!    in the spec's sense at all.
//! 3. **"two, and only two"** — a third consecutive repeat is an error
//!    regardless of byte-identity. This is necessarily stateful (a pairwise
//!    comparison alone cannot see the run length), so it is exposed via
//!    [`check_duplicate`], which takes whether the *previous* transition in
//!    the run already consumed the one legal repeat.
//!
//! This module was extracted (issue #956 follow-up) from three independent,
//! disagreeing hand-rolled copies of this rule in `dvb-conformance`,
//! `media-doctor` and `ts-fix`.

/// Byte offset of `adaptation_field_control` (bits `[5:4]`) and
/// `continuity_counter` (bits `[3:0]`) within a raw TS packet — ITU-T
/// H.222.0 (08/2023) §2.4.3.2 Table 2-5 / §2.4.3.3.
const AFC_BYTE: usize = 3;

/// `adaptation_field_control` bit indicating a payload is present (`01` or
/// `11`) — H.222.0 Table 2-5.
const PAYLOAD_FLAG: u8 = 0x10;

/// `adaptation_field_control` bit indicating an adaptation field is present
/// (`10` or `11`) — H.222.0 Table 2-5.
const ADAPTATION_FLAG: u8 = 0x20;

/// Byte offset of `adaptation_field_length` within a raw TS packet —
/// H.222.0 §2.4.3.4.
const AF_LEN_BYTE: usize = 4;

/// Byte offset of the adaptation-field flags byte (the byte immediately
/// after `adaptation_field_length`) within a raw TS packet — H.222.0
/// §2.4.3.4 Table 2-6.
const AF_FLAGS_BYTE: usize = 5;

/// Adaptation-field flags-byte bit for `PCR_flag` — H.222.0 §2.4.3.4
/// Table 2-6.
const PCR_FLAG: u8 = 0x10;

/// Byte offset (relative to the start of the packet) of the first PCR byte,
/// when `PCR_flag` is set — immediately after the flags byte, H.222.0
/// §2.4.3.5.
const PCR_FIELD_START: usize = AF_FLAGS_BYTE + 1;

/// Encoded PCR field width: 33-bit `program_clock_reference_base` + 6
/// reserved bits + 9-bit `program_clock_reference_extension` = 48 bits —
/// H.222.0 §2.4.3.5.
const PCR_FIELD_LEN: usize = 6;

/// Locate the PCR field's byte range within `pkt`, if one is present.
///
/// Returns `None` when the packet carries no adaptation field, an empty
/// adaptation field, no `PCR_flag`, or is too short to hold the field it
/// claims to have — any of which means there is no PCR exception to apply.
fn pcr_field_range(pkt: &[u8]) -> Option<(usize, usize)> {
    if pkt.len() <= AF_FLAGS_BYTE {
        return None;
    }
    if pkt[AFC_BYTE] & ADAPTATION_FLAG == 0 {
        return None;
    }
    let af_len = pkt[AF_LEN_BYTE] as usize;
    if af_len == 0 {
        return None;
    }
    if pkt[AF_FLAGS_BYTE] & PCR_FLAG == 0 {
        return None;
    }
    let end = PCR_FIELD_START + PCR_FIELD_LEN;
    if end > pkt.len() {
        return None;
    }
    Some((PCR_FIELD_START, end))
}

/// Whether `curr` is a legal §2.4.3.3 duplicate of `prev`, ignoring the
/// "two, and only two" cardinality rule (see [`check_duplicate`] for that).
///
/// `prev`/`curr` are raw TS-packet bytes — any equal length of at least 4
/// bytes (the usual case is the fixed 188-byte packet, but this makes no
/// assumption about total packet size). Differing lengths are never a
/// duplicate.
///
/// A duplicate requires, per the spec text quoted at the module level:
/// - `curr`'s `adaptation_field_control` indicates a payload (`01` or
///   `11`) — checking only `curr` is sufficient because the byte-for-byte
///   comparison below (which includes byte 3, the AFC/CC byte) forces
///   `prev`'s AFC bits to match whenever this function returns `true`;
/// - every byte of `prev` and `curr` is identical, with the sole exception
///   of the 6-byte PCR field (located from `curr`'s own adaptation-field
///   flags, which the same byte-3/byte-4/byte-5 equality requirement above
///   forces to agree with `prev`'s).
pub fn is_legal_duplicate_pair(prev: &[u8], curr: &[u8]) -> bool {
    if prev.len() != curr.len() || prev.len() <= AFC_BYTE {
        return false;
    }
    if curr[AFC_BYTE] & PAYLOAD_FLAG == 0 {
        // Property 2: adaptation_field_control must be '01' or '11'.
        return false;
    }

    let pcr_range = pcr_field_range(curr);

    for i in 0..prev.len() {
        let in_pcr_field = matches!(pcr_range, Some((start, end)) if i >= start && i < end);
        if !in_pcr_field && prev[i] != curr[i] {
            // Property 1: every byte identical, PCR fields excepted.
            return false;
        }
    }
    true
}

/// Outcome of [`check_duplicate`]: whether `curr` legally continues (or
/// illegally over-continues) a duplicate run started by `prev`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DuplicateVerdict {
    /// `curr` is not a legal duplicate of `prev` at all — either
    /// `adaptation_field_control` was not `01`/`11`, or some byte other
    /// than the PCR field differs. Callers should fall through to their
    /// normal continuity-counter check.
    NotDuplicate,
    /// `curr` is the single legal repeat of `prev` — §2.4.3.3 permits
    /// exactly one.
    Legal,
    /// `curr` is byte-identical (PCR excepted) to `prev`, but the caller
    /// signalled that the *previous* transition already consumed the one
    /// legal repeat (`dup_already_used == true`). Per "two, and only two
    /// consecutive" packets, a third consecutive repeat is itself a
    /// continuity fault.
    IllegalThirdRepeat,
}

impl DuplicateVerdict {
    /// Static label for this verdict (issue #204 convention).
    pub fn name(&self) -> &'static str {
        match self {
            Self::NotDuplicate => "not duplicate",
            Self::Legal => "legal",
            Self::IllegalThirdRepeat => "illegal third repeat",
        }
    }
}

crate::impl_spec_display!(DuplicateVerdict);

/// Check whether `curr` is a legal duplicate of `prev`, folding in the
/// "two, and only two" cardinality rule via `dup_already_used`: the
/// caller's record of whether the *previous* packet on this PID was
/// already accepted as the one legal repeat of ITS predecessor.
///
/// Typical per-PID caller state is just this one `bool`, reset to `false`
/// whenever [`DuplicateVerdict::NotDuplicate`] is returned (a fresh
/// original packet has been seen) and set to `true` after
/// [`DuplicateVerdict::Legal`].
pub fn check_duplicate(prev: &[u8], curr: &[u8], dup_already_used: bool) -> DuplicateVerdict {
    if !is_legal_duplicate_pair(prev, curr) {
        return DuplicateVerdict::NotDuplicate;
    }
    if dup_already_used {
        DuplicateVerdict::IllegalThirdRepeat
    } else {
        DuplicateVerdict::Legal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 188-byte packet, sync=0x47, given PID, AFC/CC byte, and a payload
    /// filled with a repeating pattern. No adaptation field.
    fn payload_packet(pid: u16, afc_cc: u8, fill: u8) -> [u8; 188] {
        let mut pkt = [fill; 188];
        pkt[0] = 0x47;
        pkt[1] = ((pid >> 8) as u8) & 0x1F;
        pkt[2] = (pid & 0xFF) as u8;
        pkt[3] = afc_cc;
        pkt
    }

    /// Packet with an adaptation field carrying PCR (+ optionally OPCR and
    /// splice_countdown), followed by payload filled with `fill`.
    ///
    /// Layout: `[sync][pid_hi][pid_lo][afc_cc][af_len][flags][PCR x6]
    /// [OPCR x6 if opcr][splice_countdown if splice]...[payload]`.
    fn packet_with_adaptation(
        pid: u16,
        afc_cc: u8,
        pcr: [u8; 6],
        opcr: Option<[u8; 6]>,
        splice_countdown: Option<u8>,
        fill: u8,
    ) -> [u8; 188] {
        let mut pkt = [fill; 188];
        pkt[0] = 0x47;
        pkt[1] = ((pid >> 8) as u8) & 0x1F;
        pkt[2] = (pid & 0xFF) as u8;
        pkt[3] = afc_cc | ADAPTATION_FLAG;

        let mut flags = PCR_FLAG;
        let mut body_len = 1 + 6; // flags byte + PCR
        if opcr.is_some() {
            flags |= 0x08; // OPCR_flag
            body_len += 6;
        }
        if splice_countdown.is_some() {
            flags |= 0x04; // splicing_point_flag
            body_len += 1;
        }

        pkt[4] = body_len as u8; // adaptation_field_length
        pkt[5] = flags;
        pkt[6..12].copy_from_slice(&pcr);
        let mut cursor = 12usize;
        if let Some(o) = opcr {
            pkt[cursor..cursor + 6].copy_from_slice(&o);
            cursor += 6;
        }
        if let Some(sc) = splice_countdown {
            pkt[cursor] = sc;
            cursor += 1;
        }
        // Rest of packet after the adaptation field is payload (already
        // filled with `fill` from the initial array construction);
        // `cursor` marks where it starts, which callers may mutate.
        let _ = cursor;
        pkt
    }

    #[test]
    fn byte_identical_is_legal() {
        let a = payload_packet(0x0100, 0x10, 0xAB);
        let b = a;
        assert!(is_legal_duplicate_pair(&a, &b));
    }

    #[test]
    fn pcr_only_difference_is_legal() {
        let a = packet_with_adaptation(0x0100, 0x10, [0, 0, 0, 0, 0, 0], None, None, 0xAB);
        let mut b = a;
        b[6..12].copy_from_slice(&[0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC]);
        assert!(is_legal_duplicate_pair(&a, &b));

        // Bite-check: an unrelated byte difference elsewhere still fails —
        // proves the PCR exemption isn't accidentally exempting everything.
        let mut c = a;
        c[6..12].copy_from_slice(&[0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC]);
        c[20] ^= 0xFF;
        assert!(!is_legal_duplicate_pair(&a, &c));
    }

    #[test]
    fn payload_difference_is_illegal() {
        let a = payload_packet(0x0100, 0x10, 0xAB);
        let mut b = a;
        b[100] ^= 0xFF; // one payload byte differs
        assert!(!is_legal_duplicate_pair(&a, &b));
    }

    #[test]
    fn splice_countdown_difference_is_illegal() {
        let a = packet_with_adaptation(0x0100, 0x10, [0, 0, 0, 0, 0, 0], None, Some(5), 0xCD);
        let mut b = a;
        // Flip the splice_countdown byte only (offset 12, since no OPCR).
        b[12] ^= 0x01;
        assert!(!is_legal_duplicate_pair(&a, &b));
    }

    #[test]
    fn opcr_difference_is_illegal() {
        let a = packet_with_adaptation(
            0x0100,
            0x10,
            [0, 0, 0, 0, 0, 0],
            Some([1, 2, 3, 4, 5, 6]),
            None,
            0xCD,
        );
        let mut b = a;
        // OPCR occupies bytes 12..18; flip one of them.
        b[12] ^= 0x01;
        assert!(!is_legal_duplicate_pair(&a, &b));
    }

    #[test]
    fn wrong_adaptation_field_control_is_not_a_duplicate() {
        // Byte-identical, but AFC = '00' (reserved, no payload) on both —
        // not a "duplicate" under the spec's definition regardless of
        // byte-identity.
        let a = payload_packet(0x0100, 0x00, 0xAB);
        let b = a;
        assert!(!is_legal_duplicate_pair(&a, &b));

        // AFC = '10' (adaptation-only, no payload): also not a duplicate.
        let mut c = packet_with_adaptation(0x0100, 0x00, [0; 6], None, None, 0xAB);
        c[3] &= !PAYLOAD_FLAG; // clear payload bit -> afc = '10'
        let d = c;
        assert!(!is_legal_duplicate_pair(&c, &d));
    }

    #[test]
    fn third_consecutive_repeat_is_illegal() {
        let a = payload_packet(0x0100, 0x10, 0xAB);
        let b = a; // byte-identical

        // First transition: no prior duplicate consumed -> Legal.
        assert_eq!(check_duplicate(&a, &b, false), DuplicateVerdict::Legal);

        // Second transition (a third consecutive identical packet): the
        // caller already consumed the one legal repeat -> illegal.
        let c = a;
        assert_eq!(
            check_duplicate(&b, &c, true),
            DuplicateVerdict::IllegalThirdRepeat
        );
    }

    #[test]
    fn not_a_duplicate_reports_not_duplicate_regardless_of_state() {
        let a = payload_packet(0x0100, 0x10, 0xAB);
        let mut b = a;
        b[50] ^= 0xFF;
        assert_eq!(
            check_duplicate(&a, &b, false),
            DuplicateVerdict::NotDuplicate
        );
        assert_eq!(
            check_duplicate(&a, &b, true),
            DuplicateVerdict::NotDuplicate
        );
    }

    #[test]
    fn different_lengths_are_never_duplicates() {
        let a = [0u8; 188];
        let b = [0u8; 187];
        assert!(!is_legal_duplicate_pair(&a, &b));
    }

    #[test]
    fn too_short_for_afc_byte_is_never_a_duplicate() {
        let a = [0x47, 0x00, 0x10];
        let b = a;
        assert!(!is_legal_duplicate_pair(&a, &b));
    }
}
