use smooth_runtime::{Action, Output, SmoothClient, SmoothClientConfig};
use url::Url;

use crate::common::{Host, Reply, fixture, is_ended, is_fatal, now};

#[test]
fn the_lifecycle_accessors_track_a_static_presentation() {
    let mut host = Host::new(SmoothClientConfig::default());
    host.serve_dir("av");
    assert!(!host.client.is_started());
    assert!(!host.client.is_live());
    assert!(!host.client.is_ended());
    assert!(!host.client.is_fatal());
    host.run_until(|o| o.iter().any(is_ended));
    assert!(host.client.is_started());
    assert!(host.client.is_ended());
    assert!(!host.client.is_live());
    assert!(!host.client.is_fatal());
    assert_eq!(host.client.next_deadline(), None);
    assert!(host.client.poll(now(host.now)).is_none());
    assert!(host.client.next_output().is_none());
}

#[test]
fn a_live_presentation_reports_live_and_never_ends() {
    let mut host = Host::new(SmoothClientConfig::default());
    host.serve_dir("av");
    let live = String::from_utf8(fixture("av/Manifest"))
        .expect("utf8")
        .replace(
            "Duration=\"30415941\"",
            "Duration=\"30415941\" IsLive=\"TRUE\"",
        );
    host.route("Manifest", vec![Reply::Body(live.into_bytes())]);
    host.run_until(|o| o.iter().any(|o| matches!(o, Output::Tracks(_))));
    assert!(host.client.is_started());
    assert!(host.client.is_live());
    assert!(!host.client.is_ended());
}

#[test]
fn a_fatal_construction_reports_fatal_without_starting() {
    let cap = broadcast_common::backoff::MAX_FLOOR + std::time::Duration::from_secs(1);
    let mut host = Host::new(SmoothClientConfig::default().with_backoff(
        broadcast_common::backoff::BackoffSteps {
            steps: vec![std::time::Duration::from_millis(1_000)],
            cap,
        },
    ));
    host.run_until(|o| o.iter().any(is_fatal));
    assert!(host.client.is_fatal());
    assert!(!host.client.is_started());
    assert!(!host.client.is_live());
}

#[test]
fn a_new_client_has_nothing_to_emit_before_its_manifest() {
    let mut client = SmoothClient::new(SmoothClientConfig::default());
    assert!(matches!(
        client.poll(now(0)),
        Some(Action::FetchManifest { .. })
    ));
    assert!(client.next_output().is_none());
}

#[test]
fn set_manifest_url_rebases_the_next_manifest_fetch() {
    let mut client = SmoothClient::new(SmoothClientConfig::default());
    let rebased = Url::parse("http://edge.test/live/Manifest").expect("url");
    client.set_manifest_url(rebased.clone());
    match client.poll(now(0)) {
        Some(Action::FetchManifest { url, .. }) => assert_eq!(url, rebased),
        other => panic!("expected a manifest fetch, got {other:?}"),
    }
}
