//! Spec vectors for the MPEG-2 display tracker, hand-built from
//! `docs/codec/mpeg2-video-display.md` / `afd-bar-data.md` §4.4 — never from
//! production code.

use super::*;
use alloc::vec::Vec;

fn bits(value: u64, width: usize) -> Vec<bool> {
    (0..width).rev().map(|i| (value >> i) & 1 == 1).collect()
}

fn pack(bits: &[bool]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in bits.chunks(8) {
        let mut b = 0u8;
        for (i, bit) in chunk.iter().enumerate() {
            if *bit {
                b |= 0x80 >> i;
            }
        }
        out.push(b);
    }
    out
}

/// `sequence_header()`: 720x576 (aspect 16:9 derives 64:45, aspect 4:3
/// derives 16:15), no extensions (H.262 §6.3.3: the coded size is used).
fn sequence(aspect: u64) -> Vec<u8> {
    let mut b = bits(720, 12);
    b.extend(bits(576, 12));
    b.extend(bits(aspect, 4));
    b.extend(bits(3, 4)); // frame_rate_code
    b.extend(bits(0x3FFFF, 18)); // bit_rate_value
    b.extend(bits(1, 1)); // marker_bit
    b.extend(bits(112, 10)); // vbv_buffer_size_value
    b.extend(bits(0, 1)); // constrained_parameters_flag
    b.extend(bits(0, 1)); // load_intra_quantiser_matrix
    b.extend(bits(0, 1)); // load_non_intra_quantiser_matrix
    let mut out = vec![0x00, 0x00, 0x01, 0xB3];
    out.extend_from_slice(&pack(&b));
    out
}

/// `picture_header()`: temporal_reference 0, coding type `pct`, a slice start
/// code so the picture is complete, and then the given user-data sections.
fn picture(pct: u64, user_data: &[Vec<u8>]) -> Vec<u8> {
    let mut b = bits(0, 10); // temporal_reference
    b.extend(bits(pct, 3)); // picture_coding_type
    b.extend(bits(0xFFFF, 16)); // vbv_delay
    let mut out = vec![0x00, 0x00, 0x01, 0x00];
    out.extend_from_slice(&pack(&b));
    for u in user_data {
        out.extend_from_slice(u);
    }
    out.extend_from_slice(&[0x00, 0x00, 0x01, 0x01, 0x00]);
    out
}

/// `user_data()` with a `DTG1` `afd_data()` (TS 101 154 Table B.2).
fn afd_user_data(afd: Option<u64>) -> Vec<u8> {
    let mut out = vec![0x00, 0x00, 0x01, 0xB2];
    out.extend_from_slice(&DTG1_USER_IDENTIFIER.to_be_bytes());
    match afd {
        Some(code) => {
            out.push(0x41); // '0' + active_format_flag 1 + reserved '000001'
            out.push(0xF0 | (code as u8 & 0x0F));
        }
        None => out.push(0x01), // '0' + active_format_flag 0
    }
    out
}

/// `user_data()` with a `GA94` `user_data_type_code` 0x06 `bar_data()`: a top
/// and a bottom bar (distinct values).
fn bar_user_data(top: u16, bottom: u16) -> Vec<u8> {
    let mut out = vec![0x00, 0x00, 0x01, 0xB2];
    out.extend_from_slice(&GA94_USER_IDENTIFIER.to_be_bytes());
    out.push(BAR_DATA_USER_DATA_TYPE_CODE);
    out.push(0xC0); // top + bottom flags, reserved '0000'
    let marked = |v: u16| [((v >> 8) as u8) | 0xC0, v as u8];
    out.extend_from_slice(&marked(top));
    out.extend_from_slice(&marked(bottom));
    out
}

fn user_data_only(payload: &[u8]) -> Vec<u8> {
    payload.to_vec()
}

#[test]
fn first_picture_raises_the_initial_event() {
    let mut t = Mpeg2DisplayTracker::new();
    let au = [sequence(3), picture(1, &[afd_user_data(Some(9))])].concat();
    let Mpeg2Outcome::Changed {
        display,
        initial,
        random_access,
        changed,
    } = t.observe(&au)
    else {
        panic!("first picture must raise the initial state");
    };
    assert!(initial && changed.initial);
    assert!(random_access, "a sequence header is a random-access point");
    assert_eq!(
        display.sample_aspect_ratio().map(|s| s.as_pair()),
        Some((64, 45))
    );
    assert_eq!(
        display.active_format.as_ref().map(|a| a.carriage),
        Some(UserDataCarriage::Mpeg2PictureUserData)
    );
    assert_eq!(
        display.active_format.as_ref().map(|a| a.afd),
        Some(Some(ActiveFormatDescription::Centre4x3))
    );
    assert!(t.state().is_some());
}

#[test]
fn repeated_afd_raises_nothing_and_persists() {
    let mut t = Mpeg2DisplayTracker::new();
    let seq = sequence(3);
    let first = [seq.clone(), picture(1, &[afd_user_data(Some(9))])].concat();
    assert!(matches!(t.observe(&first), Mpeg2Outcome::Changed { .. }));
    for pct in [2u64, 3] {
        let au = picture(pct, &[afd_user_data(Some(9))]);
        assert_eq!(t.observe(&au), Mpeg2Outcome::Unchanged);
    }
    // A picture with no user data keeps the AFD in force (A/53 §6.2.4).
    let au = picture(1, &[]);
    assert_eq!(t.observe(&au), Mpeg2Outcome::Unchanged);
    assert_eq!(
        t.state()
            .and_then(|s| s.active_format.clone())
            .map(|a| a.afd),
        Some(Some(ActiveFormatDescription::Centre4x3))
    );
}

