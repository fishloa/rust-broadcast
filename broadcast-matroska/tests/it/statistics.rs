//! The de-facto Matroska statistics and encoder tags, typed, over the real
//! fixtures and against the values an independent tool prints.
//!
//! `ffprobe -show_entries format_tags:stream_tags` on the fixtures prints, for
//! `mkv/vp9-opus.mkv`:
//!
//! ```text
//! ENCODER=Lavf62.12.102
//! DURATION=00:00:02.000000000   ENCODER=Lavc62.28.102 libvpx-vp9
//! DURATION=00:00:02.008000000   ENCODER=Lavc62.28.102 libopus
//! ```
//!
//! and for `mkv/h264-aac.mkv` `DURATION` 00:00:03.023000000 and
//! 00:00:03.042000000. Those are the SimpleTag values this module reads.

use broadcast_core::{Parse, Serialize};
use broadcast_matroska::matroska::Document;

/// The fixture whose per-track statistics the first tests read.
const TAGGED: &str = "mkv/vp9-opus.mkv";
/// A second fixture, so the duration arithmetic is not one file's numbers.
const TAGGED_2: &str = "mkv/h264-aac.mkv";
/// The bytes ffprobe prints for track 1 of [`TAGGED`], in nanoseconds.
const VP9_DURATION_NS: u64 = 2_000_000_000;
/// The bytes ffprobe prints for track 2 of [`TAGGED`], in nanoseconds.
const OPUS_DURATION_NS: u64 = 2_008_000_000;
/// The bytes ffprobe prints for track 1 of [`TAGGED_2`], in nanoseconds.
const AVC_DURATION_NS: u64 = 3_023_000_000;
/// The bytes ffprobe prints for track 2 of [`TAGGED_2`], in nanoseconds.
const AAC_DURATION_NS: u64 = 3_042_000_000;

/// Read one fixture and hand its bytes to `f` with the parsed document.
fn with_document<R>(fixture: &str, f: impl FnOnce(&[u8], &Document<'_>) -> R) -> R {
    let bytes = test_fixtures::read(fixture);
    let document = Document::parse(&bytes).expect("fixture parses");
    f(&bytes, &document)
}

/// The statistics of every tag targeting each track, in track order.
fn statistics_per_track<'a>(
    document: &'a Document<'a>,
) -> Vec<(u64, Vec<broadcast_matroska::matroska::Statistics<'a>>)> {
    let segment = document.segment();
    let tracks = segment.tracks().expect("a Tracks");
    let Some(tags) = segment.tags() else {
        return tracks
            .entries()
            .filter_map(broadcast_matroska::matroska::TrackEntry::track_uid)
            .map(|uid| (uid, Vec::new()))
            .collect();
    };
    tracks
        .entries()
        .filter_map(broadcast_matroska::matroska::TrackEntry::track_uid)
        .map(|uid| (uid, tags.statistics_for_track(uid).collect()))
        .collect()
}

#[test]
fn the_vp9_opus_fixture_carries_ffprobes_durations_and_encoders() {
    with_document(TAGGED, |_bytes, document| {
        let per_track = statistics_per_track(document);
        assert_eq!(per_track.len(), 2, "{TAGGED}: two tracks");
        let video = &per_track[0].1;
        let audio = &per_track[1].1;
        assert_eq!(video.len(), 1, "{TAGGED}: one tag per track");
        assert_eq!(
            video[0].duration_nanoseconds(),
            Some(VP9_DURATION_NS),
            "{TAGGED}: ffprobe prints 00:00:02.000000000"
        );
        assert_eq!(
            video[0].encoder(),
            Some("Lavc62.28.102 libvpx-vp9"),
            "{TAGGED}: ffprobe prints that encoder"
        );
        assert_eq!(
            audio[0].duration_nanoseconds(),
            Some(OPUS_DURATION_NS),
            "{TAGGED}: ffprobe prints 00:00:02.008000000"
        );
        assert_eq!(
            audio[0].encoder(),
            Some("Lavc62.28.102 libopus"),
            "{TAGGED}: ffprobe prints that encoder"
        );
    });
}

#[test]
fn the_h264_aac_fixture_carries_ffprobes_durations() {
    with_document(TAGGED_2, |_bytes, document| {
        let per_track = statistics_per_track(document);
        assert_eq!(per_track.len(), 2, "{TAGGED_2}: two tracks");
        assert_eq!(
            per_track[0].1[0].duration_nanoseconds(),
            Some(AVC_DURATION_NS),
            "{TAGGED_2}: ffprobe prints 00:00:03.023000000"
        );
        assert_eq!(
            per_track[1].1[0].duration_nanoseconds(),
            Some(AAC_DURATION_NS),
            "{TAGGED_2}: ffprobe prints 00:00:03.042000000"
        );
    });
}

#[test]
fn every_fixture_keeps_its_tags_byte_exact_through_a_rewrite() {
    for fixture in [TAGGED, TAGGED_2, "mkv/vp9-opus.webm", "mkv/h264-aac.mkv"] {
        with_document(fixture, |bytes, document| {
            let mut out = vec![0u8; document.serialized_len()];
            document.serialize_into(&mut out).expect("rewrite");
            assert_eq!(out, bytes, "{fixture}: rewrite is byte-identical");
        });
    }
}

/// Corrupt a tag value in place, keeping every element's length unchanged.
fn damage_duration(bytes: &mut [u8]) -> bool {
    let needle = b"00:00:02.000000000";
    let Some(at) = bytes
        .windows(needle.len())
        .position(|window| window == needle)
    else {
        return false;
    };
    bytes[at..at + needle.len()].copy_from_slice(b"zz:zz:zz.zzzzzzzzz");
    true
}

#[test]
fn a_damaged_tag_value_is_a_finding_and_the_file_still_reads() {
    let mut bytes = test_fixtures::read(TAGGED);
    assert!(damage_duration(&mut bytes), "{TAGGED}: duration present");
    let document = Document::parse(&bytes).expect("a damaged value is not a damaged element");
    let per_track = statistics_per_track(&document);
    let video = &per_track[0].1[0];
    assert_eq!(
        video.duration_nanoseconds(),
        None,
        "a duration that does not parse is skipped"
    );
    assert_eq!(video.malformed().len(), 1, "and reported as a finding");
    assert_eq!(video.malformed()[0].name, "DURATION");
    assert_eq!(
        video.encoder(),
        Some("Lavc62.28.102 libvpx-vp9"),
        "the rest of the tag still reads"
    );
    assert_eq!(
        per_track[1].1[0].duration_nanoseconds(),
        Some(OPUS_DURATION_NS),
        "the other track is unaffected"
    );
}

#[test]
fn a_mutated_duration_fails_the_fixture_value() {
    let mut bytes = test_fixtures::read(TAGGED);
    let needle = b"00:00:02.000000000";
    let at = bytes
        .windows(needle.len())
        .position(|window| window == needle)
        .expect("duration present");
    bytes[at..at + needle.len()].copy_from_slice(b"00:00:05.000000000");
    let document = Document::parse(&bytes).expect("fixture parses");
    let per_track = statistics_per_track(&document);
    assert_ne!(
        per_track[0].1[0].duration_nanoseconds(),
        Some(VP9_DURATION_NS),
        "the mutation is visible, so the value assertion above bites"
    );
}
