//! The player level of the one damage report: a damaged MPEG-2 TS pulled over
//! HTTP reaches the session's `Event::Damage` in order with the samples around
//! it, and a caller that fails on degraded damage stops the session there with
//! the same report inside the typed error.

use super::TsHttpClient;
use crate::tshttp::Event;
use crate::tshttp::TsHttpConfig as Config;
use broadcast_common::stage::Timestamp;
use broadcast_core::damage::{Damage, DamageKind, DamagePolicy, Severity};

const TS_PACKET: usize = 188;
const CHUNK: usize = 1316;

fn ts(ms: u64) -> Timestamp {
    Timestamp::from_millis(ms)
}

fn fixture() -> Vec<u8> {
    test_fixtures::read("ts/h264-aac-40s.ts")
}

fn pid_at(bytes: &[u8], offset: usize) -> u16 {
    (u16::from(bytes[offset + 1] & 0x1F) << 8) | u16::from(bytes[offset + 2])
}

/// The fixture with two consecutive packets of its busiest (video) PID
/// deleted past the middle.
fn damaged_stream() -> Vec<u8> {
    let bytes = fixture();
    let offsets: Vec<usize> = (0..bytes.len() / TS_PACKET)
        .map(|i| i * TS_PACKET)
        .filter(|&offset| bytes[offset] == 0x47)
        .collect();
    let mut counts: std::collections::BTreeMap<u16, usize> = std::collections::BTreeMap::new();
    for &offset in &offsets {
        let pid = pid_at(&bytes, offset);
        if pid != 0x1FFF {
            *counts.entry(pid).or_default() += 1;
        }
    }
    let (&busiest, _) = counts.iter().max_by_key(|(_, n)| **n).expect("a PID");
    let of_pid: Vec<usize> = offsets
        .into_iter()
        .filter(|&offset| pid_at(&bytes, offset) == busiest)
        .collect();
    let victim = of_pid[of_pid.len() / 2];
    assert_eq!(pid_at(&bytes, victim + TS_PACKET), busiest);
    let mut damaged = bytes;
    damaged.drain(victim..victim + 2 * TS_PACKET);
    damaged
}

/// Play `bytes` through one connection and return every session event.
fn play(bytes: &[u8], config: Config) -> (Vec<Event>, TsHttpClient) {
    let mut client = TsHttpClient::with_seed("u", config, ts(0), ts(0).as_millis());
    let _ = client.poll();
    client.on_response(200, &[], ts(0));
    let _ = client.poll_event();
    for chunk in bytes.chunks(CHUNK) {
        client.feed(chunk, ts(0));
    }
    client.on_eof(ts(0));
    let mut events = Vec::new();
    while let Some(event) = client.poll_event() {
        events.push(event);
    }
    (events, client)
}

fn reports(events: &[Event]) -> Vec<&Damage> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Damage(damage) => Some(damage),
            _ => None,
        })
        .collect()
}

fn is_sample(event: &Event) -> bool {
    matches!(
        event,
        Event::Demux(ev) if matches!(ev.as_ref(), transmux::DemuxEvent::Sample { .. })
    )
}

#[test]
fn a_clean_stream_produces_no_report() {
    let (events, _) = play(&fixture(), Config::default());
    assert!(events.iter().filter(|e| is_sample(e)).count() > 50);
    assert!(
        reports(&events).is_empty(),
        "a clean stream reports nothing: {:?}",
        reports(&events)
    );
}

#[test]
fn a_damaged_stream_delivers_the_report_between_samples() {
    let (events, _) = play(&damaged_stream(), Config::default());
    let index = events
        .iter()
        .position(|event| matches!(event, Event::Damage(_)))
        .unwrap_or_else(|| panic!("no damage event: {events:?}"));
    let damage = reports(&events)[0];
    assert_eq!(damage.kind(), DamageKind::MediaUnitDropped);
    assert_eq!(damage.severity(), Severity::Degraded);
    assert!(damage.location().byte_range().is_some());
    assert!(damage.location().track().is_some());
    let media = damage.location().media_time().expect("a media time");
    assert!(media.end() >= media.start());
    assert!(
        events[..index].iter().any(is_sample),
        "samples precede the report"
    );
    assert!(
        events[index + 1..].iter().any(is_sample),
        "playback continues after the report"
    );
}

#[test]
fn failing_on_degraded_ends_the_session_with_the_same_report() {
    let damaged = damaged_stream();
    let (reported, _) = play(&damaged, Config::default());
    let expected = reports(&reported)[0].clone();

    let config =
        Config::default().with_damage_policy(DamagePolicy::new().fail_on(Severity::Degraded));
    let (events, mut client) = play(&damaged, config);

    let index = events
        .iter()
        .position(|event| matches!(event, Event::DemuxError(_)))
        .unwrap_or_else(|| panic!("no refusal event: {events:?}"));
    let Event::DemuxError(transmux::Error::Damage(error)) = &events[index] else {
        panic!("the refusal is the typed damage error: {:?}", events[index]);
    };
    assert_eq!(
        error.damage(),
        &expected,
        "the same report ends the session"
    );
    assert!(
        !events[index + 1..].iter().any(is_sample),
        "the session stops at the refusal"
    );
    // Ended: nothing more is driven, and a later chunk is not even fed.
    client.feed(&damaged, ts(1));
    client.on_eof(ts(1));
    assert!(client.poll_event().is_none());
    assert!(client.poll().is_none());
}
