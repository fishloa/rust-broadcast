//! Damage reporting through the HLS client: a segment with a damaged run of
//! TS packets reaches the player as a report, in order with the samples around
//! it, and the caller's policy decides whether the report stops the pull.

use hls_runtime::client::{Action, Error, HlsClient, HlsClientConfig, Output, ResourceId};
use url::Url;

const URL: &str = "http://o/v/index.m3u8";
const PLAYLIST: &str = "hls/ts-hls/index.m3u8";
const FIRST: &str = "hls/ts-hls/index0.ts";
const SECOND: &str = "hls/ts-hls/index1.ts";
/// TS packet size (ISO/IEC 13818-1 §2.4.3.2).
const TS_PACKET_BYTES: usize = 188;
/// The packet run the damaged copy destroys (its sync bytes included).
const DAMAGED_RUN: core::ops::Range<usize> = 5..15;

fn playlist() -> Vec<u8> {
    test_fixtures::read(PLAYLIST)
}

fn damaged(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let start = DAMAGED_RUN.start * TS_PACKET_BYTES;
    let end = (DAMAGED_RUN.end * TS_PACKET_BYTES).min(out.len());
    for byte in &mut out[start..end] {
        *byte = 0x00;
    }
    out
}

fn drain(client: &mut HlsClient) -> Vec<Action> {
    std::iter::from_fn(|| client.poll()).collect()
}

fn start(policy: broadcast_core::damage::DamagePolicy) -> HlsClient {
    let mut client = HlsClient::new(
        HlsClientConfig::new(Url::parse(URL).expect("URL")).with_damage_policy(policy),
    );
    drain(&mut client);
    client.on_playlist(&playlist()).expect("playlist");
    drain(&mut client);
    client
}

fn absorb(client: &mut HlsClient, damage: &mut Vec<broadcast_core::damage::Damage>) -> usize {
    let mut samples = 0usize;
    for output in std::iter::from_fn(|| client.next_output()) {
        match output {
            Output::Damage(report) => damage.push(report),
            Output::Samples { samples: got, .. } => samples = samples.saturating_add(got.len()),
            _ => {}
        }
    }
    samples
}

fn segment(msn: u64) -> ResourceId {
    ResourceId::Segment {
        msn: broadcast_hls::MediaSequence::new(msn),
    }
}

#[test]
fn a_damaged_packet_run_is_reported_and_playback_continues() {
    let mut client = start(broadcast_core::damage::DamagePolicy::new());
    let mut damage = Vec::new();
    let mut samples = absorb(&mut client, &mut damage);

    client
        .on_resource(segment(0), &test_fixtures::read(FIRST))
        .expect("first segment");
    samples = samples.saturating_add(absorb(&mut client, &mut damage));
    client
        .on_resource(segment(1), &damaged(&test_fixtures::read(SECOND)))
        .expect("a damaged segment must not end the pull");
    samples = samples.saturating_add(absorb(&mut client, &mut damage));

    assert!(
        !damage.is_empty(),
        "the damaged packet run must be reported"
    );
    assert!(
        damage.iter().any(|report| matches!(
            report.kind(),
            broadcast_core::damage::DamageKind::Resynchronised
                | broadcast_core::damage::DamageKind::MediaUnitDropped
                | broadcast_core::damage::DamageKind::InputTruncated
        )),
        "{damage:?}"
    );
    assert!(
        damage
            .iter()
            .all(|report| { report.severity() == broadcast_core::damage::Severity::Degraded })
    );
    assert!(samples > 0, "playback continued past the damaged run");
}

#[test]
fn the_fail_policy_ends_the_same_pull_with_the_same_report() {
    let mut client = start(
        broadcast_core::damage::DamagePolicy::new()
            .fail_on(broadcast_core::damage::Severity::Degraded),
    );
    let mut damage = Vec::new();
    absorb(&mut client, &mut damage);
    client
        .on_resource(segment(0), &test_fixtures::read(FIRST))
        .expect("first segment");
    absorb(&mut client, &mut damage);
    let error = client
        .on_resource(segment(1), &damaged(&test_fixtures::read(SECOND)))
        .expect_err("the policy stops the pull");
    let Error::Damage(error) = error else {
        panic!("expected the policy stop, got {error}");
    };
    assert_eq!(
        error.damage().severity(),
        broadcast_core::damage::Severity::Degraded
    );
    assert!(
        damage
            .iter()
            .all(|report| report.severity() != broadcast_core::damage::Severity::Degraded)
    );
}

#[test]
fn a_clean_pull_reports_nothing() {
    let mut client = start(broadcast_core::damage::DamagePolicy::new());
    let mut damage = Vec::new();
    absorb(&mut client, &mut damage);
    client
        .on_resource(segment(0), &test_fixtures::read(FIRST))
        .expect("first segment");
    absorb(&mut client, &mut damage);
    client
        .on_resource(segment(1), &test_fixtures::read(SECOND))
        .expect("second segment");
    absorb(&mut client, &mut damage);
    assert!(
        damage.is_empty(),
        "a clean playlist produces no report: {damage:?}"
    );
}
