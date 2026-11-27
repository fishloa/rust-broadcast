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

// ── Playlist, delta, gap, duration and fetch-loss reports ──────────────

fn simple_playlist(msn: u64, last: u64, extra: &str) -> String {
    let mut text = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{msn}\n{extra}"
    );
    for n in msn..=last {
        text.push_str(&format!("#EXTINF:2.000000,\nindex{n}.ts\n"));
    }
    text
}

fn reports(client: &mut HlsClient) -> Vec<broadcast_core::damage::Damage> {
    let mut out = Vec::new();
    for output in std::iter::from_fn(|| client.next_output()) {
        if let Output::Damage(report) = output {
            out.push(report);
        }
    }
    out
}

#[test]
fn a_dangling_segment_tag_is_reported_once_across_reloads() {
    let mut client = HlsClient::new(HlsClientConfig::new(Url::parse(URL).expect("URL")));
    drain(&mut client);
    // `EXTINF` with no URI line: the segment it would have described is lost.
    let text = format!("{}\n#EXTINF:2.0,\n", simple_playlist(0, 1, ""));
    client.on_playlist(text.as_bytes()).expect("playlist");
    let first = reports(&mut client);
    assert_eq!(first.len(), 1, "the dangling tag is reported: {first:?}");
    assert_eq!(
        first[0].kind(),
        broadcast_core::damage::DamageKind::MetadataSkipped
    );
    assert_eq!(
        first[0].severity(),
        broadcast_core::damage::Severity::Degraded,
        "a segment is lost, not just metadata"
    );

    // The same malformed tag on the next reload is the same loss: one report,
    // not one per reload.
    client.on_playlist(text.as_bytes()).expect("reload");
    assert!(
        reports(&mut client).is_empty(),
        "a repeat of the same diagnostic is not re-reported"
    );
}

#[test]
fn a_bad_program_date_time_is_reported_as_information() {
    let mut client = HlsClient::new(HlsClientConfig::new(Url::parse(URL).expect("URL")));
    drain(&mut client);
    let text = simple_playlist(0, 1, "#EXT-X-PROGRAM-DATE-TIME:not-a-timestamp\n");
    client.on_playlist(text.as_bytes()).expect("playlist");
    let reported = reports(&mut client);
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert_eq!(
        reported[0].severity(),
        broadcast_core::damage::Severity::Informational,
        "only join timing loses its anchor"
    );
}

#[test]
fn an_unmergeable_delta_is_reported_before_the_full_reload() {
    let mut client = HlsClient::new(HlsClientConfig::new(Url::parse(URL).expect("URL")));
    drain(&mut client);
    // A delta with no baseline to splice it into: the reload it stands for is
    // lost and a full playlist is requested instead.
    let delta = simple_playlist(
        3,
        4,
        "#EXT-X-SERVER-CONTROL:CAN-SKIP-UNTIL=12.0\n#EXT-X-SKIP:SKIPPED-SEGMENTS=3\n",
    );
    let error = client
        .on_playlist(delta.as_bytes())
        .expect_err("an unmergeable delta asks for a full reload");
    assert!(
        matches!(error, Error::DeltaNotMergeable { .. }),
        "{error:?}"
    );
    let reported = reports(&mut client);
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert_eq!(
        reported[0].kind(),
        broadcast_core::damage::DamageKind::MetadataSkipped
    );
    assert_eq!(reported[0].what(), "HLS delta update");
    assert!(
        drain(&mut client)
            .iter()
            .any(|action| matches!(action, Action::FetchPlaylist(request) if !request.skip)),
        "the fallback is a full reload"
    );
}

