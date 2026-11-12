//! Encrypted-fragment and track-id probes (`transmux::smooth_fragment`) on real and
//! hand-damaged fragments.

use transmux::smooth_fragment::{
    FragmentState, PIFF_SAMPLE_ENCRYPTION_UUID, fragment_state, fragment_track_id,
    manifest_declares_protection,
};

fn fragment_looks_encrypted(bytes: &[u8]) -> bool {
    fragment_state(bytes) != FragmentState::Clear
}

#[test]
fn manifest_declares_protection_matches_tag_boundary_not_substring() {
    assert!(manifest_declares_protection(
        "<SmoothStreamingMedia><Protection></Protection></SmoothStreamingMedia>"
    ));
    assert!(manifest_declares_protection(
        "<SmoothStreamingMedia><Protection/></SmoothStreamingMedia>"
    ));
    assert!(!manifest_declares_protection(
        "<SmoothStreamingMedia><ProtectionFoo/></SmoothStreamingMedia>"
    ));
    assert!(!manifest_declares_protection(
        "<SmoothStreamingMedia></SmoothStreamingMedia>"
    ));
    // A look-alike in a foreign namespace, a comment, or an attribute value
    // that merely mentions the tag does not count.
    assert!(!manifest_declares_protection(
        "<SmoothStreamingMedia xmlns:ms=\"urn:x\"><ms:Protection/></SmoothStreamingMedia>"
    ));
    assert!(!manifest_declares_protection(
        "<SmoothStreamingMedia><Protection xmlns=\"urn:x\"/></SmoothStreamingMedia>"
    ));
    assert!(!manifest_declares_protection(
        "<SmoothStreamingMedia><!-- <Protection> --></SmoothStreamingMedia>"
    ));
    assert!(!manifest_declares_protection(
        "<SmoothStreamingMedia note=\"<Protection>\"/>"
    ));
}

/// Serialize one ISOBMFF box (`type` + `body`) using the crate's own
/// header writer, so the fixtures below are real, parseable boxes.
fn box_bytes(box_type: [u8; 4], body: &[u8]) -> Vec<u8> {
    use broadcast_common::Serialize;
    let header = transmux::box_types::BoxHeader::new(
        (transmux::box_types::BOX_HEADER_MIN_SIZE + body.len()) as u64,
        transmux::box_types::BoxType::from_bytes(box_type),
        None,
    );
    let mut out = vec![0u8; header.serialized_len() + body.len()];
    let n = header.serialize_into(&mut out).expect("serialize header");
    out[n..].copy_from_slice(body);
    out
}

/// Wrap `child` in a `traf`, then that in a `moof`, then that in a
/// leading `styp` — the MS-SSTR fragment shape.
fn fragment_with_traf_child(child: Vec<u8>) -> Vec<u8> {
    let traf = box_bytes(*b"traf", &child);
    let moof = box_bytes(*b"moof", &traf);
    let styp = box_bytes(*b"styp", b"msdh");
    let mut frag = styp;
    frag.extend_from_slice(&moof);
    frag
}

#[test]
fn fragment_looks_encrypted_walks_the_moof_tree() {
    // A `senc` inside `moof`/`traf` is encryption.
    assert!(fragment_looks_encrypted(&fragment_with_traf_child(
        box_bytes(*b"senc", &[0u8; 8])
    )));
    assert!(fragment_looks_encrypted(&fragment_with_traf_child(
        box_bytes(*b"saiz", &[0u8; 4])
    )));
    assert!(fragment_looks_encrypted(&fragment_with_traf_child(
        box_bytes(*b"saio", &[0u8; 4])
    )));

    // The bytes `senc`/`saiz`/`saio` appearing in `mdat` payload (or any
    // other box body) are **not** encryption — the pre-fix byte scan
    // false-positived here (audit run 7, W15).
    let moof = box_bytes(
        *b"moof",
        &box_bytes(*b"traf", &box_bytes(*b"tfhd", &[0u8; 8])),
    );
    let mut frag = box_bytes(*b"styp", b"msdh");
    frag.extend_from_slice(&moof);
    frag.extend_from_slice(&box_bytes(*b"mdat", b"...senc...saiz...saio..."));
    assert!(
        !fragment_looks_encrypted(&frag),
        "payload bytes that spell senc/saiz/saio are not encryption boxes"
    );

    // A bare `senc` with no enclosing `moof`/`traf` is not a fragment's
    // encryption signalling.
    assert!(!fragment_looks_encrypted(b"....senc...."));
    assert!(!fragment_looks_encrypted(&PIFF_SAMPLE_ENCRYPTION_UUID));
    // Raw bytes that are not a valid box sequence must fail closed (they
    // read as a malformed top-level box, not a clear fragment).
    assert_eq!(
        fragment_state(b"stypmoofmdattraftfhdtrun"),
        FragmentState::Undetermined
    );

    // The PIFF sample-encryption `uuid` **inside** a `traf` is encryption
    // (the positive case the box walk must still catch).
    let mut uuid_body = PIFF_SAMPLE_ENCRYPTION_UUID.to_vec();
    uuid_body.extend_from_slice(&[0u8; 4]); // FullBox header
    // Build the uuid box with the extended type.
    use broadcast_common::Serialize;
    let hdr = transmux::box_types::BoxHeader::new(
        (transmux::box_types::BOX_HEADER_MIN_SIZE + 16 + uuid_body.len()) as u64,
        transmux::box_types::BoxType::from_bytes(*b"uuid"),
        Some(PIFF_SAMPLE_ENCRYPTION_UUID),
    );
    let mut uuid_box = vec![0u8; hdr.serialized_len() + uuid_body.len()];
    let n = hdr.serialize_into(&mut uuid_box).expect("serialize uuid");
    uuid_box[n..].copy_from_slice(&uuid_body);
    assert!(
        fragment_looks_encrypted(&fragment_with_traf_child(uuid_box)),
        "a PIFF uuid inside traf is encryption"
    );
}

