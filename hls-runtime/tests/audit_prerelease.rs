use hls_runtime::client::{Action, Error, HlsClient};

const URL: &str = "http://origin/live/media.m3u8";

fn drain(client: &mut HlsClient) -> Vec<Action> {
    std::iter::from_fn(|| client.poll()).collect()
}

fn fetched(actions: &[Action]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::FetchResource { url, .. } => Some(url.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn clear_segments_after_a_fairplay_key_and_method_none_are_fetched() {
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    let text = "#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:0\n\
#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://k\",KEYFORMAT=\"com.apple.streamingkeydelivery\",KEYFORMATVERSIONS=\"1\"\n\
#EXTINF:4,\na.ts\n#EXT-X-KEY:METHOD=NONE\n#EXTINF:4,\nb.ts\n#EXT-X-ENDLIST\n";
    let result = client.on_playlist(text.as_bytes());
    assert!(matches!(result, Err(Error::PlaylistParse(_))), "{result:?}");
    assert_eq!(fetched(&drain(&mut client)), ["http://origin/live/b.ts"]);
}
