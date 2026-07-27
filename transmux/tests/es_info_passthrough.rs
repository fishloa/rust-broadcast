//! `TsMux` ES_info descriptor passthrough policy (issue #775).
//!
//! Root cause: `plan_elementary_streams` only carried a track's inherited PMT
//! `ES_info` descriptor-loop bytes into the re-muxed PMT for an opaque
//! [`CodecConfig::Data`] track (issue #576's fix). Since `TsDemux` now
//! populates `TrackSpec::es_info_descriptors` for every recognised codec too
//! (issue #582), that guard silently dropped e.g. a re-muxed audio track's
//! `ISO_639_language_descriptor` and a subtitle track's
//! `subtitling_descriptor` — nullifying a downstream player's track picker.
//!
//! The fix is a **deny-list**: pass every inherited descriptor through except
//! `CA_descriptor` (tag `0x09`, ISO/IEC 13818-1 §2.6.16) — copying it forward
//! would falsely advertise this cleartext-output muxer's stream as scrambled,
//! pointing at a `CA_PID` that does not exist in the new PMT — de-duplicated
//! against descriptors this muxer synthesises itself (e.g. the MPEG-H
//! `MPEG-H_3dAudio_descriptor`, covered separately by `tests/mpegh_ts.rs`
//! against a real fixture).
//!
//! Fixture: `fixtures/ts/france2.ts` — a real DVB capture, one PMT (PID
//! `0x6E`), H.264 video + 3 E-AC-3 audio tracks (`fre`/`qad`/`qaa` ISO-639
//! language) + 2 DVB-subtitled tracks (`fra`). See
//! `fixtures/ts/france2-PROVENANCE.md`. Tracks 1/2 below deliberately collect
//! descriptors across the whole re-muxed `Media` rather than trusting a
//! specific track's re-assigned PID/position, so the assertion isn't
//! incidentally tied to `plan_elementary_streams`'s PID/ordering behaviour —
//! only to whether the descriptor bytes themselves survive somewhere. This
//! test's descriptor decode is a from-scratch tag/length/body walk,
//! independent of any crate-internal descriptor parser — `transmux` itself
//! never parses ES_info bytes, only carries them verbatim (see
//! `TrackSpec::es_info_descriptors`'s doc).

use std::collections::BTreeSet;

use broadcast_common::{Package, Unpackage};
use transmux::{Media, Track, TsDemux, TsMux};

const DESC_TAG_ISO_639_LANGUAGE: u8 = 0x0A;
const DESC_TAG_DVB_SUBTITLING: u8 = 0x59;
const DESC_TAG_CA: u8 = 0x09;

const EXPECTED_AUDIO_PIDS: [u16; 3] = [0x82, 0x83, 0x84];

fn fixture_bytes() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/france2.ts");
    std::fs::read(path).expect("fixtures/ts/france2.ts must exist")
}

fn demux_fixture() -> Media {
    TsDemux::new()
        .unpackage(&fixture_bytes())
        .expect("demux france2.ts")
}

/// Walk a descriptor loop's tag/length TLVs (ISO/IEC 13818-1 §2.6,
/// `descriptor()`); return the first body matching `tag`.
fn find_descriptor(desc_loop: &[u8], tag: u8) -> Option<&[u8]> {
    let mut i = 0;
    while i + 2 <= desc_loop.len() {
        let t = desc_loop[i];
        let len = desc_loop[i + 1] as usize;
        let start = i + 2;
        let end = (start + len).min(desc_loop.len());
        if t == tag {
            return Some(&desc_loop[start..end]);
        }
        i = end;
    }
    None
}

/// Every descriptor tag present in `desc_loop`, in order (walks the whole
/// loop, not just the first match of anything).
fn descriptor_tags(desc_loop: &[u8]) -> Vec<u8> {
    let mut i = 0;
    let mut out = Vec::new();
    while i + 2 <= desc_loop.len() {
        let t = desc_loop[i];
        let len = desc_loop[i + 1] as usize;
        out.push(t);
        i += 2 + len;
    }
    out
}

fn track_for_pid(media: &Media, pid: u16) -> &Track {
    media
        .tracks
        .iter()
        .find(|t| t.spec.source_pid == Some(pid))
        .unwrap_or_else(|| panic!("no track for PID {pid:#06x}"))
}

fn remux_via_ts(media: &Media) -> Media {
    let bytes = TsMux::new().package(media).expect("remux IR back to TS");
    TsDemux::new()
        .unpackage(&bytes)
        .expect("re-demux the remuxed TS")
}

/// ISO-639 language codes recovered from every track's ES_info loop,
/// independent of which PID/position each track ends up at.
fn iso639_languages(media: &Media) -> BTreeSet<String> {
    media
        .tracks
        .iter()
        .filter_map(|t| find_descriptor(&t.spec.es_info_descriptors, DESC_TAG_ISO_639_LANGUAGE))
        .map(|d| {
            assert!(
                d.len() >= 3,
                "ISO_639_language_descriptor body must carry at least a 3-byte language code"
            );
            std::str::from_utf8(&d[..3])
                .expect("language code must be ASCII")
                .to_string()
        })
        .collect()
}

