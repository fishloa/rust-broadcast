//! Play a damaged MPEG-2 TS and print every damage report a player receives.
//!
//! A real capture is loaded, a run of its video packets is deleted, and the
//! bytes are played through one HTTP connection of a [`Session`]. Each skip
//! the session's reader tolerates is published as
//! [`broadcast_client::Event::Damage`]; this example prints each one with its
//! place in the input, the track and the media time, then reports how many
//! arrived and how many samples still played.

use broadcast_client::{Action, Config, Event, Session};
use broadcast_common::stage::Timestamp;
use broadcast_core::damage::DamagePolicy;
use http::{HeaderMap, StatusCode};
use url::Url;

const TS_PACKET: usize = 188;

fn now(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

/// The fixture with two consecutive packets of its busiest PID deleted past
/// the middle.
fn damaged_stream() -> Vec<u8> {
    let bytes = test_fixtures::read("ts/h264-aac-40s.ts");
    let pid_at =
        |offset: usize| (u16::from(bytes[offset + 1] & 0x1F) << 8) | u16::from(bytes[offset + 2]);
    let offsets: Vec<usize> = (0..bytes.len() / TS_PACKET)
        .map(|i| i * TS_PACKET)
        .filter(|&offset| bytes[offset] == 0x47)
        .collect();
    let mut counts: std::collections::BTreeMap<u16, usize> = std::collections::BTreeMap::new();
    for &offset in &offsets {
        let pid = pid_at(offset);
        if pid != 0x1FFF {
            *counts.entry(pid).or_default() += 1;
        }
    }
    let (&busiest, _) = counts.iter().max_by_key(|(_, n)| **n).expect("a PID");
    let video: Vec<usize> = offsets
        .into_iter()
        .filter(|&offset| pid_at(offset) == busiest)
        .collect();
    let victim = video[video.len() / 2];
    let mut damaged = bytes;
    damaged.drain(victim..victim + 2 * TS_PACKET);
    damaged
}

fn main() {
    let bytes = damaged_stream();
    let mut config = Config::default();
    // The default is already "report every skip and keep playing"; set it
    // explicitly so the example shows where the caller's decision lives.
    config.ts_http = config
        .ts_http
        .clone()
        .with_damage_policy(DamagePolicy::new());
    let mut session = Session::open(
        Url::parse("http://example/damaged.ts").expect("static url"),
        config,
        now(0),
    )
    .expect("open session");

    let mut played = false;
    while !played {
        let Some(action) = session.poll(now(0)) else {
            break;
        };
        if let Action::HttpRequest { conn, .. } = action {
            session.on_response(conn, StatusCode::OK, &HeaderMap::new(), now(0));
            for chunk in bytes.chunks(4096) {
                session.on_data(conn, chunk, now(0));
            }
            session.on_closed(conn, None, now(0));
            played = true;
        }
    }

    let mut reports = 0usize;
    let mut samples = 0usize;
    while let Some(event) = session.next_event() {
        match event {
            Event::Damage(damage) => {
                reports += 1;
                println!("damage: {damage}");
            }
            Event::Sample { .. } => samples += 1,
            _ => {}
        }
    }
    println!("damage_report: reports={reports} samples={samples}");
}
