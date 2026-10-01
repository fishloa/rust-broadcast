//! Issue #760 acceptance: classic MPEG-TS-segment HLS (HLS v3, RFC 8216 —
//! the dominant legacy/IPTV form: whole `.ts` segments, no `EXT-X-MAP`/init
//! resource, self-contained PAT/PMT/PES per segment) ingest through the
//! sans-IO [`HlsClient`], mirroring `examples/client_stepping.rs`'s
//! offline drive style (no socket, no real network — fixture bytes are fed
//! straight from disk).
//!
//! The fixture (`tests/fixtures/ts-hls/`, see its `PROVENANCE.md`) is real
//! ffmpeg `-f hls -hls_segment_type mpegts` output generated from the
//! workspace's own committed `fixtures/ts/h264_aac.ts` capture — not
//! hand-typed bytes, so it carries the real PAT/PMT/PES layout the wild
//! (not just the happy path a synthetic fixture would cover).
//!
//! The oracle: demux each `.ts` segment directly via `transmux::TsDemux`
//! (exactly what `HlsClient`'s TS routing does internally, per segment)
//! and compare the per-track sample counts against what actually drained out
//! of the client. If issue #760's TS routing were ever removed, the client
//! would have no init resource to wait for (this playlist never advertises
//! one) and would buffer every segment forever — zero `Output::Samples` ever
//! emitted — so the non-zero/oracle-matching assertions below would fail.

use std::collections::BTreeMap;
use std::path::PathBuf;

use broadcast_common::Unpackage;
use hls_runtime::client::{Action, HlsClient, Output};
use transmux::{CodecConfig, TsDemux};

const PLAYLIST_URL: &str = "http://fixture/index.m3u8";
const TS_SYNC_BYTE: u8 = 0x47;

fn fixture_dir() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ts-hls"
    ))
}

