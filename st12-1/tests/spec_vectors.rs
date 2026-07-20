//! Byte-identical round-trip tests over **spec-derived** wire vectors (SMPTE
//! ST 12-1:2014 §9.2, Tables 2-5), one per boundary case (a fully-populated
//! mid-range frame, the all-maximum-values frame, and the all-zero frame).
//!
//! ## Provenance
//!
//! `st12-1` has no real captured LTC bitstream to draw a fixture from: this
//! workspace has no LTC producer/consumer today (unlike, say, `rtp-packet`'s
//! `tests/fixtures/rtp_simple.bin`, captured from this workspace's own
//! `transmux::RtpPacketiser`), and LTC is normally carried as a
//! biphase-mark-encoded analog audio signal — out of this crate's scope (see
//! `docs/st12-1.md`'s "Scope" section) — so there is no in-repo bitstream to
//! extract a logical codeword from either. Per the project's documented
//! fallback (`docs/CRATE-ACCEPTANCE.md` §3 — "no real capture exists, gate
//! is the biting round-trip"), every byte vector below is **computed
//! directly from the ST 12-1 §9.2 Tables 2-5 bit diagrams**
//! (`st12-1/docs/st12-1.md`) with a standalone Python script, independently
//! of this crate's own `Serialize` implementation — so a bug that made
//! `serialize`/`parse` agree with each other but disagree with the spec
//! would still be caught.

use broadcast_common::{Parse, Serialize};
use st12_1::{BinaryGroupFlags, BinaryGroupUsage, FrameRate, LtcFrame};

// ── Worked example: Time Address = 01:23:45:13 (docs/st12-1.md) ───────────
//
// hours=01 minutes=23 seconds=45 frames=13, drop_frame=0, color_frame=1,
// bit27=1 bit43=1 bit58=0 bit59=1 (30-frame: polarity=1 BGF0=1 BGF1=0 BGF2=1
// -> Table 1 "101" = unspecified time address, page/line), user bits 1..8.
#[rustfmt::skip]
const WORKED_VECTOR: [u8; 10] =
    [0x13, 0x29, 0x35, 0x4C, 0x53, 0x6A, 0x71, 0x88, 0xFC, 0xBF];

#[test]
fn worked_vector_parses_and_round_trips() {
    let f = LtcFrame::parse(&WORKED_VECTOR).expect("parse worked spec vector");
    assert_eq!(f.hours, 1);
    assert_eq!(f.minutes, 23);
    assert_eq!(f.seconds, 45);
    assert_eq!(f.frames, 13);
    assert!(!f.drop_frame_flag);
    assert!(f.color_frame_flag);
    assert_eq!(f.user_bits, [1, 2, 3, 4, 5, 6, 7, 8]);

    assert!(f.polarity_correction(FrameRate::Fps30));
    assert_eq!(
        f.binary_group_flags(FrameRate::Fps30),
        BinaryGroupFlags {
            bgf2: true,
            bgf1: false,
            bgf0: true
        }
    );
    assert_eq!(
        f.binary_group_flags(FrameRate::Fps30).usage(),
        BinaryGroupUsage::UnspecifiedPageLine
    );

    let mut out = [0u8; st12_1::FRAME_LEN];
    f.serialize_into(&mut out).unwrap();
    assert_eq!(
        out, WORKED_VECTOR,
        "byte-identical to the spec-derived vector"
    );
}

// ── All-maximum-values frame: 23:59:59:29, every flag set, every user bit
// 0xF ────────────────────────────────────────────────────────────────────
#[rustfmt::skip]
const MAX_VECTOR: [u8; 10] =
    [0xF9, 0xFE, 0xF9, 0xFD, 0xF9, 0xFD, 0xF3, 0xFE, 0xFC, 0xBF];

#[test]
fn max_vector_parses_and_round_trips() {
    let f = LtcFrame::parse(&MAX_VECTOR).expect("parse max spec vector");
    assert_eq!(f.hours, 23);
    assert_eq!(f.minutes, 59);
    assert_eq!(f.seconds, 59);
    assert_eq!(f.frames, 29);
    assert!(f.drop_frame_flag);
    assert!(f.color_frame_flag);
    assert!(f.flag_bit_27 && f.flag_bit_43 && f.flag_bit_58 && f.flag_bit_59);
    assert_eq!(f.user_bits, [0x0F; 8]);

    let mut out = [0u8; st12_1::FRAME_LEN];
    f.serialize_into(&mut out).unwrap();
    assert_eq!(out, MAX_VECTOR, "byte-identical to the spec-derived vector");
}

// ── All-zero frame: 00:00:00:00, no flags, no user bits ────────────────────
#[rustfmt::skip]
const ZERO_VECTOR: [u8; 10] =
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFC, 0xBF];

#[test]
fn zero_vector_parses_and_round_trips() {
    let f = LtcFrame::parse(&ZERO_VECTOR).expect("parse zero spec vector");
    assert_eq!(f.hours, 0);
    assert_eq!(f.minutes, 0);
    assert_eq!(f.seconds, 0);
    assert_eq!(f.frames, 0);
    assert!(!f.drop_frame_flag);
    assert!(!f.color_frame_flag);
    assert_eq!(f.user_bits, [0; 8]);
    assert_eq!(
        f.binary_group_flags(FrameRate::Fps30).usage(),
        BinaryGroupUsage::UnspecifiedUnspecified
    );

    let mut out = [0u8; st12_1::FRAME_LEN];
    f.serialize_into(&mut out).unwrap();
    assert_eq!(
        out, ZERO_VECTOR,
        "byte-identical to the spec-derived vector"
    );
}

#[test]
fn sync_word_bytes_match_the_well_known_ltc_sync_word() {
    // The last two bytes of every valid vector above are the fixed
    // synchronization word (§9.2.5, Table 5).
    assert_eq!(st12_1::SYNC_WORD, [0xFC, 0xBF]);
    for vector in [WORKED_VECTOR, MAX_VECTOR, ZERO_VECTOR] {
        assert_eq!([vector[8], vector[9]], st12_1::SYNC_WORD);
    }
}
