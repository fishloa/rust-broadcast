//! Master (multivariant) playlist handling in `HlsClient`.

use std::path::PathBuf;

use hls_runtime::client::{Action, Error, HlsClient, Output, VariantPolicy};

fn read_ts_fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ts-hls"
    ))
    .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

const PLAYLIST_URL: &str = "http://fixture/index.m3u8";

#[test]
fn master_emits_selection_then_fetches_chosen_variant_playlist() {
    let master_url = "http://example.com/bipbop/master.m3u8";
    let mut client = HlsClient::new(master_url);
    let bytes = test_fixtures::read("hls/real/bipbop-ts/master.m3u8");
    client.on_playlist(&bytes).expect("master parses");

    let out = client.next_output().expect("Master output");
    match out {
        Output::Master(sel) => {
            assert_eq!(sel.chosen, 4); // highest bandwidth gear5
        }
        other => panic!("expected Master, got {other:?}"),
    }

    let action = client.poll().expect("FetchPlaylist");
    match action {
        Action::FetchPlaylist { url, .. } => {
            assert_eq!(url, "http://example.com/bipbop/gear5/prog_index.m3u8");
        }
        other => panic!("expected FetchPlaylist, got {other:?}"),
    }
    // no further action queued yet
    assert!(client.poll().is_none());
}