#[test]
fn afd_change_mid_gop_is_not_random_access() {
    let mut t = Mpeg2DisplayTracker::new();
    let first = [sequence(3), picture(1, &[afd_user_data(Some(9))])].concat();
    t.observe(&first);
    let au = picture(2, &[afd_user_data(Some(10))]); // P picture
    let Mpeg2Outcome::Changed {
        random_access,
        changed,
        display,
        ..
    } = t.observe(&au)
    else {
        panic!("AFD change must raise");
    };
    assert!(!random_access, "a P picture is not a random-access point");
    assert!(changed.afd);
    assert!(!changed.initial);
    assert_eq!(
        display.active_format.as_ref().map(|a| a.afd),
        Some(Some(ActiveFormatDescription::Centre16x9))
    );
}

#[test]
fn sequence_header_without_afd_resets_to_none() {
    let mut t = Mpeg2DisplayTracker::new();
    let first = [sequence(3), picture(1, &[afd_user_data(Some(10))])].concat();
    t.observe(&first);
    let au = [sequence(3), picture(1, &[])].concat();
    let Mpeg2Outcome::Changed {
        changed, display, ..
    } = t.observe(&au)
    else {
        panic!("sequence_header() with no AFD must raise");
    };
    assert!(changed.afd);
    assert!(!changed.output_rect, "the geometry itself did not move");
    assert_eq!(
        display.active_format.as_ref().map(|a| a.afd),
        Some(None),
        "reverts to the coded frame, not SameAsCodedFrame"
    );
}

#[test]
fn new_afd_without_bar_data_clears_bar_data() {
    let mut t = Mpeg2DisplayTracker::new();
    let first = [
        sequence(3),
        picture(1, &[afd_user_data(Some(0)), bar_user_data(0x0100, 0x0200)]),
    ]
    .concat();
    let Mpeg2Outcome::Changed { display, .. } = t.observe(&first) else {
        panic!("first")
    };
    assert_eq!(
        display
            .active_format
            .as_ref()
            .and_then(|a| a.bar_data.clone()),
        Some(BarData {
            top_bar_end_line: Some(0x0100),
            bottom_bar_start_line: Some(0x0200),
            left_bar_end_pixel: None,
            right_bar_start_pixel: None,
        })
    );
    let au = picture(2, &[afd_user_data(Some(9))]);
    let Mpeg2Outcome::Changed {
        changed, display, ..
    } = t.observe(&au)
    else {
        panic!("AFD without bar data must raise")
    };
    assert!(changed.afd);
    assert!(changed.bar_data);
    assert_eq!(
        display
            .active_format
            .as_ref()
            .and_then(|a| a.bar_data.clone()),
        None
    );
}

#[test]
fn user_data_before_the_picture_header_is_not_picture_afd() {
    let mut t = Mpeg2DisplayTracker::new();
    // `extension_and_user_data(0)`: AFD right after the sequence header.
    let au = [
        sequence(3),
        user_data_only(&afd_user_data(Some(9))),
        picture(1, &[]),
    ]
    .concat();
    let Mpeg2Outcome::Changed { display, .. } = t.observe(&au) else {
        panic!("first")
    };
    assert_eq!(
        display.active_format.as_ref().map(|a| a.afd),
        None,
        "a sequence-level user data is not a picture's AFD"
    );
}

#[test]
fn short_and_unknown_user_data_is_ignored() {
    let mut t = Mpeg2DisplayTracker::new();
    let mut short = vec![0x00, 0x00, 0x01, 0xB2];
    short.extend_from_slice(&DTG1_USER_IDENTIFIER.to_be_bytes());
    short.push(0x41); // flag set, no code byte
    let mut other = vec![0x00, 0x00, 0x01, 0xB2];
    other.extend_from_slice(b"XXXX");
    other.push(0x41);
    let au = [sequence(3), picture(1, &[short, other])].concat();
    let Mpeg2Outcome::Changed { display, .. } = t.observe(&au) else {
        panic!("first")
    };
    assert_eq!(display.active_format.as_ref().map(|a| a.afd), None);
}

#[test]
fn active_format_flag_zero_clears_the_afd() {
    let mut t = Mpeg2DisplayTracker::new();
    let first = [sequence(3), picture(1, &[afd_user_data(Some(9))])].concat();
    t.observe(&first);
    let au = picture(2, &[afd_user_data(None)]);
    let Mpeg2Outcome::Changed {
        changed, display, ..
    } = t.observe(&au)
    else {
        panic!("flag clear must raise")
    };
    assert!(changed.afd);
    assert_eq!(display.active_format.as_ref().map(|a| a.afd), Some(None));
}

#[test]
fn sequence_geometry_change_is_reported_at_that_access_unit() {
    let mut t = Mpeg2DisplayTracker::new();
    let first = [sequence(3), picture(1, &[])].concat();
    t.observe(&first);
    let au = [sequence(2), picture(1, &[])].concat();
    let Mpeg2Outcome::Changed {
        changed, display, ..
    } = t.observe(&au)
    else {
        panic!("aspect change must raise")
    };
    assert!(changed.aspect_ratio);
    assert_eq!(
        display.sample_aspect_ratio().map(|s| s.as_pair()),
        Some((16, 15))
    );
}