fn read_fixture(name: &str) -> Vec<u8> {
    let path = fixture_dir().join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

/// The two `.ts` segment filenames the committed playlist references, in
/// order — read directly from the committed `index.m3u8` rather than
/// hardcoded, so this test breaks loudly (not silently) if the fixture is
/// ever regenerated with a different segment count/names.
fn segment_names_from_playlist(playlist_text: &str) -> Vec<String> {
    playlist_text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect()
}

/// Drive `client` to completion against the fixture on disk (playlist +
/// segment bytes), draining every `Output` in order. No HTTP, no real
/// clock — every `Action` this VOD (ENDLIST) fixture can ever produce is
/// answered synchronously from the fixture directory.
fn drive_to_end(client: &mut HlsClient) -> Vec<Output> {
    let mut outputs = Vec::new();
    loop {
        match client.poll() {
            Some(Action::FetchPlaylist { url, blocking, .. }) => {
                assert_eq!(url, PLAYLIST_URL);
                assert!(
                    blocking.is_none(),
                    "this fixture's playlist has ENDLIST and no LL-HLS server-control, so \
                     the client must never ask for a blocking reload"
                );
                let text = read_fixture("index.m3u8");
                client.on_playlist(&text).expect("fixture playlist parses");
            }
            Some(Action::FetchResource { id, url, .. }) => {
                let name = url.rsplit('/').next().expect("url has a path segment");
                let bytes = read_fixture(name);
                client
                    .on_resource(id, &bytes)
                    .unwrap_or_else(|e| panic!("demux fixture resource {name}: {e}"));
            }
            Some(Action::WaitMs(_)) => {}
            Some(other) => panic!("unexpected action for this VOD fixture: {other:?}"),
            None => break,
        }
        while let Some(output) = client.next_output() {
            outputs.push(output);
        }
    }
    outputs
}

/// Direct-demux oracle: per-track sample counts from feeding each `.ts`
/// segment through `transmux::TsDemux` one at a time (the same
/// one-segment-at-a-time shape `HlsClient`'s TS routing uses internally),
/// independent of the client entirely.
fn oracle_track_totals(segment_names: &[String]) -> BTreeMap<u32, usize> {
    let mut totals = BTreeMap::new();
    for name in segment_names {
        let bytes = read_fixture(name);
        let media = TsDemux::new()
            .demux(&bytes)
            .unwrap_or_else(|e| panic!("oracle demux of {name} failed: {e}"));
        for track in media.tracks {
            *totals.entry(track.spec.track_id).or_insert(0) += track.samples.len();
        }
    }
    totals
}

#[test]
fn fixture_is_genuinely_classic_ts_segment_hls() {
    let playlist_text = read_fixture("index.m3u8");
    let playlist_text = String::from_utf8(playlist_text).expect("playlist is UTF-8");
    assert!(
        !playlist_text.contains("EXT-X-MAP"),
        "fixture must carry NO EXT-X-MAP (classic TS-segment HLS has no init resource):\n{playlist_text}"
    );
    let names = segment_names_from_playlist(&playlist_text);
    assert!(
        names.len() >= 2,
        "expect at least two segments from the ffmpeg -hls_time 2 generation: {names:?}"
    );
    for name in &names {
        let bytes = read_fixture(name);
        assert_eq!(
            bytes.first().copied(),
            Some(TS_SYNC_BYTE),
            "segment {name} must start with the MPEG-TS sync byte 0x47"
        );
    }
}

/// The headline #760 acceptance: the sans-IO client ingests the classic
/// TS-segment HLS fixture end to end -- exactly one synthesized
/// `Output::Init` before any `Output::Samples`, an `Output::EndOfStream` at
/// the close (ENDLIST + nothing outstanding), and per-track sample counts
/// matching the direct `TsDemux` oracle exactly (no drops/dupes).
#[test]
fn client_ingests_classic_ts_segment_hls_end_to_end() {
    let playlist_text = read_fixture("index.m3u8");
    let playlist_text_str = String::from_utf8(playlist_text).expect("playlist is UTF-8");
    let segment_names = segment_names_from_playlist(&playlist_text_str);

    let mut client = HlsClient::new(PLAYLIST_URL);
    let outputs = drive_to_end(&mut client);

    assert!(
        !outputs.is_empty(),
        "the client must produce output for a real TS-HLS fixture -- empty output means the \
         TS routing never fired and every segment is stuck buffered forever"
    );

    // Exactly one Init, and it precedes every Samples batch.
    let init_positions: Vec<usize> = outputs
        .iter()
        .enumerate()
        .filter(|(_, o)| matches!(o, Output::Init(_)))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        init_positions.len(),
        1,
        "exactly one synthesized Output::Init expected for classic TS-HLS: {outputs:?}"
    );
    let first_samples_pos = outputs
        .iter()
        .position(|o| matches!(o, Output::Samples { .. }))
        .expect("at least one Output::Samples batch expected");
    assert!(
        init_positions[0] < first_samples_pos,
        "Output::Init must precede every Output::Samples"
    );

    // The synthesized Init is a real, Fmp4Demux-decodable ftyp+moov exposing
    // the AVC video + AAC audio tracks TsDemux recovered from the fixture --
    // not just non-empty bytes.
    let Output::Init(init_bytes) = &outputs[init_positions[0]] else {
        unreachable!("checked above")
    };
    let init_media = transmux::Fmp4Demux::new()
        .unpackage(init_bytes.as_slice())
        .expect("the synthesized Init segment must itself be a valid, demuxable fMP4 init");
    assert!(
        init_media
            .tracks
            .iter()
            .any(|t| matches!(t.spec.config, CodecConfig::Avc { .. })),
        "synthesized Init must expose the fixture's AVC video track: {:?}",
        init_media.tracks
    );
    assert!(
        init_media
            .tracks
            .iter()
            .any(|t| matches!(t.spec.config, CodecConfig::Aac { .. })),
        "synthesized Init must expose the fixture's AAC audio track: {:?}",
        init_media.tracks
    );

    assert!(
        matches!(outputs.last(), Some(Output::EndOfStream)),
        "a VOD (ENDLIST) playlist with nothing outstanding must end in Output::EndOfStream: \
         {outputs:?}"
    );

    // Per-track sample totals must match the direct TsDemux oracle exactly.
    let mut got_totals: BTreeMap<u32, usize> = BTreeMap::new();
    for output in &outputs {
        if let Output::Samples { track_id, samples } = output {
            *got_totals.entry(*track_id).or_insert(0) += samples.len();
        }
    }
    let want_totals = oracle_track_totals(&segment_names);
    assert!(
        !want_totals.is_empty(),
        "sanity: the oracle itself must see at least one track"
    );
    assert_eq!(
        got_totals, want_totals,
        "client-emitted per-track sample counts must match the direct TsDemux oracle exactly \
         (no drops/dupes/misroutes)"
    );
}

// ---------------------------------------------------------------------------
// r09-W15 (#1089): one TS demuxer across segments.
// ---------------------------------------------------------------------------

