//! Regression test for issue #1042: a CEA-708 Caption Channel Packet (CCP)
//! spanning multiple `cc_data()` access units (video frames) must decode
//! identically to the same bytes delivered as one pre-assembled packet.
//!
//! No independent tool or real broadcast capture with a multi-frame-spanning
//! 708 CCP was available (this crate's own note on `CC-C1`: "the SEI/PES
//! fixtures are 608-only and synthetic"), so this reuses the CTA-708-E
//! `DefineWindow` worked example (`cc-data/docs/cea708-decode.md` p.66-67)
//! already verified byte-for-byte in
//! `cc_data::decode::cea708::tests::define_window_worked_example` -- the
//! same cited spec clause, split across several separate `push_triplets`
//! calls the way a real decoder receives one CCP across several video
//! frames' `cc_data()` (CEA-708 §4/§5: a CCP is not required to fit in one
//! access unit).
#![cfg(feature = "decode")]
use cc_data::decode::Cea708Decoder;
use cc_data::{CcTriplet, CcType};

/// One full, correctly-framed CCP (service 1, DefineWindow worked example):
/// `header(size_code=5) | service_block_header(svc=1,len=7) | 7 cmd bytes |
/// null service block header (terminator)` = 10 bytes = `size_code * 2`,
/// matching CEA-708 §5's packet-size accounting exactly (unlike the crate's
/// in-module test helper, which under-fills by one byte and relies on
/// `decode_packet`'s tolerant `min(data_size, rest.len())` slicing).
const FULL_CCP: [u8; 10] = [0x05, 0x27, 0x9A, 0x38, 0x4A, 0xD1, 0x8B, 0x0F, 0x11, 0x00];

fn triplet(cc_type: CcType, a: u8, b: u8) -> CcTriplet {
    CcTriplet {
        cc_valid: true,
        cc_type,
        cc_data_1: a,
        cc_data_2: b,
    }
}

/// Split `FULL_CCP` into 5 separate `push_triplets` calls (one byte-pair
/// each), each call standing in for one video frame's `cc_data()`.
fn push_as_separate_frames(dec: &mut Cea708Decoder) {
    dec.push_triplets(&[triplet(CcType::Dtvcc708Start, FULL_CCP[0], FULL_CCP[1])]);
    dec.push_triplets(&[triplet(CcType::Dtvcc708Data, FULL_CCP[2], FULL_CCP[3])]);
    dec.push_triplets(&[triplet(CcType::Dtvcc708Data, FULL_CCP[4], FULL_CCP[5])]);
    dec.push_triplets(&[triplet(CcType::Dtvcc708Data, FULL_CCP[6], FULL_CCP[7])]);
    dec.push_triplets(&[triplet(CcType::Dtvcc708Data, FULL_CCP[8], FULL_CCP[9])]);
}

#[test]
fn ccp_spanning_frames_decodes_same_as_preassembled() {
    let mut direct = Cea708Decoder::new();
    direct.push_packet(&FULL_CCP);
    let want = direct.windows(1)[2]
        .clone()
        .expect("window 2 defined (direct)");

    let mut framed = Cea708Decoder::new();
    push_as_separate_frames(&mut framed);
    let got = framed.windows(1)[2]
        .clone()
        .expect("window 2 defined (split across frames)");

    assert_eq!(
        got, want,
        "frame-split CCP must decode identically to the pre-assembled one"
    );
    // Pin the actual worked-example values too, not just self-consistency.
    assert_eq!(got.anchor_vertical, 74);
    assert_eq!(got.anchor_horizontal, 209);
    assert_eq!(got.row_count, 12);
    assert_eq!(got.column_count, 16);
}