/// Audit W15: a **real** PIFF-CBC sample-encrypted fragment (Bento4
/// `mp4encrypt --method PIFF-CBC`, committed as `fixtures/mp4/piff-sample-encrypted.mp4`) must
/// be detected as encrypted by the box walk — the positive case, read
/// from a genuine encrypted fragment rather than a hand-built box.
#[test]
fn real_piff_encrypted_fixture_is_detected() {
    let bytes = test_fixtures::read("mp4/piff-sample-encrypted.mp4");
    assert!(
        fragment_looks_encrypted(&bytes),
        "a real PIFF-CBC encrypted fragment must be detected as encrypted"
    );
}

/// Audit W15: a fragment whose box walk fails *after* a `moof` was seen
/// (a truncated/malformed structure) must be treated as **not reliably
/// clear** — fail closed, so a truncated encrypted fragment is never
/// silently demuxed as clear.
#[test]
fn truncated_moof_fails_closed() {
    // A `moof` header claims a body larger than the buffer supplies.
    let mut frag = box_bytes(*b"styp", b"msdh");
    // moof with a declared size far past the end.
    frag.extend_from_slice(&8u32.to_be_bytes());
    frag.extend_from_slice(b"moof");
    // No body — the moof parse will fail.
    frag.extend_from_slice(&[0u8; 4]);
    assert_eq!(
        fragment_state(&frag),
        FragmentState::Undetermined,
        "a truncated moof must be undetermined, not clear"
    );

    // A non-fragment (no moof at all) is clear.
    assert_eq!(
        fragment_state(&box_bytes(*b"styp", b"msdh")),
        FragmentState::Clear
    );

    // A `moof` whose own header is truncated so badly its size can no
    // longer be read (fewer than 8 bytes for the header) is also
    // undetermined, not clear.
    let mut header_truncated = box_bytes(*b"styp", b"msdh");
    header_truncated.extend_from_slice(&[0x00, 0x00, 0x00, 0x40]); // size past end
    header_truncated.extend_from_slice(b"moof");
    assert_eq!(
        fragment_state(&header_truncated),
        FragmentState::Undetermined,
        "a top-level moof truncated at its header must fail closed"
    );

    // A genuinely encrypted fragment is classified distinct from
    // undetermined.
    assert_eq!(
        fragment_state(&fragment_with_traf_child(box_bytes(*b"senc", &[0u8; 8]))),
        FragmentState::Encrypted
    );
}

#[test]
fn discover_moof_track_id_recovers_real_fixture_track_ids() {
    for rel in [
        "smooth/av/QualityLevels(64528)/Fragments(video=0)",
        "smooth/av/QualityLevels(98429)/Fragments(audio=0)",
    ] {
        let fragment = test_fixtures::read(rel);
        assert_eq!(fragment_track_id(&fragment).unwrap(), 1, "{rel}");
    }
}

#[test]
fn discover_moof_track_id_errors_not_panics_on_garbage() {
    assert!(matches!(
        fragment_track_id(b"not a fragment at all"),
        Err(transmux::error::Error::BufferTooShort(..))
    ));
    assert!(matches!(
        fragment_track_id(&[]),
        Err(transmux::error::Error::UnexpectedBox { .. })
    ));
}