/// 33-bit PTS/DTS modulus (ISO/IEC 13818-1 §2.4.3.7).
const PTS_MODULUS: u64 = 1 << 33;
const TS_PACKET_LEN: usize = 188;

fn decode_pts(b: &[u8]) -> u64 {
    (u64::from(b[0] >> 1) & 0x7) << 30
        | u64::from(b[1]) << 22
        | u64::from(b[2] >> 1) << 15
        | u64::from(b[3]) << 7
        | u64::from(b[4] >> 1)
}

fn encode_pts(prefix: u8, v: u64) -> [u8; 5] {
    [
        (prefix << 4) | ((((v >> 30) & 0x7) as u8) << 1) | 1,
        ((v >> 22) & 0xFF) as u8,
        ((((v >> 15) & 0x7F) as u8) << 1) | 1,
        ((v >> 7) & 0xFF) as u8,
        (((v & 0x7F) as u8) << 1) | 1,
    ]
}

/// Add `offset` (mod 2^33) to every PES PTS and DTS in a TS byte stream
/// (test-only fixture transformer; the demuxer under test never sees this
/// code's logic).
fn shift_pes_timestamps(ts: &[u8], offset: u64) -> Vec<u8> {
    let mut out = ts.to_vec();
    for pkt in out.chunks_exact_mut(TS_PACKET_LEN) {
        assert_eq!(pkt[0], TS_SYNC_BYTE);
        let pusi = pkt[1] & 0x40 != 0;
        let afc = (pkt[3] >> 4) & 0x3;
        if !pusi || afc & 1 == 0 {
            continue;
        }
        let mut at = 4;
        if afc == 3 {
            at += 1 + usize::from(pkt[4]);
        }
        if pkt[at..at + 3] != [0, 0, 1] {
            continue;
        }
        let flags = pkt[at + 7] >> 6;
        if flags & 0b10 != 0 {
            let p = at + 9;
            let prefix = pkt[p] >> 4;
            let v = (decode_pts(&pkt[p..p + 5]) + offset) % PTS_MODULUS;
            pkt[p..p + 5].copy_from_slice(&encode_pts(prefix, v));
        }
        if flags == 0b11 {
            let p = at + 14;
            let prefix = pkt[p] >> 4;
            let v = (decode_pts(&pkt[p..p + 5]) + offset) % PTS_MODULUS;
            pkt[p..p + 5].copy_from_slice(&encode_pts(prefix, v));
        }
    }
    out
}

fn avc_track_dts(media: &transmux::Media) -> (u32, Vec<i64>) {
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("fixture has an AVC track");
    (
        track.spec.track_id,
        track
            .samples
            .iter()
            .map(|s| s.dts.expect("TS video sample carries a DTS"))
            .collect(),
    )
}

/// A sample's `(dts, pts, duration)`.
type Timing = (Option<i64>, Option<i64>, Option<u32>);

/// `(dts, pts, duration)` of every sample of the track whose config matches.
fn timing_of(media: &transmux::Media, pick: impl Fn(&CodecConfig) -> bool) -> (u32, Vec<Timing>) {
    let track = media
        .tracks
        .iter()
        .find(|t| pick(&t.spec.config))
        .expect("fixture has the track");
    (
        track.spec.track_id,
        track
            .samples
            .iter()
            .map(|s| (s.dts, s.pts, s.duration))
            .collect(),
    )
}

/// Drive `client` over `segments` (url basename -> bytes) with `playlist`.
fn drive_in_memory(playlist: &str, segments: &[(&str, Vec<u8>)]) -> Vec<Output> {
    let mut client = HlsClient::new(PLAYLIST_URL);
    let mut outputs = Vec::new();
    loop {
        match client.poll() {
            Some(Action::FetchPlaylist { .. }) => client
                .on_playlist(playlist.as_bytes())
                .expect("playlist parses"),
            Some(Action::FetchResource { id, url, .. }) => {
                let name = url.rsplit('/').next().expect("path");
                let bytes = &segments
                    .iter()
                    .find(|(n, _)| *n == name)
                    .unwrap_or_else(|| panic!("no bytes for {name}"))
                    .1;
                client.on_resource(id, bytes).expect("segment demuxes");
            }
            Some(Action::WaitMs(_)) => {}
            Some(other) => panic!("unexpected action {other:?}"),
            None => break,
        }
        while let Some(o) = client.next_output() {
            outputs.push(o);
        }
    }
    outputs
}