enum Expect {
    HighestBandwidth,
    LowestBandwidth,
    Bandwidth(u32),
    Chosen(usize),
    CodecsContain(&'static str),
}

const TIE_MASTER: &str =
    "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000\na.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=1000\nb.m3u8\n";

#[test]
fn selection_policy_table() {
    let bipbop = || test_fixtures::read("hls/real/bipbop-ts/master.m3u8");
    let hevc = || test_fixtures::read("hls/real/bipbop-fmp4-hevc/master.m3u8");
    let codecs = |names: [&str; 2]| VariantPolicy {
        supported_codecs: Some(names.iter().map(|n| (*n).to_string()).collect()),
        ..Default::default()
    };
    let cases: Vec<(&str, Vec<u8>, VariantPolicy, Expect)> = vec![
        (
            "default picks the highest bandwidth",
            bipbop(),
            VariantPolicy::default(),
            Expect::HighestBandwidth,
        ),
        (
            "max_bandwidth picks the highest not above",
            bipbop(),
            VariantPolicy {
                max_bandwidth: Some(1_000_000),
                ..Default::default()
            },
            Expect::Bandwidth(915905),
        ),
        (
            "max_height filters by resolution",
            bipbop(),
            VariantPolicy {
                max_height: Some(540),
                ..Default::default()
            },
            Expect::Bandwidth(915905),
        ),
        (
            "supported_codecs keeps avc1",
            hevc(),
            codecs(["avc1", "mp4a"]),
            Expect::CodecsContain("avc1"),
        ),
        (
            "supported_codecs keeps hvc1",
            hevc(),
            codecs(["hvc1", "mp4a"]),
            Expect::CodecsContain("hvc1"),
        ),
        (
            "none fit falls back to the lowest bandwidth",
            bipbop(),
            VariantPolicy {
                max_bandwidth: Some(1),
                ..Default::default()
            },
            Expect::LowestBandwidth,
        ),
        (
            "a tie picks the first in playlist order",
            TIE_MASTER.as_bytes().to_vec(),
            VariantPolicy::default(),
            Expect::Chosen(0),
        ),
    ];
    for (label, master, policy, expect) in cases {
        let mut client = HlsClient::new("http://example.com/bipbop/master.m3u8");
        client.set_variant_policy(policy);
        client.on_playlist(&master).expect("master parses");
        let sel = match client.next_output().unwrap() {
            Output::Master(s) => s,
            o => panic!("{label}: {o:?}"),
        };
        let chosen = &sel.variants[sel.chosen];
        match expect {
            Expect::HighestBandwidth => assert_eq!(
                chosen.bandwidth,
                sel.variants.iter().map(|v| v.bandwidth).max().unwrap(),
                "{label}"
            ),
            Expect::LowestBandwidth => assert_eq!(
                chosen.bandwidth,
                sel.variants.iter().map(|v| v.bandwidth).min().unwrap(),
                "{label}"
            ),
            Expect::Bandwidth(bw) => assert_eq!(chosen.bandwidth, bw, "{label}"),
            Expect::Chosen(i) => assert_eq!(sel.chosen, i, "{label}"),
            Expect::CodecsContain(needle) => {
                let codecs = chosen.codecs.as_deref().unwrap_or("");
                assert!(codecs.contains(needle), "{label}: {needle} in {codecs}");
            }
        }
    }
}

#[test]
fn master_exposes_audio_renditions_with_resolved_urls() {
    let master_url = "http://example.com/9.6-multivariant-alternative-audio.m3u8";
    let mut client = HlsClient::new(master_url);
    let Some(bytes) = test_fixtures::require_bytes(
        "hls/spec/9.6-multivariant-alternative-audio.m3u8",
        "hls-runtime master_playlist",
    ) else {
        return;
    };
    client.on_playlist(&bytes).expect("master parses");
    let sel = match client.next_output().unwrap() {
        Output::Master(s) => s,
        o => panic!("{o:?}"),
    };
    // three audio renditions
    let audio: Vec<_> = sel
        .renditions
        .iter()
        .filter(|r| matches!(r.media_type, broadcast_hls::MediaType::Audio))
        .collect();
    assert_eq!(audio.len(), 3);
    assert_eq!(audio[0].name, "English");
    assert_eq!(audio[0].language.as_deref(), Some("en"));
    assert!(audio[0].default);
    assert!(audio[0].autoselect);
    assert_eq!(
        audio[0].url.as_deref(),
        Some("http://example.com/main/english-audio.m3u8")
    );

    assert_eq!(audio[1].name, "Deutsch");
    assert_eq!(audio[1].language.as_deref(), Some("de"));
    assert!(!audio[1].default);

    assert_eq!(audio[2].name, "Commentary");
    assert!(!audio[2].autoselect);

    // a rendition without URI has url None
    let closed: Vec<_> = sel
        .renditions
        .iter()
        .filter(|r| matches!(r.media_type, broadcast_hls::MediaType::ClosedCaptions))
        .collect();
    for r in closed {
        assert!(r.url.is_none());
    }
}

#[test]
fn relative_and_absolute_variant_uris_resolve_against_master_url() {
    // relative (bipbop)
    let mut client = HlsClient::new("http://example.com/bipbop/master.m3u8");
    let bytes = test_fixtures::read("hls/real/bipbop-ts/master.m3u8");
    client.on_playlist(&bytes).expect("master parses");
    let sel = match client.next_output().unwrap() {
        Output::Master(s) => s,
        o => panic!("{o:?}"),
    };
    assert_eq!(
        sel.variants[0].url,
        "http://example.com/bipbop/gear1/prog_index.m3u8"
    );

    // absolute (9.4)
    let mut client = HlsClient::new("http://base/playlist.m3u8");
    let Some(bytes) = test_fixtures::require_bytes(
        "hls/spec/9.4-multivariant-playlist.m3u8",
        "hls-runtime master_playlist",
    ) else {
        return;
    };
    client.on_playlist(&bytes).expect("master parses");
    let sel = match client.next_output().unwrap() {
        Output::Master(s) => s,
        o => panic!("{o:?}"),
    };
    assert_eq!(sel.variants[0].url, "http://example.com/low.m3u8");
}

#[test]
fn select_variant_switches_emits_discontinuity_and_repoints() {
    let master_url = "http://example.com/bipbop/master.m3u8";
    let mut client = HlsClient::new(master_url);
    let bytes = test_fixtures::read("hls/real/bipbop-ts/master.m3u8");
    client.on_playlist(&bytes).expect("master parses");
    let _ = client.next_output(); // Master
    client.select_variant(1).expect("select");
    let out = client.next_output().expect("Discontinuity");
    assert!(matches!(out, Output::Discontinuity));
    let action = client.poll().expect("FetchPlaylist");
    match action {
        Action::FetchPlaylist { url, .. } => {
            assert_eq!(url, "http://example.com/bipbop/gear2/prog_index.m3u8");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn select_variant_out_of_range_and_before_master_error() {
    let mut client = HlsClient::new("http://example.com/m.m3u8");
    assert!(matches!(
        client.select_variant(0),
        Err(Error::NoSuchVariant { .. })
    ));

    let bytes = test_fixtures::read("hls/real/bipbop-ts/master.m3u8");
    let mut client = HlsClient::new("http://example.com/bipbop/master.m3u8");
    client.on_playlist(&bytes).expect("master parses");
    let _ = client.next_output();
    assert!(matches!(
        client.select_variant(100),
        Err(Error::NoSuchVariant { index: 100 })
    ));
}

#[test]
fn media_playlist_input_unchanged() {
    let mut client = HlsClient::new("http://example.com/media.m3u8");
    let Some(bytes) = test_fixtures::require_bytes(
        "hls/spec/9.1-simple-media-playlist.m3u8",
        "hls-runtime master_playlist",
    ) else {
        return;
    };
    client.on_playlist(&bytes).expect("media parses");
    assert!(!matches!(client.next_output(), Some(Output::Master(_))));
}

#[test]
fn full_flow_master_to_samples() {
    let master_text = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000\nhttp://fixture/index.m3u8\n";
    let mut client = HlsClient::new("http://example.com/master.m3u8");
    client
        .on_playlist(master_text.as_bytes())
        .expect("master parses");

    // collect outputs in order
    let mut kinds: Vec<&str> = Vec::new();
    let mut sample_count = 0usize;
    // drive
    loop {
        match client.poll() {
            Some(Action::FetchPlaylist { url, .. }) => {
                if url == PLAYLIST_URL {
                    let text = read_ts_fixture("index.m3u8");
                    let text = String::from_utf8(text).expect("utf8");
                    client
                        .on_playlist(text.as_bytes())
                        .expect("variant playlist");
                } else {
                    // shouldn't happen
                    panic!("unexpected playlist url {url}");
                }
            }
            Some(Action::FetchResource { id, url, .. }) => {
                let name = url.rsplit('/').next().expect("url path");
                let bytes = read_ts_fixture(name);
                client.on_resource(id, &bytes).expect("resource");
            }
            Some(Action::WaitMs(_)) => {}
            None => break,
            Some(other) => panic!("unexpected action {other:?}"),
        }
        while let Some(output) = client.next_output() {
            match &output {
                Output::Master(_) => kinds.push("Master"),
                Output::Init(_) => kinds.push("Init"),
                Output::Samples { samples, .. } => {
                    kinds.push("Samples");
                    sample_count += samples.len();
                }
                Output::Discontinuity => kinds.push("Discontinuity"),
                Output::EndOfStream => kinds.push("EndOfStream"),
                _ => {}
            }
        }
    }

    // Master before Init before Samples
    let master_pos = kinds.iter().position(|k| *k == "Master").expect("Master");
    let init_pos = kinds.iter().position(|k| *k == "Init").expect("Init");
    let samples_pos = kinds.iter().position(|k| *k == "Samples").expect("Samples");
    assert!(master_pos < init_pos);
    assert!(init_pos < samples_pos);
    assert!(sample_count >= 1);
}
