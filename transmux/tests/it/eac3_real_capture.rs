//! Issue #1186: E-AC-3 config recovery now waits for a complete access unit,
//! and (when the programme has dependent substreams) holds the final access
//! unit of a PES back so a dependent substream starting the next PES can join
//! it. Real broadcast captures carry NO dependent substreams, so these runs
//! prove the new machinery changes nothing for them: the config is found, no
//! dependent substream is invented, and every sample is exactly one whole
//! independent syncframe at the frame-exact cadence.
//!
//! Inputs: the committed `fixtures/ts/dolby/eac3.ts` and
//! `fixtures/ts/gulli-opengop.ts`, plus (skipped cleanly when absent) the
//! `france-tnt-uhf32-si-audio.ts` from the private fixtures repo (the whole
//! capture's non-video packets).
use transmux::Ec3SyncframeInfo;
use transmux::ac3_syncframe::{split_eac3_syncframe_ranges, split_eac3_syncframes};
use transmux::pipeline::CodecConfig;

const AC3_SYNC: [u8; 2] = [0x0B, 0x77];

/// Per E-AC-3 track: `(samples, whole-frame timeline gaps)`.
fn check(label: &str, ts: &[u8]) -> Vec<(usize, usize)> {
    let media = test_harness::collect::media_from_ts(ts).expect("demux");
    let mut out = Vec::new();
    for track in &media.tracks {
        let CodecConfig::Eac3 { config, .. } = &track.spec.config else {
            continue;
        };
        assert!(
            config
                .substreams
                .iter()
                .all(|s| s.num_dep_sub == 0 && s.chan_loc.is_none()),
            "{label}: a real 5.1 capture has no dependent substream, got {config:?}"
        );
        assert!(!track.samples.is_empty(), "{label}: samples emitted");
        let mut prev_dts: Option<i64> = None;
        let mut gaps = 0usize;
        for (i, s) in track.samples.iter().enumerate() {
            assert_eq!(
                &s.data[..2],
                &AC3_SYNC,
                "{label}: sample {i} starts at a syncword"
            );
            let frames = Ec3SyncframeInfo::from_es_all(&s.data);
            assert_eq!(frames.len(), 1, "{label}: sample {i} holds one syncframe");
            let info = frames[0];
            assert_eq!(
                (info.frmsiz as usize + 1) * 2,
                s.data.len(),
                "{label}: sample {i} is exactly one whole syncframe"
            );
            assert_eq!(
                s.timing.and_then(|t| t.duration()),
                Some(info.samples_per_frame())
            );
            if let Some(p) = prev_dts {
                // Frame-exact cadence, except where the real capture itself
                // re-anchors (a timeline gap is a whole number of frames).
                let delta = s.timing.and_then(|t| t.dts()).unwrap() - p;
                let frame = info.samples_per_frame() as i64;
                if delta != frame {
                    assert!(
                        delta > frame && delta % frame == 0,
                        "{label}: sample {i} dts delta {delta} is neither one frame nor a whole-frame gap"
                    );
                    gaps += 1;
                }
            }
            prev_dts = s.timing.and_then(|t| t.dts());
        }
        out.push((track.samples.len(), gaps));
    }
    out
}

fn committed(name: &str) -> Vec<u8> {
    let path = test_fixtures::path(&format!("ts/{}", name));
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn committed_real_eac3_captures_unchanged() {
    assert_eq!(
        check("dolby/eac3.ts", &committed("dolby/eac3.ts")),
        [(58, 0)]
    );
    assert_eq!(
        check("gulli-opengop.ts", &committed("gulli-opengop.ts")),
        [(230, 0)]
    );
}

/// A PES payload routinely ends mid-frame (a lost packet truncates the tail),
/// so the E-AC-3 walkers must drop the incomplete frame rather than index past
/// the buffer — both the access-unit split and the syncframe ranges.
#[test]
fn a_truncated_last_eac3_frame_is_dropped() {
    let media = test_harness::collect::media_from_ts(&committed("dolby/eac3.ts")).expect("demux");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Eac3 { .. }))
        .expect("an E-AC-3 track");
    let es: Vec<u8> = track
        .samples
        .iter()
        .flat_map(|s| s.data.iter().copied())
        .collect();
    let whole = split_eac3_syncframes(&es);
    let ranges = split_eac3_syncframe_ranges(&es);
    assert_eq!(whole.len(), 58);
    assert_eq!(ranges.len(), whole.len());
    assert_eq!(ranges.last().unwrap().1, es.len());

    let cut = &es[..es.len() - 1];
    assert_eq!(
        split_eac3_syncframes(cut).len(),
        whole.len() - 1,
        "only the complete frames are access units"
    );
    let cut_ranges = split_eac3_syncframe_ranges(cut);
    assert_eq!(cut_ranges.len(), whole.len() - 1);
    assert!(
        cut_ranges.iter().all(|&(_, end)| end <= cut.len()),
        "no range may end past the payload"
    );

    let first_len = ranges[0].1 - ranges[0].0;
    let one_plus_two = &es[..first_len + 2];
    assert_eq!(split_eac3_syncframes(one_plus_two).len(), 1);
    assert_eq!(split_eac3_syncframe_ranges(one_plus_two), [(0, first_len)]);
}

#[test]
fn real_capture_france_tnt_eac3_unchanged() {
    // The whole 62 s capture, video PES dropped (`tools/make-fixture-excerpts.py
    // si-audio`). Per-track values below are re-derived from the excerpt. They
    // are equivalent in coverage to the previous pin over the first 24 MB of the
    // full capture: that pin (3..8 timeline gaps per track) was reproduced
    // exactly from the same 24 MB with the video removed, so the E-AC-3
    // demux result does not depend on the video packets, and the whole-capture
    // run extends it to 1.5k+ frames per track.
    let Some(buf) =
        test_fixtures::require_bytes("ts/france-tnt-uhf32-si-audio.ts", "eac3_real_capture")
    else {
        return;
    };
    assert_eq!(
        check("france-tnt-uhf32-si-audio.ts", &buf),
        [
            (1560, 49),
            (1536, 51),
            (1533, 34),
            (1518, 55),
            (1542, 52),
            (1536, 53),
            (1566, 47),
            (1540, 33),
            (1566, 48),
            (1524, 51),
            (1542, 50),
            (1603, 31)
        ]
    );
}