/// A 24/7 TS pull crosses the 2^33 PTS wrap (~26.5 h) between two segments.
/// A fresh demuxer per segment restarted the unwrap, feeding a DTS that
/// jumped back by 2^33 ticks; the persistent demuxer must keep counting up,
/// and every sample (video *and* audio) must carry the duration the batch
/// demuxer computes over the whole continuous stream — including each
/// segment's last access unit, whose real duration is only known once the
/// next segment's first sample arrives.
///
/// Oracle: the independent batch `transmux::TsDemux` over the *unshifted*
/// concatenation of both segments (no wrap involved), plus the literal
/// `offset` the test applied.
#[test]
fn pts_wrap_between_segments_is_unrolled_across_the_boundary() {
    let seg0 = read_fixture("index0.ts");
    let seg1 = read_fixture("index1.ts");
    let (_, dts0) = avc_track_dts(&TsDemux::new().demux(&seg0).unwrap());
    let (_, dts1) = avc_track_dts(&TsDemux::new().demux(&seg1).unwrap());
    let first_of_seg1 = u64::try_from(dts1[0]).unwrap();
    let last_of_seg0 = u64::try_from(*dts0.last().unwrap()).unwrap();
    assert!(
        last_of_seg0 < first_of_seg1,
        "fixture segments are contiguous"
    );

    // Segment 1 is moved `EXTRA` ticks later than a constant-frame-rate
    // continuation, so the gap between segment 0's last and segment 1's first
    // video frame differs from the usual frame duration: only the real delta
    // (known once segment 1 arrives) gives segment 0's last frame its true
    // duration, a guess from the previous frame does not.
    const EXTRA: u64 = 1500;
    let reference = TsDemux::new()
        .demux(&[seg0.clone(), shift_pes_timestamps(&seg1, EXTRA)].concat())
        .unwrap();
    let is_video = |c: &CodecConfig| matches!(c, CodecConfig::Avc { .. });
    let is_audio = |c: &CodecConfig| matches!(c, CodecConfig::Aac { .. });
    let (video_id, want_video) = timing_of(&reference, is_video);
    let (audio_id, want_audio) = timing_of(&reference, is_audio);

    // Shift so segment 0's video DTS stay below 2^33 and segment 1's first
    // DTS lands exactly on raw value 1, i.e. just past the wrap.
    let offset = PTS_MODULUS - first_of_seg1 - EXTRA + 1;
    let shifted0 = shift_pes_timestamps(&seg0, offset);
    let shifted1 = shift_pes_timestamps(&seg1, offset + EXTRA);

    // Sanity: the shift really put the wrap between the segments.
    let (_, shifted_dts0) = avc_track_dts(&TsDemux::new().demux(&shifted0).unwrap());
    assert!(
        shifted_dts0
            .iter()
            .all(|&d| u64::try_from(d).unwrap() < PTS_MODULUS)
    );

    let outputs = drive_in_memory(
        &String::from_utf8(read_fixture("index.m3u8")).unwrap(),
        &[("index0.ts", shifted0), ("index1.ts", shifted1)],
    );
    let timing = |track: u32| -> Vec<Timing> {
        outputs
            .iter()
            .filter_map(|o| match o {
                Output::Samples { track_id, samples } if *track_id == track => Some(samples),
                _ => None,
            })
            .flatten()
            .map(|s| (s.dts, s.pts, s.duration))
            .collect()
    };

    // Video: exactly the reference, shifted by the literal offset.
    let offset = i64::try_from(offset).unwrap();
    let want_video_shifted: Vec<_> = want_video
        .iter()
        .map(|(d, p, dur)| (d.map(|v| v + offset), p.map(|v| v + offset), *dur))
        .collect();
    let got_video = timing(video_id);
    assert_eq!(
        got_video, want_video_shifted,
        "video dts/pts/duration must match the continuous-stream reference across the wrap"
    );
    assert!(
        got_video.last().unwrap().0.unwrap() >= i64::try_from(PTS_MODULUS).unwrap(),
        "the last segment's DTS must be unrolled past 2^33"
    );

    // Audio is in its own timescale, so compare what is timescale-exact:
    // every duration, every successive dts delta, every pts-dts gap.
    let got_audio = timing(audio_id);
    assert_eq!(got_audio.len(), want_audio.len());
    let durations = |v: &[Timing]| -> Vec<Option<u32>> { v.iter().map(|t| t.2).collect() };
    let deltas = |v: &[Timing]| -> Vec<i64> {
        v.windows(2)
            .map(|w| w[1].0.unwrap() - w[0].0.unwrap())
            .collect()
    };
    let gaps =
        |v: &[Timing]| -> Vec<i64> { v.iter().map(|t| t.1.unwrap() - t.0.unwrap()).collect() };
    assert_eq!(durations(&got_audio), durations(&want_audio));
    assert_eq!(deltas(&got_audio), deltas(&want_audio));
    assert_eq!(gaps(&got_audio), gaps(&want_audio));
}

