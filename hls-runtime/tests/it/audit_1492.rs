use hls_runtime::client::{Action, Error, HlsClient, Output};

const URL: &str = "http://origin/live/media.m3u8";

fn drain(client: &mut HlsClient) -> Vec<Action> {
    std::iter::from_fn(|| client.poll()).collect()
}

fn drain_outputs(client: &mut HlsClient) -> Vec<Output> {
    std::iter::from_fn(|| client.next_output()).collect()
}

fn has_discontinuity(outputs: &[Output]) -> bool {
    outputs.iter().any(|o| matches!(o, Output::Discontinuity))
}

fn key_urls(actions: &[Action]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::FetchKey { url, .. } => Some(url.clone()),
            _ => None,
        })
        .collect()
}

fn window(first: u64, entries: &[(&str, u32)]) -> String {
    let mut text = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{first}\n"
    );
    for (name, token) in entries {
        text.push_str(&format!("#EXTINF:2.0,\n{name}?token={token}\n"));
    }
    text
}

fn second_window_outputs(first: &str, second: &str) -> Vec<Output> {
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    client.on_playlist(first.as_bytes()).unwrap();
    drain(&mut client);
    drain_outputs(&mut client);
    client.on_playlist(second.as_bytes()).unwrap();
    drain_outputs(&mut client)
}

#[test]
fn a_window_where_every_shared_segment_is_re_signed_is_not_a_restart() {
    let outputs = second_window_outputs(
        &window(100, &[("a.ts", 1), ("b.ts", 1), ("c.ts", 1)]),
        &window(101, &[("b.ts", 2), ("c.ts", 2), ("d.ts", 2)]),
    );
    assert!(!has_discontinuity(&outputs), "{outputs:?}");
}

#[test]
fn a_window_where_only_some_shared_segments_differ_by_query_is_a_restart() {
    let outputs = second_window_outputs(
        &window(100, &[("a.ts", 1), ("b.ts", 1), ("c.ts", 1)]),
        &window(101, &[("b.ts", 1), ("c.ts", 2), ("d.ts", 2)]),
    );
    assert!(has_discontinuity(&outputs), "{outputs:?}");
}

fn map_playlist(key_lines: &str) -> String {
    format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
         #EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n{key_lines}\
         #EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:2.0,\nseg0.m4s\n"
    )
}

#[test]
fn a_later_identity_key_line_replaces_the_earlier_key_for_the_init() {
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    client
        .on_playlist(
            map_playlist("#EXT-X-KEY:METHOD=AES-128,URI=\"k2\",KEYFORMAT=\"identity\"\n")
                .as_bytes(),
        )
        .unwrap();
    assert_eq!(key_urls(&drain(&mut client)), ["http://origin/live/k2"]);
}

#[test]
fn a_non_identity_key_line_leaves_the_identity_key_in_effect() {
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    client
        .on_playlist(
            map_playlist(
                "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"k2\",KEYFORMAT=\"com.apple.streamingkeydelivery\"\n",
            )
            .as_bytes(),
        )
        .unwrap();
    assert_eq!(
        key_urls(&drain(&mut client)),
        ["http://origin/live/key.bin"]
    );
}

#[test]
fn an_unusable_identity_key_line_fails_the_playlist_instead_of_keeping_the_earlier_key() {
    for bad in [
        "#EXT-X-KEY:METHOD=AES-128\n",
        "#EXT-X-KEY:METHOD=BOGUS,URI=\"x\"\n",
    ] {
        let mut client = HlsClient::new(URL);
        drain(&mut client);
        let result = client.on_playlist(map_playlist(bad).as_bytes());
        assert!(
            matches!(result, Err(Error::PlaylistParse(_))),
            "{bad}: {result:?}"
        );
        assert!(
            key_urls(&drain(&mut client)).is_empty(),
            "no key request may be issued from a playlist that failed: {bad}"
        );
    }
}
