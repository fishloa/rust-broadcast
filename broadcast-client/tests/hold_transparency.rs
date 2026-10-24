mod support;

use broadcast_client::Config;
use support::replay::{bitrate_bps, replay_paced, replay_with_host};

const CHUNK: usize = 32 * 1024;
const MAX_BYTES: usize = 3 * 1024 * 1024;
const MIN_FIXTURES: usize = 20;
const WALLS_US: [u64; 4] = [0, 300_000, 1_000_000, 3_000_000];

fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(test_fixtures::root().join("ts")).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_some_and(|e| e == "ts") {
            paths.push(p);
        }
    }
    paths.extend(test_fixtures::private_files("", &["ts"]));
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .filter_map(|p| {
            let mut b = std::fs::read(&p).ok()?;
            b.truncate(MAX_BYTES / 188 * 188);
            (b.first() == Some(&0x47)).then(|| (p.display().to_string(), b))
        })
        .collect()
}

fn with_wall(w: u64) -> Config {
    Config {
        tracks_hold_max_wall_us: w,
        ..Config::default()
    }
}

#[test]
fn the_hold_never_changes_the_sample_sequence() {
    let corpus = corpus();
    let mut checked = 0;
    let mut failures = Vec::new();
    for (name, bytes) in &corpus {
        let bps = bitrate_bps(bytes);
        let base = replay_paced(bytes, with_wall(0), CHUNK, bps).samples();
        if base.is_empty() {
            continue;
        }
        checked += 1;
        for w in &WALLS_US[1..] {
            let held = replay_paced(bytes, with_wall(*w), CHUNK, bps).samples();
            if base.len() != held.len() {
                failures.push(format!(
                    "{name} wall {w}: {} vs {} samples",
                    base.len(),
                    held.len()
                ));
            } else if let Some(i) = base.iter().zip(&held).position(|(a, b)| a != b) {
                failures.push(format!(
                    "{name} wall {w}: sample {i} differs: {:?} vs {:?}",
                    base[i], held[i]
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    if test_fixtures::private_root().is_some() {
        assert!(
            checked >= MIN_FIXTURES,
            "only {checked} fixtures produced samples"
        );
    }
}

const HOST_TICK_US: u64 = 20_000;
const LIVE_CAPTURES: [&str; 4] = [
    "zenith-live/rai-gulp-8MB.ts",
    "zenith-live/rai-news-24-8MB.ts",
    "zenith-live/arte-8MB.ts",
    "zenith-live/tf-1-8MB.ts",
];

#[test]
fn a_host_playing_from_start_never_sees_a_late_sample_whatever_the_hold() {
    for rel in LIVE_CAPTURES {
        let Some(bytes) = test_fixtures::require_bytes(rel, "hold_transparency") else {
            continue;
        };
        let bps = bitrate_bps(&bytes);
        for w in WALLS_US {
            let (replay, host) =
                replay_with_host(&bytes, with_wall(w), CHUNK, bps, Some(HOST_TICK_US));
            let first_tracks = replay.tracks_times().first().copied();
            if let (Some(start), Some(tracks)) = (host.start_at_us, first_tracks) {
                assert!(start >= tracks, "{rel} wall {w}: Start before Tracks");
            }
            assert_eq!(host.late_drops, 0, "{rel} wall {w}: {host:?}");
        }
    }
}
