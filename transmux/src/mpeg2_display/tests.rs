//! Spec vectors for the MPEG-2 display tracker, hand-built from
//! `private/specs-verified/mpeg/transmux/codec/mpeg2-video-display.md` / `afd-bar-data.md` §4.4 — never from
//! production code.

use super::*;
use crate::TrackId;
use alloc::vec::Vec;
use broadcast_bitstream::h262::{
    EXTENSION_START_CODE_BYTE, FIRST_RESERVED_ASPECT, SEQUENCE_DISPLAY_EXTENSION_ID,
};

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
    let media = crate::collect::media_from_ts(&input).expect("hand-built TS demuxes");
    let track_id = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), crate::ir::CodecConfig::Mpeg2Video { .. }))
        .expect("an MPEG-2 track")
        .spec
        .track_id;
    (input, track_id)
}

/// Build `sequence_header()` at `width`x`height` with the given
/// `aspect_ratio_information`.
fn sequence_at(width: u64, height: u64, aspect: u64) -> Vec<u8> {
    let mut b = bits(width, 12);
    b.extend(bits(height, 12));
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

/// Build `sequence_header()` at 720x576 with the given `aspect_ratio_information`.
fn sequence(aspect: u64) -> Vec<u8> {
    sequence_at(720, 576, aspect)
}

/// `sequence_display_extension()`: `video_format`, the optional colour
/// description triple, and the intended display rectangle.
fn display_extension(
    video_format: u64,
    colour: Option<(u64, u64, u64)>,
    width: u64,
    height: u64,
) -> Vec<u8> {
    let mut b = bits(u64::from(SEQUENCE_DISPLAY_EXTENSION_ID), 4);
    b.extend(bits(video_format, 3));
    b.extend(bits(u64::from(colour.is_some()), 1));
    if let Some((p, t, m)) = colour {
        b.extend(bits(p, 8));
        b.extend(bits(t, 8));
        b.extend(bits(m, 8));
    }
    b.extend(bits(width, 14));
    b.extend(bits(1, 1));
    b.extend(bits(height, 14));
    let mut out = vec![0x00, 0x00, 0x01, EXTENSION_START_CODE_BYTE];
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
        Some(BarData::new(Some(0x0100), Some(0x0200), None, None))
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
    use crate::ts_demux::{DemuxEvent, StreamingTsDemux};

    let (ts, track_id) = mid_gop_stream();
    let media = crate::collect::media_from_ts(&ts).expect("batch");
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
    demux.feed(&ts).expect("valid timing");
    demux.finish().expect("valid timing");
    let mut events = Vec::new();
    while let Some(ev) = demux.poll_event() {
        match ev {
            DemuxEvent::Sample {
                track_id: id,
                sample,
                ..
            } if id == track_id => events.push((
                "sample",
                sample.timing.and_then(|t| t.dts()),
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

/// The geometry `sequence_header()` resolves to, via the same parser the
/// tracker uses.
fn geometry_of(au: &[u8]) -> Option<DisplayGeometry> {
    let start = find_sequence_header(au)?;
    Some(DisplayGeometry::from_mpeg2_sequence(
        &parse_sequence_at(au, start).ok()?,
    ))
}

/// #3795: `seed_state` is the level-triggered start — an access unit that
/// resolves to the seeded state emits nothing, rather than a spurious
/// `initial` `Changed`. A no-op seed would emit one.
#[test]
fn seeded_state_suppresses_the_first_access_unit() {
    let au = [sequence(3), picture(1, &[])].concat();
    let geometry = geometry_of(&sequence(3));
    assert!(
        geometry.is_some(),
        "the sequence header resolves a geometry"
    );

    let mut seeded = Mpeg2DisplayTracker::new();
    seeded.seed_state(geometry.clone());
    assert!(seeded.state().is_some(), "seed_state fills the state");
    assert_eq!(seeded.observe(&au), Mpeg2Outcome::Unchanged);

    let mut full = Mpeg2DisplayTracker::new();
    let display = Arc::new(DisplayInfo::from_geometry(geometry).expect("display"));
    full.seed_full(Some(display));
    assert!(full.state().is_some(), "seed_full fills the state");
    assert_eq!(full.observe(&au), Mpeg2Outcome::Unchanged);
}

/// #3795: a second seed is ignored — the first state in force is kept, so a
/// later `TrackAdded` cannot move the state under the samples.
#[test]
fn a_second_seed_is_ignored() {
    let geometry = geometry_of(&sequence(3));

    let mut t = Mpeg2DisplayTracker::new();
    t.seed_state(geometry.clone());
    let first = Arc::clone(t.state().expect("seeded"));
    t.seed_state(geometry_of(&sequence(2)));
    assert!(
        Arc::ptr_eq(&first, t.state().expect("kept")),
        "seed_state keeps the first state"
    );

    let mut t = Mpeg2DisplayTracker::new();
    t.seed_full(Some(Arc::new(
        DisplayInfo::from_geometry(geometry).expect("display"),
    )));
    let first = Arc::clone(t.state().expect("seeded"));
    t.seed_full(Some(Arc::new(DisplayInfo::new(None))));
    assert!(
        Arc::ptr_eq(&first, t.state().expect("kept")),
        "seed_full keeps the first state"
    );
}

/// #3796: a picture user-data payload shorter than the 4-byte identifier is
/// ignored, not indexed `[0..3]` on the wire.
#[test]
fn user_data_shorter_than_the_identifier_is_ignored() {
    let mut t = Mpeg2DisplayTracker::new();
    let two_byte = vec![0x00, 0x00, 0x01, 0xB2, 0x44, 0x54];
    let empty = vec![0x00, 0x00, 0x01, 0xB2];
    let au = [sequence(3), picture(1, &[two_byte, empty])].concat();
    let Mpeg2Outcome::Changed { display, .. } = t.observe(&au) else {
        panic!("first")
    };
    assert_eq!(
        display.active_format.as_ref().map(|a| a.afd),
        None,
        "a 2-byte and an empty payload carry no identifier"
    );
}

/// The sequence geometry an elementary-stream prefix resolves to is the coded
/// size, with the Table 6-3 aspect as an extended SAR and no crop rectangle.
#[test]
fn geometry_from_a_sequence_header_is_the_coded_picture() {
    let g = DisplayGeometry::from_mpeg2_es(&sequence(3)).expect("geometry");
    assert_eq!((g.coded_width, g.coded_height), (720, 576));
    assert_eq!(g.output_rect.width(), 720, "no crop syntax in H.262");
    assert_eq!(g.output_rect.height(), 576);
    assert_eq!(g.sample_aspect_ratio(), Some((64, 45)));
    assert!(g.is_mpeg2(), "MPEG-2 inferred colour is in force");
}

#[test]
fn square_samples_are_square() {
    let g = DisplayGeometry::from_mpeg2_es(&sequence_at(320, 240, 1)).expect("geometry");
    assert_eq!(g.aspect_ratio.map(|a| a.idc()), Some(1));
    assert_eq!(g.sample_aspect_ratio(), Some((1, 1)));
}

#[test]
fn forbidden_and_reserved_aspects_are_unspecified() {
    for aspect in core::iter::once(0u64).chain(u64::from(FIRST_RESERVED_ASPECT)..16) {
        let g = DisplayGeometry::from_mpeg2_es(&sequence(aspect)).expect("geometry");
        assert_eq!(g.aspect_ratio, None, "aspect {aspect}");
        assert_eq!(g.sample_aspect_ratio(), None);
    }
}

#[test]
fn display_extension_colour_is_signalled_and_inferred_is_bt709() {
    let plain = DisplayGeometry::from_mpeg2_es(&sequence(2)).expect("geometry");
    assert_eq!(plain.video_signal, None);
    assert!(plain.is_mpeg2());
    assert_eq!(
        plain.colour_description(),
        ColourDescription::from_wire(1, 1, 1)
    );
    let signalled = DisplayGeometry::from_mpeg2_es(
        &[sequence(2), display_extension(1, Some((6, 6, 5)), 704, 480)].concat(),
    )
    .expect("geometry");
    assert_eq!(
        signalled.colour_description(),
        ColourDescription::from_wire(6, 6, 5)
    );
    let format_only = DisplayGeometry::from_mpeg2_es(
        &[sequence(2), display_extension(2, None, 704, 480)].concat(),
    )
    .expect("geometry");
    assert_eq!(
        format_only.video_format(),
        VideoFormat::Ntsc,
        "video_format 2 is NTSC (Table 6-6)"
    );
    assert_eq!(
        format_only.colour_description(),
        ColourDescription::from_wire(1, 1, 1),
        "colour_description 0 infers BT.709"
    );
    assert!(!format_only.full_range(), "H.262 has no full-range flag");
}

#[test]
fn no_sequence_header_is_an_error() {
    assert!(matches!(
        DisplayGeometry::from_mpeg2_es(&[0x00, 0x00, 0x01, 0x00, 0x08]),
        Err(crate::error::Error::InvalidInput(..))
    ));
}

#[test]
fn codec_config_without_sequence_header_yields_none() {
    use isobmff::mp4esds::{
        DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, SLConfigDescriptor,
    };
    let esds = EsdsBox::new(ESDescriptor::new(
        0,
        0,
        Some(DecoderConfigDescriptor::new(
            crate::ts_demux::OTI_MPEG2_VIDEO_MAIN,
            crate::ts_demux::STREAM_TYPE_VISUAL,
            false,
            0,
            0,
            0,
            Some(DecoderSpecificInfo::new(vec![0x55])),
        )),
        Some(SLConfigDescriptor::predefined_two()),
    ));
    let config = crate::ir::CodecConfig::Mpeg2Video {
        esds,
        width: 704,
        height: 480,
    };
    assert_eq!(DisplayGeometry::from_codec_config(&config), None);
}

#[test]
fn codec_config_sequence_header_gives_geometry() {
    use isobmff::mp4esds::{
        DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, SLConfigDescriptor,
    };
    let es = sequence(3);
    let esds = EsdsBox::new(ESDescriptor::new(
        0,
        0,
        Some(DecoderConfigDescriptor::new(
            crate::ts_demux::OTI_MPEG2_VIDEO_MAIN,
            crate::ts_demux::STREAM_TYPE_VISUAL,
            false,
            0,
            0,
            0,
            Some(DecoderSpecificInfo::new(es)),
        )),
        Some(SLConfigDescriptor::predefined_two()),
    ));
    let config = crate::ir::CodecConfig::Mpeg2Video {
        esds,
        width: 704,
        height: 576,
    };
    let g = DisplayGeometry::from_codec_config(&config).expect("esds sequence header");
    assert!(g.is_mpeg2());
    assert_eq!(g.sample_aspect_ratio(), Some((64, 45)));
}
