use std::collections::BTreeSet;

use broadcast_client::{Config, Event};

use crate::support::replay::{Replay, bitrate_bps, replay_paced};

const CHUNK: usize = 32 * 1024;
const PRIVATE_MPTS: &str = "ts/france-tnt-uhf32-video-head-6-mb.ts";

fn run(bytes: &[u8], config: Config) -> Replay {
    replay_paced(bytes, config, CHUNK, bitrate_bps(bytes))
}

fn programs_of(replay: &Replay) -> BTreeSet<Option<u16>> {
    replay
        .events
        .iter()
        .filter_map(|t| match &t.event {
            Event::Tracks(specs) => Some(specs),
            _ => None,
        })
        .flatten()
        .map(|spec| spec.program_number.map(|p| p.value()))
        .collect()
}

#[test]
fn relay_emits_the_same_samples_as_the_default_and_no_later() {
    let bytes = test_fixtures::read("ts/h264-aac-40s.ts");
    let default = run(&bytes, Config::default());
    let relay = run(&bytes, Config::relay());
    assert_eq!(relay.samples(), default.samples());
    assert!(relay.first_sample_us() <= default.first_sample_us());
}

#[test]
fn relay_announces_tracks_without_waiting_for_the_hold() {
    let bytes = test_fixtures::read("ts/france2.ts");
    let default = run(&bytes, Config::default());
    let relay = run(&bytes, Config::relay());
    assert!(!relay.tracks_times().is_empty());
    assert!(relay.tracks_times()[0] <= default.tracks_times()[0]);
    assert_eq!(relay.samples(), default.samples());
}

#[test]
fn relay_tracks_keep_their_program_number_for_every_programme_of_a_multiplex() {
    let Some(bytes) = test_fixtures::require_bytes(PRIVATE_MPTS, "relay_mode") else {
        return;
    };
    let relay = run(&bytes, Config::relay());
    let programs = programs_of(&relay);
    assert!(!programs.contains(&None), "{programs:?}");
    assert!(
        programs.len() > 1,
        "a multiplex has several programmes: {programs:?}"
    );
}
