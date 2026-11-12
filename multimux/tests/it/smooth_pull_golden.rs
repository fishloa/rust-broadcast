//! Sample-sequence goldens for the Smooth pull ingest session playing the
//! committed `fixtures/smooth/{av,av-low}` fixtures through an in-memory fetch.
//! `SMOOTH_PULL_GOLDEN_BLESS=<dir>` writes the traces instead of comparing.

use broadcast_common::{Stage, Timestamp};
use media_plane::ingress::{IngestSession, SessionEvent};
use multimux::source::smooth_pull::{SmoothIngestSession, SmoothInput};
use smooth_runtime::{Action, FetchFailure};
use url::Url;

const GOLDEN_BLESS_ENV: &str = "SMOOTH_PULL_GOLDEN_BLESS";
const NOT_FOUND: u16 = 404;
const ORIGIN: &str = "http://origin.test/";
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET_BASIS, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME)
    })
}

fn fixture_bytes(fixture: &str, url: &str) -> Option<Vec<u8>> {
    let rel = url.strip_prefix(ORIGIN).expect("origin url");
    let path = test_fixtures::path(&format!("smooth/{fixture}/{}", percent_decode(rel)));
    std::fs::read(path).ok()
}

fn percent_decode(s: &str) -> String {
    Url::parse(&format!("{ORIGIN}{s}"))
        .expect("url")
        .path()
        .trim_start_matches('/')
        .replace("%28", "(")
        .replace("%29", ")")
}

fn sample_trace(fixture: &str) -> String {
    let mut session = SmoothIngestSession::new(Url::parse(&format!("{ORIGIN}Manifest")).unwrap());
    let mut now = Timestamp::from_nanos(0);
    let mut out = String::new();
    loop {
        while let Some(event) = session.poll() {
            match event {
                SessionEvent::NewProgram { tracks, .. } => {
                    let ids: Vec<u32> = tracks.iter().map(|t| t.track_id.value()).collect();
                    out.push_str(&format!("tracks {ids:?}\n"));
                }
                SessionEvent::Sample {
                    track_id, sample, ..
                } => out.push_str(&format!(
                    "sample track={} dts={:?} pts={:?} dur={:?} sync={} len={} fnv={:016x}\n",
                    track_id.value(),
                    sample.dts,
                    sample.pts,
                    sample.duration,
                    sample.flags.is_sync,
                    sample.data.len(),
                    fnv1a(&sample.data),
                )),
                _ => {}
            }
        }
        let Some(action) = session.poll_transmit() else {
            match session.next_deadline() {
                Some(deadline) => {
                    now = deadline;
                    session.on_deadline(now);
                    continue;
                }
                None => break,
            }
        };
        let (id, is_manifest, url) = match action {
            Action::FetchManifest { id, url } => (id, true, url.to_string()),
            Action::FetchFragment { id, url, .. } => (id, false, url.to_string()),
            _ => panic!("unexpected action"),
        };
        let body = fixture_bytes(fixture, &url);
        let input = match body {
            None => SmoothInput::Failed(
                id,
                FetchFailure {
                    status: Some(NOT_FOUND),
                    ..FetchFailure::default()
                },
            ),
            Some(ref bytes) if is_manifest => SmoothInput::Manifest(id, bytes),
            Some(ref bytes) => SmoothInput::Fragment(id, bytes),
        };
        if let Err(e) = session.feed(input, now) {
            out.push_str(&format!("fatal {e}\n"));
            return out;
        }
    }
    assert!(session.ended());
    out.push_str("ended\n");
    out
}

fn check_golden(fixture: &str) {
    let name = format!("pull-multimux-{fixture}.txt");
    let actual = sample_trace(fixture);
    if let Ok(dir) = std::env::var(GOLDEN_BLESS_ENV) {
        std::fs::write(std::path::Path::new(&dir).join(&name), &actual).expect("write golden");
        return;
    }
    let expected = std::fs::read_to_string(test_fixtures::path(&format!("golden/smooth/{name}")))
        .expect("read golden");
    assert_eq!(actual, expected, "{name} differs from the recorded trace");
}

#[test]
fn smooth_pull_golden_av() {
    check_golden("av");
}

#[test]
fn smooth_pull_golden_av_low() {
    check_golden("av-low");
}
