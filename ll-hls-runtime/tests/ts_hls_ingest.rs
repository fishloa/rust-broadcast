//! Issue #760 acceptance: classic MPEG-TS-segment HLS (HLS v3, RFC 8216 —
//! the dominant legacy/IPTV form: whole `.ts` segments, no `EXT-X-MAP`/init
//! resource, self-contained PAT/PMT/PES per segment) ingest through the
//! sans-IO [`LlHlsClient`], mirroring `examples/client_stepping.rs`'s
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
//! (exactly what `LlHlsClient`'s TS routing does internally, per segment)
//! and compare the per-track sample counts against what actually drained out
//! of the client. If issue #760's TS routing were ever removed, the client
//! would have no init resource to wait for (this playlist never advertises
//! one) and would buffer every segment forever — zero `Output::Samples` ever
//! emitted — so the non-zero/oracle-matching assertions below would fail.

use std::collections::BTreeMap;
use std::path::PathBuf;

use broadcast_common::Unpackage;
use ll_hls_runtime::client::{Action, LlHlsClient, Output};
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
fn drive_to_end(client: &mut LlHlsClient) -> Vec<Output> {
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
/// one-segment-at-a-time shape `LlHlsClient`'s TS routing uses internally),
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

    let mut client = LlHlsClient::new(PLAYLIST_URL);
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
