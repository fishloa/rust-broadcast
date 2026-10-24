use hls_runtime::client::{Action, HlsClient};

const ORIGIN_URL: &str = "http://origin.test/hls/index.m3u8";
const FINAL_URL: &str = "http://cdn.test/live/index.m3u8";
const PLAYLIST: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
    #EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n#EXT-X-MAP:URI=\"init.mp4\"\n\
    #EXTINF:2.0,\nseg0.m4s\n";

fn fetch_urls(client: &mut HlsClient) -> Vec<String> {
    std::iter::from_fn(|| client.poll())
        .filter_map(|a| match a {
            Action::FetchResource { url, .. } | Action::FetchKey { url, .. } => Some(url),
            _ => None,
        })
        .collect()
}

#[test]
fn relative_uris_resolve_against_the_playlist_base_not_the_request_url() {
    let mut control = HlsClient::new(ORIGIN_URL);
    control.poll();
    let _ = control.on_playlist(PLAYLIST.as_bytes());
    let before = fetch_urls(&mut control);
    assert!(
        before
            .iter()
            .all(|u| u.starts_with("http://origin.test/hls/")),
        "{before:?}"
    );

    let mut client = HlsClient::new(ORIGIN_URL);
    client.poll();
    client.set_playlist_base(FINAL_URL);
    let _ = client.on_playlist(PLAYLIST.as_bytes());
    let urls = fetch_urls(&mut client);
    assert!(!urls.is_empty());
    assert!(
        urls.iter().all(|u| u.starts_with("http://cdn.test/live/")),
        "{urls:?}"
    );
    assert!(
        urls.contains(&"http://cdn.test/live/seg0.m4s".to_string()),
        "{urls:?}"
    );
    assert_eq!(
        client.playlist_url(),
        ORIGIN_URL,
        "reloads keep the request URL"
    );
}