/// `fixtures/ts/h264/main.ts` (video only) after the h264+AAC segments, with
/// an `EXT-X-DISCONTINUITY` between: the track set changes, so a second,
/// different `Output::Init` must follow the `Output::Discontinuity`.
#[test]
fn track_set_change_across_a_discontinuity_re_emits_init() {
    let seg0 = read_fixture("index0.ts");
    let other = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/ts/h264/main.ts"
    ))
    .expect("workspace h264 fixture");
    let playlist = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n\
#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\nindex0.ts\n#EXT-X-DISCONTINUITY\n\
#EXTINF:2.0,\nmain.ts\n#EXT-X-ENDLIST\n";
    let outputs = drive_in_memory(playlist, &[("index0.ts", seg0), ("main.ts", other.clone())]);

    let kinds: Vec<&str> = outputs
        .iter()
        .map(|o| match o {
            Output::Init(_) => "init",
            Output::Samples { .. } => "samples",
            Output::Discontinuity => "disc",
            Output::EndOfStream => "end",
            _ => "other",
        })
        .collect();
    let init_at: Vec<usize> = kinds
        .iter()
        .enumerate()
        .filter(|(_, k)| **k == "init")
        .map(|(i, _)| i)
        .collect();
    let disc_at = kinds
        .iter()
        .position(|k| *k == "disc")
        .expect("discontinuity");
    assert_eq!(init_at.len(), 2, "{kinds:?}");
    assert!(init_at[0] < disc_at && disc_at < init_at[1], "{kinds:?}");
    assert_eq!(kinds.last(), Some(&"end"));

    let tracks = |o: &Output| match o {
        Output::Init(b) => transmux::Fmp4Demux::new()
            .unpackage(b.as_slice())
            .unwrap()
            .tracks
            .iter()
            .map(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
            .collect::<Vec<bool>>(),
        _ => unreachable!(),
    };
    assert!(
        tracks(&outputs[init_at[0]]).contains(&true),
        "first init has AAC"
    );
    assert_eq!(
        tracks(&outputs[init_at[1]]),
        vec![false],
        "second init is the single video track of main.ts"
    );
}

/// A duplicate delivery of a TS segment must not emit its samples twice
/// (r09-W16), checked against a real demuxable segment.
#[test]
fn duplicate_ts_segment_delivery_emits_samples_once() {
    let mut client = HlsClient::new(PLAYLIST_URL);
    while client.poll().is_some() {}
    client
        .on_playlist(&read_fixture("index.m3u8"))
        .expect("playlist");
    let ids: Vec<_> = std::iter::from_fn(|| client.poll())
        .filter_map(|a| match a {
            Action::FetchResource { id, .. } => Some(id),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 2);
    let seg0 = read_fixture("index0.ts");
    client.on_resource(ids[0], &seg0).unwrap();
    let err = client.on_resource(ids[0], &seg0).expect_err("duplicate");
    assert!(matches!(
        err,
        hls_runtime::client::Error::DuplicateResource { .. }
    ));

    // Deliver the second segment, then count everything: each segment's
    // samples exactly once (the last access units are held back until the
    // next segment or ENDLIST, so only the total over both is exact).
    client
        .on_resource(ids[1], &read_fixture("index1.ts"))
        .unwrap();
    let per_segment = |name: &str| -> usize {
        TsDemux::new()
            .demux(&read_fixture(name))
            .unwrap()
            .tracks
            .iter()
            .map(|t| t.samples.len())
            .sum()
    };
    let want = per_segment("index0.ts") + per_segment("index1.ts");
    let mut got = 0;
    let mut ended = false;
    while let Some(o) = client.next_output() {
        match o {
            Output::Samples { samples, .. } => got += samples.len(),
            Output::EndOfStream => ended = true,
            _ => {}
        }
    }
    assert!(ended, "ENDLIST playlist with nothing outstanding ends");
    assert_eq!(got, want);
}
