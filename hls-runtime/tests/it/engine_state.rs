//! Audit 21 finding 55 (issue #2950): the scheduling accessors, variant
//! choice at its bandwidth boundary, origin-restart detection and the
//! preload-hint prefetch, driven purely through the public API.
use broadcast_common::range::ByteSpan;
use broadcast_hls::MediaSequence;
use core::num::NonZeroU64;
use hls_runtime::client::{
    Action, HlsClient, HlsClientConfig, Output, ResourceId, VariantInfo, VariantPolicy,
    choose_variant,
};
use url::Url;

const URL: &str = "http://origin/live/media.m3u8";

fn config() -> HlsClientConfig {
    HlsClientConfig::new(Url::parse(URL).expect("URL"))
}

fn playlist(first: u64, count: u64, target: u32, endlist: bool) -> String {
    let mut text = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:{target}\n\
         #EXT-X-MEDIA-SEQUENCE:{first}\n"
    );
    for msn in first..first + count {
        text.push_str(&format!("#EXTINF:2.0,\nseg{msn}.ts\n"));
    }
    if endlist {
        text.push_str("#EXT-X-ENDLIST\n");
    }
    text
}

/// A window whose segments carry `EXT-X-PROGRAM-DATE-TIME`, `start_secs`
/// after 2026-10-02T00:00:00Z, two seconds apart.
fn pdt_playlist(first: u64, count: u64, start_secs: u64, target: u32) -> String {
    let mut text = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:{target}\n\
         #EXT-X-MEDIA-SEQUENCE:{first}\n"
    );
    for (i, msn) in (first..first + count).enumerate() {
        let secs = start_secs + 2 * i as u64;
        let hh = secs / 3600;
        let mm = (secs % 3600) / 60;
        let ss = secs % 60;
        text.push_str(&format!(
            "#EXT-X-PROGRAM-DATE-TIME:2026-10-02T{hh:02}:{mm:02}:{ss:02}Z\n\
             #EXTINF:2.0,\nseg{msn}.ts\n"
        ));
    }
    text
}

fn fetched_ids(client: &mut HlsClient) -> Vec<ResourceId> {
    let mut ids = Vec::new();
    while let Some(action) = client.poll() {
        if let Action::FetchResource { id, .. } = action {
            ids.push(id);
        }
    }
    ids
}

fn fetched(client: &mut HlsClient) -> Vec<(ResourceId, Url, Option<ByteSpan>)> {
    let mut out = Vec::new();
    while let Some(action) = client.poll() {
        if let Action::FetchResource {
            id,
            url,
            byte_range,
        } = action
        {
            out.push((id, url, byte_range));
        }
    }
    out
}

fn drain_outputs(client: &mut HlsClient) -> Vec<Output> {
    std::iter::from_fn(|| client.next_output()).collect()
}

#[test]
fn endlist_and_target_duration_report_the_last_full_playlist() {
    let mut client = HlsClient::new(config());
    assert!(!client.endlist_seen(), "no playlist processed yet");
    assert_eq!(client.target_duration_secs(), None);

    client
        .on_playlist(playlist(0, 2, 6, true).as_bytes())
        .expect("VOD playlist parses");
    assert!(client.endlist_seen());
    assert_eq!(client.target_duration_secs(), Some(6));

    let mut live = HlsClient::new(config());
    live.on_playlist(playlist(0, 2, 6, false).as_bytes())
        .expect("live playlist parses");
    assert!(!live.endlist_seen());
    assert_eq!(live.target_duration_secs(), Some(6));
}

#[test]
fn ensure_playlist_reload_is_true_exactly_when_no_reload_is_queued() {
    let mut client = HlsClient::new(config());
    assert!(
        matches!(client.poll(), Some(Action::FetchPlaylist(_))),
        "a new client seeds one playlist fetch"
    );
    assert!(client.ensure_playlist_reload(), "nothing queued");
    assert!(
        !client.ensure_playlist_reload(),
        "a reload is already queued, so no second one is added"
    );
    assert!(matches!(client.poll(), Some(Action::FetchPlaylist(_))));
    assert!(
        client.poll().is_none(),
        "ensure_playlist_reload queued exactly one fetch"
    );
}

#[test]
fn set_playlist_base_governs_relative_segment_resolution() {
    let mut client = HlsClient::new(config());
    let _ = fetched_ids(&mut client);
    client.set_playlist_base(Url::parse("http://cdn.example/edge/").expect("URL"));
    client
        .on_playlist(playlist(0, 1, 2, true).as_bytes())
        .expect("playlist parses");
    let requests = fetched(&mut client);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].1.as_str(), "http://cdn.example/edge/seg0.ts");
}

