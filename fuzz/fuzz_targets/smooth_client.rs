#![no_main]

use std::sync::OnceLock;
use broadcast_common::stage::Timestamp;

use libfuzzer_sys::fuzz_target;
use smooth_runtime::{Action, FetchFailure, SmoothClient, SmoothConfig};
use url::Url;

const STEP_BUDGET: usize = 2_000;
const MODE_ARBITRARY_MANIFEST: u8 = 0b001;
const MODE_LIVE: u8 = 0b010;
const BODY_SHIFT: u32 = 2;
const BODY_MODES: u8 = 4;
const MANIFEST_PATH: &str = "smooth/av/Manifest";
const VIDEO_PATH: &str = "smooth/av/QualityLevels(64528)/Fragments(video=0)";
const AUDIO_PATH: &str = "smooth/av/QualityLevels(98429)/Fragments(audio=0)";
const LIVE_ATTRS: &str = "Duration=\"30415941\" IsLive=\"TRUE\" DVRWindowLength=\"20000000\"";
const STATIC_ATTRS: &str = "Duration=\"30415941\"";
const NOT_FOUND: u16 = 404;

struct Corpus {
    manifest: String,
    video: Vec<u8>,
    audio: Vec<u8>,
}

fn corpus() -> &'static Corpus {
    static CORPUS: OnceLock<Corpus> = OnceLock::new();
    CORPUS.get_or_init(|| Corpus {
        manifest: String::from_utf8(test_fixtures::read(MANIFEST_PATH)).unwrap_or_default(),
        video: test_fixtures::read(VIDEO_PATH),
        audio: test_fixtures::read(AUDIO_PATH),
    })
}

fuzz_target!(|data: &[u8]| {
    let corpus = corpus();
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    let (manifest, noise) = if mode & MODE_ARBITRARY_MANIFEST != 0 {
        let cut = rest.len() / 2;
        (rest[..cut].to_vec(), &rest[cut..])
    } else if mode & MODE_LIVE != 0 {
        (
            corpus
                .manifest
                .replace(STATIC_ATTRS, LIVE_ATTRS)
                .into_bytes(),
            rest,
        )
    } else {
        (corpus.manifest.as_bytes().to_vec(), rest)
    };
    let Ok(url) = Url::parse("http://origin.test/Manifest") else {
        return;
    };
    let mut client = SmoothClient::new(url, SmoothConfig::default());
    let mut now = 0u64;
    for _ in 0..STEP_BUDGET {
        while let Some(action) = client.poll(Timestamp::from_micros(now)) {
            match action {
                Action::FetchManifest { id, .. } => client.on_manifest(Timestamp::from_micros(now), id, &manifest),
                Action::FetchFragment { id, stream, .. } => {
                    let base = if stream.0 == 0 {
                        &corpus.video
                    } else {
                        &corpus.audio
                    };
                    match (mode >> BODY_SHIFT) % BODY_MODES {
                        0 => client.on_fragment(Timestamp::from_micros(now), id, base),
                        1 => client.on_fragment(Timestamp::from_micros(now), id, noise),
                        2 => client.on_failure(
                            Timestamp::from_micros(now),
                            id,
                            FetchFailure {
                                status: Some(NOT_FOUND),
                                ..FetchFailure::default()
                            },
                        ),
                        _ => {
                            let keep = noise.first().map_or(0, |&b| usize::from(b)).min(base.len());
                            client.on_fragment(Timestamp::from_micros(now), id, &base[..keep]);
                        }
                    }
                }
                _ => {}
            }
        }
        while client.next_output().is_some() {}
        if client.is_fatal() || client.is_ended() {
            return;
        }
        match client.next_deadline() {
            Some(t) => now = now.max(t),
            None => return,
        }
    }
});
