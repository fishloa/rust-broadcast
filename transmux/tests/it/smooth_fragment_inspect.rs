//! `fragment_state`, `fragment_track_id` and `fragment_tfxd` (MS-SSTR 2.2.4) over
//! a fragment `SmoothPackager::package_track_fragment` builds from the real
//! `ts/h264-aac.ts` fixture, plus hand-built `moof` structures for the encrypted
//! and malformed classifications, plus `package_track_fragment`'s own contract.

use broadcast_common::Unpackage;
use broadcast_common::error::BufferTooShort;
use transmux::Error;
use transmux::media::{Media, Track};
use transmux::smooth::SmoothPackager;
use transmux::smooth_fragment::{FragmentState, fragment_state, fragment_tfxd, fragment_track_id};
use transmux::ts_demux::TsDemux;

fn media() -> Media {
    let ts = test_fixtures::read("ts/h264-aac.ts");
    TsDemux::new()
        .unpackage(&ts[..])
        .expect("demux h264-aac.ts")
}

fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

fn moof_with_traf_child(kind: &[u8; 4]) -> Vec<u8> {
    let traf = bx(b"traf", &bx(kind, &[0; 8]));
    [bx(b"moof", &traf), bx(b"mdat", &[0xAA; 16])].concat()
}

#[test]
fn a_packaged_fragment_reports_its_track_id_tfxd_and_clear_state() {
    let media = media();
    for track in &media.tracks {
        let start = 12_345_678_000;
        let fragment = SmoothPackager::default()
            .package_track_fragment(track, 7, start)
            .expect("package one fragment");
        assert_eq!(fragment.track_id, track.spec.track_id);
        assert_eq!(fragment.sequence_number, 7);
        assert_eq!(fragment.start_time, start);
        assert!(fragment.duration > 0);

        assert_eq!(
            fragment_track_id(&fragment.data).unwrap(),
            track.spec.track_id
        );
        let tfxd = fragment_tfxd(&fragment.data).expect("tfxd in the fragment");
        assert_eq!(tfxd.fragment_absolute_time, start);
        assert_eq!(tfxd.fragment_duration, fragment.duration);
        assert_eq!(fragment_state(&fragment.data), FragmentState::Clear);
        assert_eq!(
            fragment_state(&fragment.data[..fragment.data.len() - 3]),
            FragmentState::Undetermined,
            "a truncated mdat after a moof cannot be proven clear"
        );
    }
}

#[test]
fn package_track_fragment_duration_is_the_sum_of_the_sample_durations() {
    let media = media();
    let packager = SmoothPackager::default();
    let track: &Track = &media.tracks[0];
    let whole = packager.package_track_fragment(track, 1, 0).unwrap();
    let mut half = track.clone();
    half.samples.truncate(track.samples.len() / 2);
    let half = packager.package_track_fragment(&half, 1, 0).unwrap();
    assert!(half.duration > 0 && half.duration < whole.duration);
    let mut empty = track.clone();
    empty.samples.clear();
    assert!(matches!(
        packager.package_track_fragment(&empty, 1, 0),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn encryption_boxes_inside_a_traf_classify_the_fragment_as_encrypted() {
    for kind in [b"senc", b"saiz", b"saio"] {
        assert_eq!(
            fragment_state(&moof_with_traf_child(kind)),
            FragmentState::Encrypted,
            "{}",
            String::from_utf8_lossy(kind)
        );
    }
    assert_eq!(
        fragment_state(&moof_with_traf_child(b"tfhd")),
        FragmentState::Clear
    );
}

#[test]
fn a_four_byte_run_in_mdat_does_not_trip_the_encryption_scan() {
    let fragment = [
        moof_with_traf_child(b"tfhd"),
        bx(b"mdat", b"....senc....saiz...."),
    ]
    .concat();
    assert_eq!(fragment_state(&fragment), FragmentState::Clear);
}

#[test]
fn malformed_structures_fail_closed_after_a_moof_and_open_before_one() {
    let good = moof_with_traf_child(b"tfhd");
    let mut bad_size = good.clone();
    let moof_len = u32::from_be_bytes(good[..4].try_into().unwrap());
    bad_size.extend_from_slice(&[0, 0, 0, 3, b'f', b'r', b'e', b'e']);
    assert_eq!(fragment_state(&bad_size), FragmentState::Undetermined);
    assert_eq!(
        moof_len as usize + 24,
        good.len(),
        "moof then a 16-byte mdat box"
    );

    let truncated_moof = &good[..moof_len as usize - 2];
    assert_eq!(fragment_state(truncated_moof), FragmentState::Undetermined);

    let before_any_moof = [0, 0, 0, 3, b'f', b'r', b'e', b'e'];
    assert_eq!(fragment_state(&before_any_moof), FragmentState::Clear);
    assert_eq!(fragment_state(&[]), FragmentState::Clear);

    let broken_child = bx(b"moof", &bx(b"traf", &[0, 0, 0, 1, b's', b'e', b'n', b'c']));
    assert_eq!(fragment_state(&broken_child), FragmentState::Undetermined);
}

#[test]
fn fragment_track_id_reports_a_missing_moof_or_traf() {
    assert!(matches!(
        fragment_track_id(&bx(b"mdat", &[0; 8])),
        Err(Error::UnexpectedBox { expected: "moof" })
    ));
    assert!(matches!(
        fragment_track_id(&[]),
        Err(Error::UnexpectedBox { expected: "moof" })
    ));
    let got = fragment_track_id(&bx(b"moof", &bx(b"mfhd", &[0, 0, 0, 0, 0, 0, 0, 1])));
    assert!(
        matches!(
            got,
            Err(Error::BufferTooShort(BufferTooShort {
                need: 1,
                have: 0,
                what: "moof missing traf"
            }))
        ),
        "{got:?}"
    );
}

#[test]
fn fragment_track_id_hands_back_the_typed_track_id() {
    // Issue #2647: the return is a `TrackId`, so no raw integer escapes the
    // typed-id boundary; the wire `tfhd@track_ID` is 3 here on purpose (not 1)
    // so a hard-coded default cannot pass.
    const TFHD_FULL_BOX: u8 = 0x00;
    const TFHD_FLAGS: [u8; 3] = [0x00, 0x00, 0x00];
    const TRACK_ID: u32 = 3;
    let mut tfhd_body = vec![TFHD_FULL_BOX];
    tfhd_body.extend_from_slice(&TFHD_FLAGS);
    tfhd_body.extend_from_slice(&TRACK_ID.to_be_bytes());
    let moof = [
        bx(b"mfhd", &[0, 0, 0, 0, 0, 0, 0, 1]),
        bx(b"traf", &bx(b"tfhd", &tfhd_body)),
    ]
    .concat();
    let fragment = bx(b"moof", &moof);
    let got: transmux::TrackId =
        fragment_track_id(&fragment).expect("a well-formed moof/traf/tfhd carries its id");
    assert_eq!(got, transmux::TrackId::new(TRACK_ID));
}

#[test]
fn fragment_tfxd_is_none_without_a_tfxd_or_on_unwalkable_input() {
    assert!(fragment_tfxd(&[]).is_none());
    assert!(fragment_tfxd(&moof_with_traf_child(b"tfhd")).is_none());
    assert!(fragment_tfxd(&[0, 0, 0, 3, b'm', b'o', b'o', b'f']).is_none());
}