#[test]
fn choose_variant_treats_the_bandwidth_cap_as_inclusive() {
    let variants = [
        VariantInfo::new(None, 1_000_000),
        VariantInfo::new(None, 2_000_000),
        VariantInfo::new(None, 4_000_000),
    ];
    let cap =
        |bits: u64| VariantPolicy::default().with_max_bandwidth(u32::try_from(bits).expect("fits"));
    assert_eq!(
        choose_variant(&variants, &cap(2_000_000)),
        1,
        "a variant at exactly max_bandwidth is allowed"
    );
    assert_eq!(
        choose_variant(&variants, &cap(1_999_999)),
        0,
        "one bit over the cap is not"
    );
    assert_eq!(choose_variant(&variants, &cap(4_000_000)), 2);

    let none_allowed = [
        VariantInfo::new(None, 4_000_000),
        VariantInfo::new(None, 4_000_000),
    ];
    assert_eq!(
        choose_variant(&none_allowed, &cap(1)),
        0,
        "with nothing allowed the lowest-bandwidth (first) variant is chosen"
    );
}

#[test]
fn a_preload_hint_of_type_map_prefetches_that_init() {
    let text = "#EXTM3U\n#EXT-X-VERSION:9\n#EXT-X-TARGETDURATION:2\n\
#EXT-X-MEDIA-SEQUENCE:0\n\
#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5\n\
#EXT-X-PART-INF:PART-TARGET=0.5\n\
#EXTINF:2.0,\nseg0.m4s\n\
#EXT-X-PRELOAD-HINT:TYPE=MAP,URI=\"init.mp4\"\n";
    let mut client = HlsClient::new(config());
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(text.as_bytes())
        .expect("playlist parses");
    let requests = fetched(&mut client);
    assert!(
        requests
            .iter()
            .any(|(id, url, _)| *id == ResourceId::Init
                && url.as_str() == "http://origin/live/init.mp4"),
        "a TYPE=MAP preload hint must request that init: {requests:?}"
    );
}

#[test]
fn a_preload_hint_part_carries_its_byte_range() {
    let text = "#EXTM3U\n#EXT-X-VERSION:9\n#EXT-X-TARGETDURATION:2\n\
#EXT-X-MEDIA-SEQUENCE:0\n\
#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5\n\
#EXT-X-PART-INF:PART-TARGET=0.5\n\
#EXTINF:2.0,\nseg0.m4s\n\
#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"seg1.0.m4s\",BYTERANGE-START=100,BYTERANGE-LENGTH=500\n";
    let mut client = HlsClient::new(config());
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(text.as_bytes())
        .expect("playlist parses");
    let requests = fetched(&mut client);
    let hinted = requests
        .iter()
        .find(|(id, _, _)| matches!(id, ResourceId::Part { .. }))
        .expect("the hinted part is requested");
    assert_eq!(
        hinted.2,
        Some(ByteSpan::new(100, NonZeroU64::new(500).expect("non-zero"))),
        "BYTERANGE-START/LENGTH of the preload hint"
    );
}

/// A media sequence number going backwards restarts the stream exactly once:
/// one `Discontinuity` output and the new window fetched; an equal number
/// does not restart.
#[test]
fn a_media_sequence_regression_restarts_once_and_an_equal_number_does_not() {
    let mut client = HlsClient::new(config());
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(playlist(100, 2, 2, false).as_bytes())
        .expect("playlist parses");
    let _ = fetched_ids(&mut client);
    let _ = drain_outputs(&mut client);

    client
        .on_playlist(playlist(10, 2, 2, false).as_bytes())
        .expect("a regressed playlist still parses");
    let outputs = drain_outputs(&mut client);
    assert_eq!(
        outputs
            .iter()
            .filter(|o| matches!(o, Output::Discontinuity))
            .count(),
        1,
        "exactly one Discontinuity for the restart: {outputs:?}"
    );
    assert_eq!(
        fetched_ids(&mut client),
        vec![
            ResourceId::Segment {
                msn: MediaSequence::new(10)
            },
            ResourceId::Segment {
                msn: MediaSequence::new(11)
            },
        ]
    );

    let _ = drain_outputs(&mut client);
    client
        .on_playlist(playlist(10, 2, 2, false).as_bytes())
        .expect("the same window parses");
    assert!(
        !drain_outputs(&mut client)
            .iter()
            .any(|o| matches!(o, Output::Discontinuity)),
        "an unchanged window is not a restart"
    );
}

/// A wholly older window (its PROGRAM-DATE-TIME ends before the previous
/// window begins) is skipped, never delivered behind what was already played.
#[test]
fn a_wholly_older_window_is_skipped_not_fetched() {
    let mut client = HlsClient::new(config());
    let _ = fetched_ids(&mut client);
    client
        .on_playlist(pdt_playlist(100, 2, 12 * 3600, 2).as_bytes())
        .expect("playlist parses");
    let _ = fetched_ids(&mut client);

    let _ = drain_outputs(&mut client);
    client
        .on_playlist(pdt_playlist(10, 2, 10 * 3600, 2).as_bytes())
        .expect("an older window parses");
    assert_eq!(
        fetched_ids(&mut client),
        Vec::new(),
        "every segment of an older window is below the lag floor"
    );
    assert!(
        !drain_outputs(&mut client)
            .iter()
            .any(|o| matches!(o, Output::Discontinuity)),
        "an older window is a lagging copy, not a restart"
    );
}
