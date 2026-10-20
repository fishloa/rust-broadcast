//! Spec vectors for the MPEG-2 display tracker, hand-built from
//! `docs/codec/mpeg2-video-display.md` / `afd-bar-data.md` §4.4 — never from
//! production code.

use super::*;
use crate::ir::TrackId;
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

/// Pictures 0..=2 without AFD, 3..=5 with AFD 9, as a real MPEG-2 TS built
/// with the crate's own unit-test packet builders (`crate::ts_demux::tests`
/// helpers), then demuxed end to end.
fn mid_gop_stream() -> (Vec<u8>, TrackId) {
    use crate::ts_demux::tests::{
        ES_PID_UNDER_TEST, ES_STREAM_ID_VIDEO, PAT_PID_UNDER_TEST, PMT_PID_UNDER_TEST, null_packet,
        pat_body, pes_packet, pes_packets, pmt_body, psi_section_packets,
    };
    let coding = [1u64, 2, 2, 2, 2, 2];
    let with_afd = [false, false, false, true, true, true];
    let mut input = Vec::new();
    for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
        input.extend_from_slice(&null_packet());
    }
    input.extend_from_slice(&psi_section_packets(
        PAT_PID_UNDER_TEST,
        0,
        crate::ts_demux::TABLE_ID_PAT,
        &pat_body(),
    ));
    input.extend_from_slice(&psi_section_packets(
        PMT_PID_UNDER_TEST,
        0,
        crate::ts_demux::TABLE_ID_PMT,
        &pmt_body(ES_PID_UNDER_TEST, crate::ts_demux::STREAM_TYPE_MPEG2_VIDEO),
    ));
    let mut cc = 0u8;
    let sequence = sequence(3);
    for (i, (&pct, &afd)) in coding.iter().zip(&with_afd).enumerate() {
        let mut au = Vec::new();
        if i == 0 {
            au.extend_from_slice(&sequence);
        }
        au.extend_from_slice(&ts_picture(pct, afd.then_some(9)));
        let pts = i as i64 * 3600;
        let pes = pes_packet(ES_STREAM_ID_VIDEO, pts, Some(pts), &au);
        let packets = pes_packets(ES_PID_UNDER_TEST, cc, &pes);
        cc = cc.wrapping_add((packets.len() / mpeg_ts::ts::TS_PACKET_SIZE) as u8);
        input.extend_from_slice(&packets);
    }
    let media = crate::ts_demux::TsDemux::new()
        .demux(&input)
        .expect("hand-built TS demuxes");
    let track_id = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), crate::ir::CodecConfig::Mpeg2Video { .. }))
        .expect("an MPEG-2 track")
        .spec
        .track_id;
    (input, track_id)
}

/// Build `sequence_header()` at 720x576 with the given `aspect_ratio_information`.
fn sequence(aspect: u64) -> Vec<u8> {
    let mut b = bits(720, 12);
    b.extend(bits(576, 12));
    b.extend(bits(aspect, 4));
    b.extend(bits(3, 4));
    b.extend(bits(0x3FFFF, 18));
    b.extend(bits(1, 1));
    b.extend(bits(112, 10));
    b.extend(bits(0, 3));
    let mut out = vec![0x00, 0x00, 0x01, 0xB3];
    out.extend_from_slice(&pack(&b));
    out
}

/// `picture_header()` + slice, with an optional `DTG1` AFD after the header.
fn ts_picture(pct: u64, afd: Option<u64>) -> Vec<u8> {
    let mut b = bits(0, 10);
    b.extend(bits(pct, 3));
    b.extend(bits(0xFFFF, 16));
    let mut out = vec![0x00, 0x00, 0x01, 0x00];
    out.extend_from_slice(&pack(&b));
    if let Some(code) = afd {
        out.extend_from_slice(&afd_user_data(Some(code)));
    }
    out.extend_from_slice(&[0x00, 0x00, 0x01, 0x01, 0x00]);
    out
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

#[test]
fn mid_gop_afd_change_through_the_demuxer() {
    use crate::ir::CodecConfig;
    use crate::ts_demux::{DemuxEvent, StreamingTsDemux, TsDemux};

    let (ts, track_id) = mid_gop_stream();
    let media = TsDemux::new().demux(&ts).expect("batch");
    let track = media
        .tracks
        .iter()
        .find(|t| t.spec.track_id == track_id)
        .expect("track");
    assert_eq!(track.samples.len(), 6);
    let batch: Vec<_> = track
        .samples
        .iter()
        .map(|s| {
            s.display
                .as_deref()
                .and_then(|d| d.active_format.as_ref().map(|a| a.afd))
        })
        .collect();
    assert_eq!(
        batch[..3],
        [None, None, None],
        "pictures before any AFD carry no area of interest (default)"
    );
    assert_eq!(
        batch[3..],
        [Some(Some(ActiveFormatDescription::Centre4x3)); 3],
        "pictures 3.. carry AFD 9"
    );
    // The static geometry of the track came from the real access units.
    assert!(matches!(track.config(), CodecConfig::Mpeg2Video { .. }));
    if let Some(display) = track.spec.display.clone() {
        assert_eq!((display.coded_width, display.coded_height), (720, 576));
        assert_eq!(display.sample_aspect_ratio(), Some((64, 45)));
    }

    let mut demux = StreamingTsDemux::new();
    demux.feed(&ts);
    demux.finish();
    let mut events = Vec::new();
    while let Some(ev) = demux.poll_event() {
        match ev {
            DemuxEvent::Sample {
                track_id: id,
                sample,
                ..
            } if id == track_id => events.push((
                "sample",
                sample.dts,
                sample
                    .display
                    .as_deref()
                    .and_then(|d| d.active_format.as_ref().map(|a| a.afd)),
                sample.flags.is_sync,
            )),
            DemuxEvent::DisplayChanged {
                track_id: id,
                display,
                changed,
                dts,
                random_access,
                ..
            } if id == track_id => events.push((
                if changed.initial {
                    "initial"
                } else {
                    "changed"
                },
                dts,
                display.active_format.as_ref().map(|a| a.afd),
                random_access,
            )),
            _ => {}
        }
    }
    let changes: Vec<_> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| e.0 == "changed")
        .collect();
    assert_eq!(changes.len(), 1, "exactly the AFD introduction raises");
    let (at, event) = changes[0];
    let (_kind, dts, afd, random_access) = event;
    assert!(!random_access, "a P picture is not random access");
    assert_eq!(*afd, Some(Some(ActiveFormatDescription::Centre4x3)));
    let (prev_kind, prev_dts, prev_afd, _) = &events[at - 1];
    assert_eq!(*prev_kind, "sample", "the change follows a sample");
    assert_ne!(prev_dts, dts);
    assert!(
        matches!(prev_afd, None | Some(None)),
        "the previous picture carries no AFD"
    );
    let (next_kind, sample_dts, sample_afd, _) = &events[at + 1];
    assert_eq!(*next_kind, "sample", "the change precedes its sample");
    assert_eq!(sample_dts, dts, "with that sample's own DTS");
    assert_eq!(*sample_afd, *afd);
}
