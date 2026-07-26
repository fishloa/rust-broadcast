//! Integration test for the SSAI ad-stitch walkthrough (issue #664).
//!
//! `#[path]`-includes `examples/ssai_ad_stitch.rs` as a module so this test
//! exercises the *exact* pipeline the example runs (fixture -> hand-built
//! SCTE-35 cue -> TS extraction -> `splice_insert` -> HLS/DASH), then asserts
//! on the concrete rendered text/bytes — not just "it ran without panicking".

#[allow(dead_code)]
#[path = "../examples/ssai_ad_stitch.rs"]
mod ssai_ad_stitch;

use timed_metadata::convert::emsg_to_scte35;
use transmux::EmsgBox;

/// The HLS media playlist must carry exactly one `#EXT-X-DISCONTINUITY` per
/// splice join (ad-in + resume = 2), each immediately before an `#EXTINF`.
#[test]
fn playlist_has_discontinuity_at_each_splice_point() {
    let demo = ssai_ad_stitch::run().expect("pipeline runs end to end");

    assert_eq!(
        demo.discontinuity_points.len(),
        4,
        "2 tracks x (ad-in, resume) = 4 SplicePoints, got {:?}",
        demo.discontinuity_points
    );

    let discontinuity_count = demo.m3u8.matches("#EXT-X-DISCONTINUITY\n").count();
    assert_eq!(
        discontinuity_count, 2,
        "video playlist must carry exactly 2 #EXT-X-DISCONTINUITY tags (ad-in + resume):\n{}",
        demo.m3u8
    );

    // Every #EXT-X-DISCONTINUITY line must be immediately followed by an
    // #EXTINF line (RFC 8216 §4.3.4.3: the tag precedes the segment it marks).
    for (line, next) in demo.m3u8.lines().zip(demo.m3u8.lines().skip(1)) {
        if line == "#EXT-X-DISCONTINUITY" {
            assert!(
                next.starts_with("#EXTINF:"),
                "#EXT-X-DISCONTINUITY must be followed by #EXTINF, got {next:?}"
            );
        }
    }

    // First segment (before the ad) is NOT discontinuous.
    let first_extinf_idx = demo.m3u8.find("#EXTINF").expect("has a segment");
    let first_disc_idx = demo.m3u8.find("#EXT-X-DISCONTINUITY");
    assert!(
        first_disc_idx.is_none_or(|d| d > first_extinf_idx),
        "the very first segment must not be discontinuous"
    );
}

/// The `EXT-X-DATERANGE` tag must be present, reference the real cue's
/// `splice_event_id`, and carry the verbatim SCTE-35 bytes as `SCTE35-OUT`.
#[test]
fn daterange_carries_the_real_cue_verbatim() {
    let demo = ssai_ad_stitch::run().expect("pipeline runs end to end");

    assert_eq!(demo.daterange.id, "100002");
    assert_eq!(
        demo.daterange.planned_duration,
        Some(1.0),
        "ad break is 1.0s"
    );

    let scte35 = demo
        .daterange
        .scte35
        .as_ref()
        .expect("DATERANGE carries a SCTE35 attribute");
    assert_eq!(
        scte35.raw, demo.raw_cue,
        "DATERANGE must carry the exact bytes extracted from the TS PID"
    );

    let tag_line = demo.daterange.to_tag_line();
    assert!(
        demo.m3u8.contains(&tag_line),
        "rendered playlist must contain the exact DATERANGE tag line:\n{tag_line}\n---\n{}",
        demo.m3u8
    );
    assert!(tag_line.contains("ID=\"100002\""));
    assert!(tag_line.contains("PLANNED-DURATION=1"));
}

/// The DASH MPD must declare the inband SCTE-35 `InbandEventStream`, and the
/// `emsg` box embedded in the ad segment must parse back to the exact same
/// verbatim SCTE-35 cue bytes carried in the HLS DATERANGE.
#[test]
fn mpd_declares_inband_event_stream_and_emsg_round_trips() {
    let demo = ssai_ad_stitch::run().expect("pipeline runs end to end");

    assert!(
        demo.mpd
            .contains(r#"<InbandEventStream schemeIdUri="urn:scte:scte35:2013:bin"/>"#),
        "MPD must declare the SCTE-35 InbandEventStream on the video AdaptationSet:\n{}",
        demo.mpd
    );
    assert!(
        demo.mpd.contains(r#"<Representation id="1""#),
        "video representation id=1"
    );
    assert!(
        demo.mpd.contains(r#"<Representation id="2""#),
        "audio representation id=2"
    );

    let emsg = EmsgBox::parse(&demo.emsg_bytes).expect("emsg box parses");
    assert!(
        emsg.is_scte35(),
        "emsg scheme must be the SCTE-35 binary carriage"
    );
    assert_eq!(emsg.id, 100_002);

    let extracted = emsg_to_scte35(&demo.emsg_bytes).expect("emsg carries a SCTE-35 message_data");
    assert_eq!(
        extracted, demo.raw_cue,
        "the emsg's message_data must round-trip to the exact same cue bytes as the DATERANGE"
    );
}

/// The manifests + every segment/init file the pipeline claims to have
/// written must actually exist on disk with non-trivial content.
#[test]
fn output_files_are_written_to_disk() {
    let demo = ssai_ad_stitch::run().expect("pipeline runs end to end");

    let expect_file = |name: &str| {
        let path = demo.out_dir.join(name);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert!(!bytes.is_empty(), "{name} must not be empty");
    };

    expect_file("playlist.m3u8");
    expect_file("manifest.mpd");
    expect_file("init-1.mp4");
    expect_file("init-2.mp4");
    for i in 1..=3 {
        expect_file(&format!("seg-1-{i}.m4s"));
        expect_file(&format!("seg-2-{i}.m4s"));
    }
    expect_file("cue.bin");
    expect_file("emsg.bin");
}
