mod support;

use broadcast_client::Config;
use support::replay::{bitrate_bps, replay_paced, replay_with_host};

const CHUNK: usize = 32 * 1024;
const MAX_BYTES: usize = 3 * 1024 * 1024;
const MIN_FIXTURES: usize = 20;
const WALLS_US: [u64; 3] = [0, 300_000, 3_000_000];
const SOAK_WALLS_US: [u64; 4] = [0, 300_000, 1_000_000, 3_000_000];

fn walls() -> &'static [u64] {
    if std::env::var_os("RUST_BROADCAST_SOAK").is_some() {
        &SOAK_WALLS_US
    } else {
        &WALLS_US
    }
}

fn public_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(test_fixtures::root().join("ts")).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_some_and(|e| e == "ts") {
            paths.push(p);
        }
    }
    paths
}

fn corpus(mut paths: Vec<std::path::PathBuf>) -> Vec<(String, Vec<u8>)> {
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
    {
        let mut new_config = Config::default();
        new_config.tracks_hold_max_wall_us = w;
        new_config
    }
}

fn the_hold_never_changes_the_sample_sequence(corpus: Vec<(String, Vec<u8>)>) -> usize {
    let walls = walls();
    let mut checked = 0;
    let mut failures = Vec::new();
    for (name, bytes) in &corpus {
        let bps = bitrate_bps(bytes);
        let base = replay_paced(bytes, with_wall(walls[0]), CHUNK, bps).samples();
        if base.is_empty() {
            continue;
        }
        checked += 1;
        for w in &walls[1..] {
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
    checked
}

#[test]
fn the_hold_never_changes_the_sample_sequence_on_public_captures() {
    let checked = the_hold_never_changes_the_sample_sequence(corpus(public_paths()));
    assert!(checked > 0, "no public capture produced samples");
}

const PRIVATE_SHARDS: usize = 4;

fn private_shard(shard: usize) {
    let paths: Vec<_> = test_fixtures::private_files("", &["ts"])
        .into_iter()
        .enumerate()
        .filter(|(i, _)| i % PRIVATE_SHARDS == shard)
        .map(|(_, p)| p)
        .collect();
    let checked = the_hold_never_changes_the_sample_sequence(corpus(paths));
    if test_fixtures::private_root().is_some() {
        assert!(checked > 0, "shard {shard}: no fixture produced samples");
    }
}

macro_rules! private_shard {
    ($test:ident, $shard:expr) => {
        #[test]
        fn $test() {
            private_shard($shard);
        }
    };
}

private_shard!(
    the_hold_never_changes_the_sample_sequence_on_private_shard_0,
    0
);
private_shard!(
    the_hold_never_changes_the_sample_sequence_on_private_shard_1,
    1
);
private_shard!(
    the_hold_never_changes_the_sample_sequence_on_private_shard_2,
    2
);
private_shard!(
    the_hold_never_changes_the_sample_sequence_on_private_shard_3,
    3
);

#[test]
fn the_private_corpus_is_complete() {
    if test_fixtures::private_root().is_none() {
        return;
    }
    let readable = corpus(test_fixtures::private_files("", &["ts"])).len();
    assert!(
        readable >= MIN_FIXTURES,
        "only {readable} private fixtures are readable"
    );
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
        for &w in walls() {
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