/// DVB `subtitling_descriptor` bodies recovered from every track's ES_info
/// loop, independent of which PID/position each track ends up at.
fn subtitling_descriptors(media: &Media) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = media
        .tracks
        .iter()
        .filter_map(|t| find_descriptor(&t.spec.es_info_descriptors, DESC_TAG_DVB_SUBTITLING))
        .map(|d| d.to_vec())
        .collect();
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Test 1 (headline) — the 3 audio tracks' ISO-639 languages survive a re-mux.
// ---------------------------------------------------------------------------

#[test]
fn remux_preserves_audio_language_descriptors_for_recognised_codec() {
    let media = demux_fixture();

    let expected_languages: BTreeSet<String> = ["fre", "qad", "qaa"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    // Sanity: the source tracks really do carry these (fails loudly, not
    // silently, if the fixture ever drifts).
    assert_eq!(
        iso639_languages(&media),
        expected_languages,
        "sanity: fixture must carry ISO-639 fre/qad/qaa on its 3 audio tracks"
    );

    // The re-mux: TsDemux -> TsMux -> TsDemux.
    let remuxed = remux_via_ts(&media);

    assert_eq!(
        iso639_languages(&remuxed),
        expected_languages,
        "the 3 audio tracks' ISO-639 languages must survive a TS re-mux (issue #775) — \
         empty before the fix, since a recognised codec's ES_info was dropped"
    );
}

// ---------------------------------------------------------------------------
// Test 2 — the DVB subtitling descriptor (language + type) also survives.
// ---------------------------------------------------------------------------

#[test]
fn remux_preserves_dvb_subtitling_descriptor() {
    let media = demux_fixture();
    let source_subs = subtitling_descriptors(&media);
    assert_eq!(
        source_subs.len(),
        2,
        "sanity: fixture must carry exactly 2 DVB-subtitled tracks"
    );

    let remuxed = remux_via_ts(&media);
    let remuxed_subs = subtitling_descriptors(&remuxed);

    assert_eq!(
        remuxed_subs, source_subs,
        "subtitling_descriptor bytes (language + subtitling_type + page ids) must survive \
         a re-mux byte-for-byte, for both subtitle tracks (issue #775)"
    );
    for d in &remuxed_subs {
        assert!(
            d.len() >= 3,
            "subtitling_descriptor body too short to carry a language code"
        );
        let lang = std::str::from_utf8(&d[..3]).expect("language code must be ASCII");
        assert_eq!(lang, "fra", "subtitle language");
    }
}

// ---------------------------------------------------------------------------
// Test 3 — CA_descriptor denial on a real (mutated) track: dropped, siblings
// survive, source order preserved.
// ---------------------------------------------------------------------------

#[test]
fn remux_denies_ca_descriptor_but_keeps_sibling_descriptors() {
    let media = demux_fixture();
    let mut audio_track = track_for_pid(&media, EXPECTED_AUDIO_PIDS[0]).clone();

    // Confirm the real fixture track carries no CA_descriptor of its own,
    // then inject one between two real sibling descriptors (proving the
    // deny-list excises it out of the middle of the loop, not just off the
    // end) — the survivors' source order must be preserved.
    assert!(
        find_descriptor(&audio_track.spec.es_info_descriptors, DESC_TAG_CA).is_none(),
        "test setup: fixture track must not already carry a CA_descriptor"
    );
    let original_descriptors = audio_track.spec.es_info_descriptors.clone();
    let fake_ca_descriptor: Vec<u8> = vec![DESC_TAG_CA, 0x04, 0x00, 0x01, 0x00, 0x82];
    let mut mutated = original_descriptors.clone();
    mutated.extend_from_slice(&fake_ca_descriptor);
    // A second, harmless private descriptor after the CA one, so the CA tag
    // isn't merely the last thing in the loop either.
    let trailer: Vec<u8> = vec![0x88, 0x02, 0xAA, 0xBB];
    mutated.extend_from_slice(&trailer);
    audio_track.spec.es_info_descriptors = mutated;

    let single_track_media = Media::new(vec![audio_track], media.movie_timescale);
    let remuxed = remux_via_ts(&single_track_media);
    assert_eq!(
        remuxed.tracks.len(),
        1,
        "single-track Media must re-mux to a single track"
    );
    let out_descriptors = &remuxed.tracks[0].spec.es_info_descriptors;

    assert!(
        find_descriptor(out_descriptors, DESC_TAG_CA).is_none(),
        "CA_descriptor must be denied from the re-muxed ES_info loop (issue #775)"
    );
    assert!(
        find_descriptor(out_descriptors, DESC_TAG_ISO_639_LANGUAGE).is_some(),
        "sibling ISO_639_language_descriptor must survive the CA denial"
    );
    assert!(
        find_descriptor(out_descriptors, 0x88).is_some(),
        "sibling private descriptor (0x88) placed AFTER the CA_descriptor must also survive"
    );
    // Source order preserved: every tag from `original_descriptors` in
    // order, then the trailer tag — the excised CA tag simply isn't there.
    let mut expected_tags = descriptor_tags(&original_descriptors);
    expected_tags.push(0x88);
    assert_eq!(
        descriptor_tags(out_descriptors),
        expected_tags,
        "surviving descriptor tags must appear in source order with the CA tag excised"
    );
}