#[test]
fn a_media_sequence_gap_is_reported_with_the_first_lost_number() {
    let mut client = HlsClient::new(HlsClientConfig::new(Url::parse(URL).expect("URL")));
    drain(&mut client);
    client
        .on_playlist(simple_playlist(0, 1, "").as_bytes())
        .expect("first window");
    reports(&mut client);
    // The window jumps from 0..=1 to 5..=6: numbers 2, 3 and 4 are never
    // requested by anything.
    client
        .on_playlist(simple_playlist(5, 6, "").as_bytes())
        .expect("second window");
    let reported = reports(&mut client);
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert_eq!(
        reported[0].kind(),
        broadcast_core::damage::DamageKind::MediaUnitDropped
    );
    assert_eq!(reported[0].what(), "HLS media sequence");
    assert_eq!(
        reported[0].location().ordinal(),
        Some(2),
        "the report names the first lost sequence number"
    );
}

#[test]
fn an_extinf_that_disagrees_with_the_media_is_reported_only_with_a_tolerance() {
    fn rewritten(multiplier: &str) -> String {
        let text = String::from_utf8(playlist()).expect("utf8");
        text.replace("#EXTINF:1.000000,", multiplier)
    }

    // No tolerance: the client never compares, so nothing is reported.
    let mut client = start(broadcast_core::damage::DamagePolicy::new());
    let mut damage = Vec::new();
    absorb(&mut client, &mut damage);
    client
        .on_resource(segment(0), &test_fixtures::read(FIRST))
        .expect("first segment");
    absorb(&mut client, &mut damage);
    let mut rewritten_client = HlsClient::new(HlsClientConfig::new(Url::parse(URL).expect("URL")));
    drain(&mut rewritten_client);
    rewritten_client
        .on_playlist(rewritten("#EXTINF:40.000000,").as_bytes())
        .expect("playlist");
    drain(&mut rewritten_client);
    assert!(
        reports(&mut rewritten_client).is_empty(),
        "without a caller tolerance nothing is compared"
    );

    // With a 100 ms tolerance the 1 s segment that claims 40 s is reported.
    let mut tolerant = HlsClient::new(
        HlsClientConfig::new(Url::parse(URL).expect("URL"))
            .with_segment_duration_tolerance(Some(core::time::Duration::from_millis(100))),
    );
    drain(&mut tolerant);
    tolerant
        .on_playlist(rewritten("#EXTINF:40.000000,").as_bytes())
        .expect("playlist");
    drain(&mut tolerant);
    tolerant
        .on_resource(segment(0), &test_fixtures::read(FIRST))
        .expect("first segment");
    reports(&mut tolerant);
    tolerant
        .on_resource(segment(1), &test_fixtures::read(SECOND))
        .expect("second segment");
    let reported = reports(&mut tolerant);
    assert_eq!(reported.len(), 1, "the drifted segment is reported once");
    assert_eq!(
        reported[0].kind(),
        broadcast_core::damage::DamageKind::MediaUnitDamaged
    );
    assert_eq!(reported[0].what(), "HLS segment duration");
    assert_eq!(reported[0].location().ordinal(), Some(1));
    assert!(reported[0].location().media_time().is_some());
}

#[test]
fn a_segment_given_up_on_is_reported_with_its_reason() {
    let mut client = start(broadcast_core::damage::DamagePolicy::new());
    drain(&mut client);
    client
        .skip_resource_due_to(segment(0), hls_runtime::client::SkipReason::NotFound)
        .expect("the report is published");
    let reported = reports(&mut client);
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert_eq!(
        reported[0].kind(),
        broadcast_core::damage::DamageKind::MediaUnitDropped
    );
    assert_eq!(reported[0].what(), "HLS segment (not found)");
    assert_eq!(reported[0].location().ordinal(), Some(0));
}

#[test]
fn a_given_up_segment_stops_a_failing_policy() {
    let mut client = start(
        broadcast_core::damage::DamagePolicy::new()
            .fail_on(broadcast_core::damage::Severity::Degraded),
    );
    let error = client
        .skip_resource_due_to(segment(0), hls_runtime::client::SkipReason::TimedOut)
        .expect_err("the policy stops the pull");
    let Error::Damage(error) = error else {
        panic!("expected the policy stop, got {error}");
    };
    assert_eq!(error.damage().what(), "HLS segment (timed out)");
}
